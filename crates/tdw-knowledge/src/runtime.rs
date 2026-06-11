//! The knowledge runtime the MCP server hosts (knowledge-system B8).
//!
//! [`KnowledgeRuntime`] bundles the hybrid [`Retriever`] with direct handles
//! to the graph and tag engines (for entity/traverse/path/tag tools that
//! bypass retrieval) plus the version triple every search response reports,
//! so agents can attribute result drift to the exact rule-set/infer/embedder
//! versions that produced it. B9 adds the gated write side (proposals) to
//! this same seam.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEngine, LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_retrieve::Retriever;
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

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
    /// Version triple reported on every search response.
    ///
    /// Wrapped in [`RwLock`] so hot-reload (K-L1) can update the
    /// rule/infer versions atomically after the engines reload —
    /// multiple concurrent readers (`versions()`) see a consistent
    /// snapshot while one writer (`update_versions`) holds the lock briefly.
    versions: RwLock<KnowledgeVersions>,
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
            versions: RwLock::new(versions),
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

    /// Stamp the rule/infer versions at construction time.
    ///
    /// For live updates after hot-reload, use [`update_versions`](Self::update_versions).
    #[must_use]
    pub fn with_versions(mut self, rules_version: Option<u64>, infer_version: Option<u64>) -> Self {
        let versions = self
            .versions
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        versions.rules_version = rules_version;
        versions.infer_version = infer_version;
        self
    }

    /// Update the rule/infer versions atomically after a hot-reload (K-L1).
    ///
    /// Acquires the write lock briefly; all concurrent `versions()` readers see a
    /// consistent snapshot. This is the live-daemon counterpart to `with_versions`
    /// (which is builder-only and cannot be called after `Arc` wrapping).
    pub fn update_versions(&self, rules_version: Option<u64>, infer_version: Option<u64>) {
        let mut guard = self
            .versions
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.rules_version = rules_version;
        guard.infer_version = infer_version;
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

    /// A snapshot of the version triple stamped onto search responses.
    ///
    /// Acquires the read lock briefly. Callers that need a stable snapshot across
    /// multiple fields should clone the returned value.
    #[must_use]
    pub fn versions(&self) -> KnowledgeVersions {
        self.versions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
