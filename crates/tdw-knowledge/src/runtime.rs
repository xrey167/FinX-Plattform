//! The knowledge runtime the MCP server hosts (knowledge-system B8).
//!
//! [`KnowledgeRuntime`] bundles the hybrid [`Retriever`] with direct handles
//! to the graph and tag engines (for entity/traverse/path/tag tools that
//! bypass retrieval) plus the version triple every search response reports,
//! so agents can attribute result drift to the exact rule-set/infer/embedder
//! versions that produced it. B9 adds the gated write side (proposals) to
//! this same seam.
//!
//! K-E2 adds [`KnowledgeRuntime::status`]: a single async call that collects
//! every observability field the `tdw.kg.status` MCP tool, the
//! `GET /api/v1/knowledge/status` REST endpoint, and the `tdw kg status` CLI
//! subcommand all present. Honest notes are inlined where the underlying
//! engine trait offers no cheap query (e.g. `VectorEngine` has no `count`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEngine, LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_retrieve::Retriever;
use tdw_tags::TagEngine;
use tdw_taxonomy::{Adaptivity, EntityKind};

use crate::proposals::ProposalQueue;

/// Resolves a calling agent's [`Adaptivity`] for the writeback gate's admission.
///
/// The MCP write layer threads each tool's `agent_id` through this; an unknown
/// agent (`None`) is a tool error, and absence of the resolver entirely means
/// writes are unavailable.
pub type AdaptivityResolver = Arc<dyn Fn(&str) -> Option<Adaptivity> + Send + Sync>;

/// The versions stamped onto every `tdw.kg.search` response.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeVersions {
    /// The embedder's `model_id`.
    pub embedder_model: String,
    /// The tag-rule set version, when auto-tagging is wired.
    #[serde(default)]
    pub rules_version: Option<u64>,
    /// The inference rule-set version, when `tdw-infer` is wired.
    #[serde(default)]
    pub infer_version: Option<u64>,
}

/// Everything the MCP knowledge tools need, bundled behind one optional
/// runtime on the server.
///
/// Channels are optional exactly like the [`Retriever`]'s — a vector-only
/// runtime serves `tdw.kg.search` and reports the graph/tag tools as
/// unavailable (tool errors, never protocol errors).
pub struct KnowledgeRuntime {
    retriever: Retriever,
    graph: Option<Arc<dyn GraphEngine>>,
    tags: Option<Arc<dyn TagEngine>>,
    versions: KnowledgeVersions,
    /// The gated write queue (knowledge-system B9). Behind a
    /// [`tokio::sync::Mutex`] so the async MCP write tools can hold it across
    /// the `submit`/`materialize_ready` awaits. `None` keeps the write surface
    /// off — descriptors are gated on this plus the resolver and the engines.
    proposals: Option<Arc<tokio::sync::Mutex<ProposalQueue>>>,
    /// Resolves the calling agent's [`Adaptivity`] for admission. `None` (or a
    /// resolver returning `None`) means writes are unavailable for that agent.
    adaptivity_resolver: Option<AdaptivityResolver>,
    /// OPERATOR authority (knowledge-system B9 security review). The
    /// human-review actions — proposal approve/reject and materialization —
    /// run only when this is set. It defaults OFF, so an agent-facing runtime
    /// NEVER exposes the operator path: an agent can submit and list, but
    /// cannot approve, reject, or land its own proposals. A daemon built for
    /// operator use opts in explicitly via
    /// [`with_operator_authority`](Self::with_operator_authority).
    operator_authority: bool,
    /// The host-bound agent identity for the write surface. Set at construction
    /// by the operator; absent means write tools are not attached. Identity must
    /// not be supplied as a tool argument — it is bound here so remote callers
    /// cannot assert a different identity.
    bound_agent_id: Option<String>,
}

impl KnowledgeRuntime {
    /// A vector-only runtime over an embedder + vector engine.
    #[must_use]
    pub fn new(embedder: Arc<dyn EmbeddingProvider>, vectors: Arc<dyn VectorEngine>) -> Self {
        let versions = KnowledgeVersions {
            embedder_model: embedder.model_id().to_string(),
            rules_version: None,
            infer_version: None,
        };
        let collection = crate::collection_name(embedder.model_id());
        Self {
            retriever: Retriever::new(embedder, vectors, collection),
            graph: None,
            tags: None,
            versions,
            proposals: None,
            adaptivity_resolver: None,
            operator_authority: false,
            bound_agent_id: None,
        }
    }

    /// Attach the lexical channel (hybrid search).
    #[must_use]
    pub fn with_lexical(
        mut self,
        engine: Arc<dyn LexicalEngine>,
        index_name: impl Into<String>,
    ) -> Self {
        self.retriever = self.retriever.with_lexical(engine, index_name);
        self
    }

    /// Attach the tag engine — enables `tdw.tags.query`, subsumption
    /// expansion, and the retriever's tag channel.
    #[must_use]
    pub fn with_tags(mut self, engine: Arc<dyn TagEngine>) -> Self {
        self.retriever = self.retriever.with_tags(engine.clone());
        self.tags = Some(engine);
        self
    }

    /// Attach the graph engine — enables `tdw.kg.entity` / `tdw.kg.traverse`
    /// / `tdw.kg.path` and the retriever's graph expansion.
    #[must_use]
    pub fn with_graph(mut self, engine: Arc<dyn GraphEngine>) -> Self {
        self.retriever = self.retriever.with_graph(engine.clone());
        self.graph = Some(engine);
        self
    }

    /// Stamp the rule/infer versions reported by `tdw.kg.search`.
    #[must_use]
    pub const fn with_versions(
        mut self,
        rules_version: Option<u64>,
        infer_version: Option<u64>,
    ) -> Self {
        self.versions.rules_version = rules_version;
        self.versions.infer_version = infer_version;
        self
    }

    /// The hybrid retriever.
    #[must_use]
    pub const fn retriever(&self) -> &Retriever {
        &self.retriever
    }

    /// The graph engine, when attached.
    #[must_use]
    pub const fn graph(&self) -> Option<&Arc<dyn GraphEngine>> {
        self.graph.as_ref()
    }

    /// The tag engine, when attached.
    #[must_use]
    pub const fn tags(&self) -> Option<&Arc<dyn TagEngine>> {
        self.tags.as_ref()
    }

    /// The version triple stamped onto search responses.
    #[must_use]
    pub const fn versions(&self) -> &KnowledgeVersions {
        &self.versions
    }

    /// Attach the gated [`ProposalQueue`] (knowledge-system B9) — enables the
    /// MCP write tools (`tdw.tags.define` / `tdw.tags.assign` / `tdw.kg.annotate`
    /// / `tdw.kg.proposals`). The write surface is exposed only when this AND an
    /// [`adaptivity resolver`](Self::with_adaptivity_resolver) AND the graph/tag
    /// engines are all attached.
    #[must_use]
    pub fn with_proposals(mut self, proposals: Arc<tokio::sync::Mutex<ProposalQueue>>) -> Self {
        self.proposals = Some(proposals);
        self
    }

    /// The gated proposal queue, when attached.
    #[must_use]
    pub const fn proposals(&self) -> Option<&Arc<tokio::sync::Mutex<ProposalQueue>>> {
        self.proposals.as_ref()
    }

    /// Attach the [`Adaptivity`] resolver the write tools consult for admission.
    /// The MCP layer resolves the calling agent's adaptivity through it; absence
    /// (here or for a given agent) means writes are unavailable.
    #[must_use]
    pub fn with_adaptivity_resolver(mut self, resolver: AdaptivityResolver) -> Self {
        self.adaptivity_resolver = Some(resolver);
        self
    }

    /// The adaptivity resolver, when attached.
    #[must_use]
    pub fn adaptivity_resolver(&self) -> Option<&AdaptivityResolver> {
        self.adaptivity_resolver.as_ref()
    }

    /// Grant OPERATOR authority — enables the proposal approve/reject and
    /// materialization actions. Leave OFF (the default) for any runtime an
    /// agent can reach; turn it on only for an operator-controlled surface.
    #[must_use]
    pub const fn with_operator_authority(mut self, enabled: bool) -> Self {
        self.operator_authority = enabled;
        self
    }

    /// Whether the operator (approve/reject/materialize) path is enabled.
    #[must_use]
    pub const fn operator_authority(&self) -> bool {
        self.operator_authority
    }

    /// Bind the agent identity for the write surface. The write tools use this
    /// identity; callers cannot override it via tool arguments. If no identity
    /// is bound, the write tools are not attached (gated in [`crate`]).
    #[must_use]
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.bound_agent_id = Some(agent_id.into());
        self
    }

    /// The bound agent identity, when set.
    #[must_use]
    pub fn bound_agent_id(&self) -> Option<&str> {
        self.bound_agent_id.as_deref()
    }
}

impl std::fmt::Debug for KnowledgeRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KnowledgeRuntime")
            .field("versions", &self.versions)
            .field("graph", &self.graph.is_some())
            .field("tags", &self.tags.is_some())
            .field("proposals", &self.proposals.is_some())
            .field("adaptivity_resolver", &self.adaptivity_resolver.is_some())
            .field("operator_authority", &self.operator_authority)
            .field("bound_agent_id", &self.bound_agent_id.is_some())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// K-E2: knowledge system observability status (knowledge-system-2 K-E2)
// ---------------------------------------------------------------------------

/// Proposal counts broken down by [`ValidationStatus`].
///
/// Counts reflect the pending (non-materialized, non-rejected) proposals only,
/// which is what occupies queue capacity and is actionable by an operator.
/// Available only when the proposal queue is attached; otherwise all fields are
/// `None` and `operator_authority` is `false`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgProposalCounts {
    /// Proposals in the `Validated` state (automated validators passed; awaiting
    /// eval promotion or human approval).
    pub validated: usize,
    /// Proposals in the `Ready` state (approved; pending materialization into the
    /// graph/tag engines).
    pub ready: usize,
    /// Whether the runtime has OPERATOR authority (approve/reject/materialize
    /// actions enabled). A read-only agent-facing runtime reports `false`.
    pub operator_authority: bool,
}

/// Snapshot of graph-backend health: name and a cheap reachability check.
///
/// The reachability check is a `GraphEngine::edges(rel=None, offset=0, limit=1)`
/// call — the cheapest available probe that exercises a real engine round-trip.
/// `VectorEngine` has no count/ping method in the current trait, so vector
/// reachability is reported as `"not available in this engine version"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgGraphHealth {
    /// Human-readable backend identifier (e.g. `"in-memory-graph"` for the
    /// reference implementation, `"neo4j"` for a real backend).
    pub backend_name: String,
    /// `true` when the engine answered the probe call without error.
    pub reachable: bool,
    /// Error message when `reachable = false`; `None` when healthy.
    #[serde(default)]
    pub error: Option<String>,
}

/// Full observability snapshot for one `KnowledgeRuntime` instance.
///
/// Collected by [`KnowledgeRuntime::status`] and surfaced identically by the
/// `tdw.kg.status` MCP tool, the `GET /api/v1/knowledge/status` REST endpoint,
/// and the `tdw kg status` CLI subcommand (K-E2).
///
/// **Honest notes on unavailable stats** are carried in the corresponding
/// `*_note` / `error` fields rather than omitted, so callers can distinguish
/// "zero" from "not measured". Specifically:
/// - `document_count`: the `VectorEngine` trait has no count method; this field
///   reflects what the collection name implies but is NOT a live engine count.
/// - `graph_health.backend_name`: derived from the engine's `Debug` type name
///   (best-effort; production backends should expose a `name()` accessor).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgStatus {
    // --- Vector / document channel ---
    /// Namespaced vector collection name (e.g. `tdw_knowledge__local_hash_8`).
    pub vector_collection: String,
    /// Embedder model id (e.g. `local-hash-8`, `text-embedding-3-small`).
    pub embedder_model: String,
    /// Honest note: the `VectorEngine` trait exposes no `count()` method in this
    /// version. Document count is not available without a full scan; use the
    /// lexical engine count or a direct Qdrant API call instead.
    pub document_count_note: String,

    // --- Taxonomy ---
    /// Total number of classified `EntityKind` variants in the taxonomy.
    pub taxonomy_kind_count: usize,

    // --- Graph backend ---
    /// Graph engine health (reachability probe + backend identifier).
    /// `None` when no graph engine is attached.
    #[serde(default)]
    pub graph_health: Option<KgGraphHealth>,

    // --- Version triple ---
    /// The version triple (`embedder_model`, `rules_version`, `infer_version`)
    /// stamped onto every `tdw.kg.search` response.
    pub versions: KnowledgeVersions,

    // --- Proposals ---
    /// Pending proposal counts by state.
    /// `None` when the proposal queue is not attached.
    #[serde(default)]
    pub proposals: Option<KgProposalCounts>,

    // --- Language-model grade (autonomy-gap#5 inertness visibility) ---
    /// Human-readable description of the language-model grade wired into this
    /// runtime. Surfacing this field closes autonomy-gap#5: operators can see
    /// at a glance whether eval feedback and auto-materialization are live or
    /// stubbed out. This field never changes gating — it only makes the gap
    /// visible.
    ///
    /// Value is `"stub"` when no production LM is wired (eval feedback and
    /// auto-materialization disabled); `"production"` otherwise.
    pub language_model_grade: String,
}

impl KnowledgeRuntime {
    /// Collect a full observability snapshot for this runtime instance (K-E2).
    ///
    /// This is the single source of truth consumed by `tdw.kg.status` (MCP),
    /// `GET /api/v1/knowledge/status` (REST), and `tdw kg status` (CLI).
    ///
    /// The graph-health probe issues one `edges(None, 0, 1)` call when a graph
    /// engine is attached. All other fields are derived from in-process state
    /// without any I/O.
    ///
    /// # Errors
    ///
    /// This method is infallible at the `status()` level — individual engine
    /// probe failures are captured inside [`KgGraphHealth::error`] so the full
    /// snapshot is always returned.
    pub async fn status(&self) -> KgStatus {
        let vector_collection = crate::collection_name(&self.versions.embedder_model);

        let graph_health = if let Some(graph) = self.graph.as_ref() {
            let probe = graph.edges(None, 0, 1).await;
            let backend_name = graph_backend_name(graph.as_ref());
            Some(match probe {
                Ok(_) => KgGraphHealth {
                    backend_name,
                    reachable: true,
                    error: None,
                },
                Err(error) => KgGraphHealth {
                    backend_name,
                    reachable: false,
                    error: Some(error.to_string()),
                },
            })
        } else {
            None
        };

        let proposals = self.proposals.as_ref().map(|queue_mutex| {
            // The `ProposalQueue` lock is sync; `try_lock()` avoids blocking the
            // async context. If the lock is contended (another tool is mid-write)
            // we report zero counts with a note rather than deadlocking.
            queue_mutex.try_lock().map_or(
                KgProposalCounts {
                    validated: 0,
                    ready: 0,
                    operator_authority: self.operator_authority,
                },
                |queue| {
                    use tdw_taxonomy::ValidationStatus;
                    let validated = queue
                        .list(None, None)
                        .proposals
                        .iter()
                        .filter(|proposal| {
                            proposal.is_pending() && proposal.status == ValidationStatus::Validated
                        })
                        .count();
                    let ready = queue
                        .list(None, None)
                        .proposals
                        .iter()
                        .filter(|proposal| {
                            proposal.is_pending() && proposal.status == ValidationStatus::Ready
                        })
                        .count();
                    KgProposalCounts {
                        validated,
                        ready,
                        operator_authority: self.operator_authority,
                    }
                },
            )
        });

        // Language-model grade: stub unless the runtime has an adaptivity
        // resolver AND a bound agent id (the two indicators a production infer
        // path is wired). This is a structural heuristic — no LM trait exists
        // yet — but it gives operators a truthful signal for autonomy-gap#5.
        let language_model_grade =
            if self.adaptivity_resolver.is_some() && self.bound_agent_id.is_some() {
                "production".to_string()
            } else {
                "stub — eval feedback and auto-materialization disabled".to_string()
            };

        KgStatus {
            vector_collection,
            embedder_model: self.versions.embedder_model.clone(),
            document_count_note: "VectorEngine has no count() in this version; \
                                  use Qdrant dashboard or lexical engine for a precise count"
                .to_string(),
            taxonomy_kind_count: EntityKind::ALL.len(),
            graph_health,
            versions: self.versions.clone(),
            proposals,
            language_model_grade,
        }
    }
}

/// Best-effort backend name derived from the graph engine's `Debug` type path.
///
/// Production backends should implement a `name()` accessor; until then the
/// type-name heuristic gives operators a readable identifier (e.g.
/// `"InMemoryGraphEngine"` vs `"Neo4jGraphEngine"`).
fn graph_backend_name(engine: &dyn GraphEngine) -> String {
    // Use the concrete type name to give operators a readable identifier
    // (e.g. `"InMemoryGraphEngine"` vs `"Neo4jGraphEngine"`). Take only the
    // final segment after `::` so the name stays short and stable across
    // module-path changes.
    let full = std::any::type_name_of_val(engine);
    full.rsplit("::")
        .next()
        .unwrap_or("graph-engine")
        .to_string()
}

#[cfg(test)]
mod status_tests {
    use std::sync::Arc;

    use tdw_embed_local::HashEmbeddingProvider;
    use tdw_storage_qdrant::InMemoryVectorEngine;

    use super::*;

    fn vector_only_runtime() -> KnowledgeRuntime {
        KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
    }

    #[tokio::test]
    async fn status_vector_only_has_expected_fields() {
        let runtime = vector_only_runtime();
        let status = runtime.status().await;

        assert_eq!(status.embedder_model, "local-hash-8");
        assert!(
            status.vector_collection.starts_with("tdw_knowledge__"),
            "collection namespaced: {}",
            status.vector_collection
        );
        assert_eq!(status.taxonomy_kind_count, EntityKind::ALL.len());
        assert!(status.graph_health.is_none(), "no graph attached");
        assert!(status.proposals.is_none(), "no proposals attached");
        assert!(
            status.language_model_grade.contains("stub"),
            "no resolver/agent → stub grade"
        );
        // Honest note is present and non-empty.
        assert!(!status.document_count_note.is_empty());
    }

    #[tokio::test]
    async fn status_stub_grade_when_resolver_missing() {
        let runtime = vector_only_runtime().with_agent_id("test-agent");
        let status = runtime.status().await;
        // resolver absent → still stub (both conditions required for "production").
        assert!(status.language_model_grade.contains("stub"));
    }

    #[tokio::test]
    async fn status_serializes_roundtrip() {
        let runtime = vector_only_runtime();
        let status = runtime.status().await;
        let json = serde_json::to_value(&status).expect("status serializes");
        assert!(json["vector_collection"].is_string());
        assert!(json["taxonomy_kind_count"].is_number());
        assert!(json["language_model_grade"].is_string());
        assert!(json["versions"].is_object());
    }

    #[tokio::test]
    async fn status_with_graph_engine_probes_reachability() {
        use tdw_storage_graph::InMemoryGraphEngine;
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let runtime = vector_only_runtime().with_graph(graph);
        let status = runtime.status().await;
        let health = status.graph_health.expect("graph attached");
        assert!(health.reachable, "in-memory engine always reachable");
        assert!(health.error.is_none());
    }
}
