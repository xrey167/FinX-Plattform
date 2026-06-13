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
//! K-X6 adds the user-identity and finding-indexer seams for the first-class
//! research-findings surface.

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEngine, LexicalEngine, VectorEngine};
use tdw_embed::EmbeddingProvider;
use tdw_retrieve::Retriever;
use tdw_tags::TagEngine;
use tdw_taxonomy::{Adaptivity, EntityKind};

use crate::feeds::FeedFreshness;
use crate::indexer::KnowledgeIndexer;
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
    /// Operator-supplied graph backend name (e.g. `"in-memory"`, `"neo4j"`).
    /// Set via [`with_graph_name`](Self::with_graph_name). When absent, status
    /// falls back to `"graph-engine"` rather than using an unstable type-name
    /// heuristic.
    graph_name: Option<String>,
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
    /// The host-bound user identity for the finding surface (knowledge-system
    /// K-X6). Absent means the finding tools are not attached. Identity must
    /// not be supplied as a tool argument — it is bound here so remote callers
    /// cannot assert a different identity.
    bound_user_id: Option<String>,
    /// The finding indexer for hybrid search indexing of user-authored findings
    /// (knowledge-system K-X6). Behind a `std::sync::Mutex` so the sync MCP
    /// dispatch can hold the lock across the `index_at` await via `block_on`.
    /// `None` means findings are written to the graph but NOT indexed for
    /// retrieval — valid for write-only surfaces or graph-only deployments.
    finding_indexer: Option<Arc<std::sync::Mutex<KnowledgeIndexer>>>,
    /// Live eval-freshness cell (K-L3). Shared with the scheduled eval worker,
    /// which writes [`EvalFreshness`] after each run. `None` when no eval is
    /// configured (status reports [`EvalFreshness::Unconfigured`]).
    eval_freshness: Option<Arc<tokio::sync::Mutex<EvalFreshness>>>,
    /// Live consolidation-freshness cell (K-L2). Shared with the consolidation
    /// scheduler, which writes [`ConsolidationFreshness`] after each tick.
    /// `None` before the first write or when the scheduler is not running
    /// (status reports [`ConsolidationFreshness::Pending`]).
    consolidation_freshness: Option<Arc<tokio::sync::Mutex<ConsolidationFreshness>>>,
    /// Live sweep-freshness cell (K-L4). Shared with the auto-materialization
    /// sweep worker in `tdw-backend`. `None` when the sweep is not registered
    /// (status reports [`SweepFreshness::Disabled`]).
    auto_materialize_freshness: Option<Arc<tokio::sync::Mutex<SweepFreshness>>>,
    /// Live per-feed freshness cells (K-L6). One cell per enabled feed,
    /// shared with the spawned feed tasks that write [`FeedFreshness`] after
    /// each poll. Empty when no feeds are configured.
    feed_freshness_cells: Vec<Arc<tokio::sync::Mutex<FeedFreshness>>>,
    /// Live watchlist-freshness cell (K-X5). Shared with the spawned watchlist
    /// cron task, which writes [`WatchlistFreshness`] after each tick.
    /// `None` when the watchlist scheduler is not running (status field omitted
    /// from JSON so agents see the absence explicitly).
    watchlist_freshness: Option<Arc<std::sync::Mutex<WatchlistFreshness>>>,
    /// Live questions-freshness cell (K-X8). Shared with the spawned questions
    /// cron task, which writes [`QuestionsFreshness`] after each tick.
    /// `None` when the question scheduler is not running (status field omitted
    /// from JSON so agents see the absence explicitly).
    questions_freshness: Option<Arc<std::sync::Mutex<QuestionsFreshness>>>,
    /// Live extraction-freshness cell (K-M1). Shared with the `KnowledgeIndexer`
    /// (via the composition root) which writes [`ExtractionFreshness`] after
    /// each extraction batch. `None` when the extraction stage is not wired
    /// (`[knowledge.extraction] enabled = false` or no production-grade model).
    extraction_freshness: Option<Arc<std::sync::Mutex<crate::ExtractionFreshness>>>,
    /// Live skill-counts cell (K-R2). Shared with the skill-lifecycle/tournament
    /// worker, which writes [`crate::skills::SkillCounts`] after each tournament
    /// tick. `None` when skill lifecycle is not wired (status field omitted from
    /// JSON so the absence is explicit — loud by design).
    skill_counts: Option<Arc<std::sync::Mutex<crate::skills::SkillCounts>>>,
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
            graph_name: None,
            tags: None,
            versions: RwLock::new(versions),
            proposals: None,
            adaptivity_resolver: None,
            operator_authority: false,
            bound_agent_id: None,
            bound_user_id: None,
            finding_indexer: None,
            eval_freshness: None,
            consolidation_freshness: None,
            auto_materialize_freshness: None,
            feed_freshness_cells: Vec::new(),
            watchlist_freshness: None,
            questions_freshness: None,
            extraction_freshness: None,
            skill_counts: None,
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

    /// Record the graph backend name reported by `tdw.kg.status`.
    ///
    /// The daemon knows which backend it constructed (e.g. `"in-memory"`,
    /// `"neo4j"`) and should set this at construction time so the status
    /// snapshot carries an accurate, stable name. When absent the snapshot
    /// falls back to `"graph-engine"`.
    #[must_use]
    pub fn with_graph_name(mut self, name: impl Into<String>) -> Self {
        self.graph_name = Some(name.into());
        self
    }

    /// Stamp the rule/infer versions reported by `tdw.kg.search`.
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

    /// Bind the user identity for the finding surface (knowledge-system K-X6).
    /// The finding tools use this identity; callers cannot override it via tool
    /// arguments. If no user identity is bound, the finding tools are not
    /// attached (gated in [`crate`]).
    #[must_use]
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.bound_user_id = Some(user_id.into());
        self
    }

    /// The bound user identity, when set.
    #[must_use]
    pub fn bound_user_id(&self) -> Option<&str> {
        self.bound_user_id.as_deref()
    }

    /// Attach a [`KnowledgeIndexer`] for hybrid search indexing of
    /// user-authored findings (knowledge-system K-X6). When absent, findings
    /// are written to the graph but are NOT retrievable via `tdw.kg.search`
    /// until a full re-index is run.
    #[must_use]
    pub fn with_finding_indexer(
        mut self,
        indexer: Arc<std::sync::Mutex<KnowledgeIndexer>>,
    ) -> Self {
        self.finding_indexer = Some(indexer);
        self
    }

    /// The finding indexer, when attached.
    #[must_use]
    pub const fn finding_indexer(&self) -> Option<&Arc<std::sync::Mutex<KnowledgeIndexer>>> {
        self.finding_indexer.as_ref()
    }

    /// Attach the eval-freshness cell (K-L3).
    ///
    /// The scheduled eval worker holds a clone of this `Arc` and writes the
    /// updated [`EvalFreshness`] after each run.  The `status()` method reads
    /// the cell to populate the `eval_freshness` field in [`KgStatus`].
    ///
    /// Pass a pre-constructed `Arc<tokio::sync::Mutex<EvalFreshness>>` so the
    /// daemon can share ownership with the worker task without additional
    /// wrapping.
    #[must_use]
    pub fn with_eval_freshness(mut self, cell: Arc<tokio::sync::Mutex<EvalFreshness>>) -> Self {
        self.eval_freshness = Some(cell);
        self
    }

    /// The eval-freshness cell, when attached.
    ///
    /// The scheduled eval worker uses this to write updated freshness after
    /// each run.
    #[must_use]
    pub const fn eval_freshness_cell(&self) -> Option<&Arc<tokio::sync::Mutex<EvalFreshness>>> {
        self.eval_freshness.as_ref()
    }

    /// Attach the consolidation-freshness cell (K-L2).
    ///
    /// The consolidation scheduler (`spawn_consolidation_scheduler_with_feedback`
    /// in `tdw-agent-store`) holds a clone of this `Arc` and writes the updated
    /// [`ConsolidationFreshness`] after each tick. The `status()` method reads the
    /// cell to populate the `consolidation_freshness` field in [`KgStatus`].
    #[must_use]
    pub fn with_consolidation_freshness(
        mut self,
        cell: Arc<tokio::sync::Mutex<ConsolidationFreshness>>,
    ) -> Self {
        self.consolidation_freshness = Some(cell);
        self
    }

    /// The consolidation-freshness cell, when attached.
    ///
    /// The consolidation scheduler uses this to write updated freshness after
    /// each tick.
    #[must_use]
    pub const fn consolidation_freshness_cell(
        &self,
    ) -> Option<&Arc<tokio::sync::Mutex<ConsolidationFreshness>>> {
        self.consolidation_freshness.as_ref()
    }

    /// Attach the auto-materialization sweep freshness cell (K-L4).
    ///
    /// Pass an `Arc<tokio::sync::Mutex<SweepFreshness>>` constructed by the
    /// composition root (`tdw-backend`) so the sweep worker and `tdw.kg.status`
    /// share the same live cell. Consumes and returns `self` for builder use.
    #[must_use]
    pub fn with_auto_materialize_freshness(
        mut self,
        cell: Arc<tokio::sync::Mutex<SweepFreshness>>,
    ) -> Self {
        self.auto_materialize_freshness = Some(cell);
        self
    }

    /// The sweep-freshness cell, when attached.
    ///
    /// The sweep worker in `tdw-backend` writes [`SweepFreshness`] after every
    /// sweep invocation via this handle.
    #[must_use]
    pub const fn auto_materialize_freshness_cell(
        &self,
    ) -> Option<&Arc<tokio::sync::Mutex<SweepFreshness>>> {
        self.auto_materialize_freshness.as_ref()
    }

    /// Attach a per-feed freshness cell (K-L6).
    ///
    /// Call once per enabled feed after spawning the feed task. The cell is
    /// shared with the task; `status()` reads all attached cells to populate
    /// `KgStatus::feed_statuses`.
    #[must_use]
    pub fn with_feed_freshness(mut self, cell: Arc<tokio::sync::Mutex<FeedFreshness>>) -> Self {
        self.feed_freshness_cells.push(cell);
        self
    }

    /// All per-feed freshness cells, in registration order.
    #[must_use]
    pub fn feed_freshness_cells(&self) -> &[Arc<tokio::sync::Mutex<FeedFreshness>>] {
        &self.feed_freshness_cells
    }

    /// Attach the watchlist freshness cell (K-X5).
    ///
    /// Pass the `Arc<std::sync::Mutex<WatchlistFreshness>>` constructed by the
    /// composition root (`tdw-backend`) so the watchlist cron task and
    /// `tdw.kg.status` share the same live cell. Consumes and returns `self`
    /// for builder use.
    #[must_use]
    pub fn with_watchlist_freshness(
        mut self,
        cell: Arc<std::sync::Mutex<WatchlistFreshness>>,
    ) -> Self {
        self.watchlist_freshness = Some(cell);
        self
    }

    /// The watchlist-freshness cell, when attached.
    ///
    /// The watchlist cron task in `tdw-backend` writes [`WatchlistFreshness`]
    /// after every tick via this handle. `None` when the scheduler is not wired
    /// (status field is omitted from JSON).
    #[must_use]
    pub const fn watchlist_freshness_cell(
        &self,
    ) -> Option<&Arc<std::sync::Mutex<WatchlistFreshness>>> {
        self.watchlist_freshness.as_ref()
    }

    /// Attach the questions freshness cell (K-X8).
    ///
    /// Pass the `Arc<std::sync::Mutex<QuestionsFreshness>>` constructed by the
    /// composition root (`tdw-backend`) so the questions cron task and
    /// `tdw.kg.status` share the same live cell. Consumes and returns `self`
    /// for builder use.
    #[must_use]
    pub fn with_questions_freshness(
        mut self,
        cell: Arc<std::sync::Mutex<QuestionsFreshness>>,
    ) -> Self {
        self.questions_freshness = Some(cell);
        self
    }

    /// The questions-freshness cell, when attached.
    ///
    /// The questions cron task in `tdw-backend` writes [`QuestionsFreshness`]
    /// after every tick via this handle. `None` when the scheduler is not wired
    /// (status field is omitted from JSON).
    #[must_use]
    pub const fn questions_freshness_cell(
        &self,
    ) -> Option<&Arc<std::sync::Mutex<QuestionsFreshness>>> {
        self.questions_freshness.as_ref()
    }

    /// Attach the LLM extraction-freshness cell (K-M1).
    ///
    /// Pass an `Arc<std::sync::Mutex<ExtractionFreshness>>` constructed by the
    /// composition root (`tdw-backend`) so the `KnowledgeIndexer`'s extraction
    /// stage and `tdw.kg.status` share the same live cell.
    ///
    /// When absent the `extraction_freshness` field is omitted from the status
    /// JSON — explicit absence (not silent zero) so operators see the gap.
    #[must_use]
    pub fn with_extraction_freshness(
        mut self,
        cell: Arc<std::sync::Mutex<crate::ExtractionFreshness>>,
    ) -> Self {
        self.extraction_freshness = Some(cell);
        self
    }

    /// The extraction-freshness cell, when attached.
    ///
    /// The `KnowledgeIndexer` writes [`ExtractionFreshness`] after each
    /// extraction batch via this handle.
    #[must_use]
    pub const fn extraction_freshness_cell(
        &self,
    ) -> Option<&Arc<std::sync::Mutex<crate::ExtractionFreshness>>> {
        self.extraction_freshness.as_ref()
    }

    /// Attach the skill-counts cell (K-R2).
    ///
    /// Pass the `Arc<std::sync::Mutex<SkillCounts>>` constructed by the
    /// composition root (`tdw-backend`) so the skill-lifecycle/tournament worker
    /// and `tdw.kg.status` share the same live cell. Consumes and returns `self`
    /// for builder use. When absent, `KgStatus::skills` is `None` (omitted from
    /// JSON) — skill lifecycle is not wired.
    #[must_use]
    pub fn with_skill_counts(
        mut self,
        cell: Arc<std::sync::Mutex<crate::skills::SkillCounts>>,
    ) -> Self {
        self.skill_counts = Some(cell);
        self
    }

    /// The skill-counts cell, when attached.
    ///
    /// The skill-lifecycle/tournament worker in `tdw-backend` writes
    /// [`crate::skills::SkillCounts`] after every tournament tick via this handle.
    #[must_use]
    pub const fn skill_counts_cell(
        &self,
    ) -> Option<&Arc<std::sync::Mutex<crate::skills::SkillCounts>>> {
        self.skill_counts.as_ref()
    }
}

impl std::fmt::Debug for KnowledgeRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KnowledgeRuntime")
            .field("versions", &self.versions)
            .field("graph", &self.graph.is_some())
            .field("graph_name", &self.graph_name)
            .field("tags", &self.tags.is_some())
            .field("proposals", &self.proposals.is_some())
            .field("adaptivity_resolver", &self.adaptivity_resolver.is_some())
            .field("operator_authority", &self.operator_authority)
            .field("bound_agent_id", &self.bound_agent_id.is_some())
            .field("bound_user_id", &self.bound_user_id.is_some())
            .field("finding_indexer", &self.finding_indexer.is_some())
            .field("eval_freshness", &self.eval_freshness.is_some())
            .field(
                "consolidation_freshness",
                &self.consolidation_freshness.is_some(),
            )
            .field(
                "auto_materialize_freshness",
                &self.auto_materialize_freshness.is_some(),
            )
            .field("feed_freshness_cells", &self.feed_freshness_cells.len())
            .field("extraction_freshness", &self.extraction_freshness.is_some())
            .field("skill_counts", &self.skill_counts.is_some())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// K-L2: consolidation-freshness status (knowledge-system-2 K-L2)
// ---------------------------------------------------------------------------

/// Consolidation-freshness status for `tdw.kg.status` (K-L2).
///
/// Surfaced as `consolidation_freshness` on [`KgStatus`] so operators see at a
/// glance whether the last scheduled consolidation tick applied any plans,
/// encountered an error, or has never fired.
///
/// The scheduler (`spawn_consolidation_scheduler_with_feedback` in
/// `tdw-agent-store`) writes this cell after each tick. The type lives here
/// (in `tdw-knowledge`) so `tdw-agent-store` (which already depends on
/// `tdw-knowledge`) can write it without a circular dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ConsolidationFreshness {
    /// The scheduler has not yet fired (daemon just started, or tick interval
    /// has not elapsed). Reported as `"pending"` in `tdw.kg.status`.
    Pending,

    /// The last tick completed successfully. `applied_count` is the number of
    /// memories that were promoted or expired; zero means the store was already
    /// up-to-date (no actions needed).
    Ok {
        /// RFC 3339 timestamp of the last tick (`now` injected at tick time).
        last_tick_at: String,
        /// Memories before the tick.
        before_count: usize,
        /// Memories after the tick (after expirations).
        after_count: usize,
        /// Number of consolidation actions applied (promotions + expirations).
        applied_count: usize,
    },

    /// The last tick encountered a persistence error (e.g. could not write a
    /// `*.json5` file). The scheduler continues; this state records the most
    /// recent error for operator visibility.
    Error {
        /// RFC 3339 timestamp of the failing tick.
        last_tick_at: String,
        /// Human-readable error description.
        error: String,
    },
}

// ---------------------------------------------------------------------------
// K-L3: eval-freshness / drift status (knowledge-system-2 K-L3)
// ---------------------------------------------------------------------------

/// Eval-freshness / drift status for `tdw.kg.status` (K-L3).
///
/// Surfaced as `eval_freshness` on [`KgStatus`] so operators see at a glance
/// whether the last scheduled self-eval passed, regressed, or never ran.
///
/// The scheduled eval worker (in `tdw-eval-runner::scheduled_eval`) writes
/// this after each run via the shared cell on [`KnowledgeRuntime`]. The type
/// lives here (in `tdw-knowledge`) and is re-exported from `tdw-eval-runner`
/// so there is no circular dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum EvalFreshness {
    /// No golden split is configured — the scheduler has nothing to run.
    ///
    /// Not a silent do-nothing: the status field explicitly says no eval is
    /// scheduled, so operators notice quickly.
    Unconfigured,

    /// The scheduler is configured but has not run yet (no stored result).
    Pending {
        /// The split id the scheduler will run when it first fires.
        split_id: String,
    },

    /// The last eval run passed all thresholds.
    Green {
        /// Epoch-ms timestamp of the last eval run (`now_ms` when it ran).
        last_run_ms: i64,
        /// The split id that was evaluated.
        split_id: String,
        /// Summary line from the last verdict.
        summary: String,
    },

    /// The last eval run detected a metric regression beyond threshold.
    Regressed {
        /// Epoch-ms timestamp of the last eval run.
        last_run_ms: i64,
        /// The split id that was evaluated.
        split_id: String,
        /// Full summary from the regression verdict.
        summary: String,
        /// `true` when the baseline and current reports have different
        /// drift-key values (embedder model, rules version, or infer version
        /// changed between the baseline run and the current run).
        ///
        /// A metric drop that correlates with a key change is a *version
        /// effect* rather than a code regression — this flag lets operators
        /// distinguish the two causes without re-reading the raw report.
        drift_key_changed: bool,
    },

    /// The last eval run itself failed (engine error or empty case set).
    Stale {
        /// Epoch-ms timestamp of the last attempt.
        last_run_ms: i64,
        /// The split id attempted.
        split_id: String,
        /// Error message.
        error: String,
    },
}

impl EvalFreshness {
    /// Whether this status warrants an alert (regressed or stale).
    #[must_use]
    pub const fn is_alarm(&self) -> bool {
        matches!(self, Self::Regressed { .. } | Self::Stale { .. })
    }
}

// ---------------------------------------------------------------------------
// K-L4: auto-materialization sweep freshness
// ---------------------------------------------------------------------------

/// Observability status for the gated auto-materialization sweep (K-L4).
///
/// Surfaced as `proposals/auto-materialize` on [`KgStatus`] so operators see
/// at a glance whether the autonomous sweep is enabled and what it last did.
///
/// The sweep worker (in `tdw-backend`) writes this cell after every sweep run
/// via the shared `Arc<tokio::sync::Mutex<SweepFreshness>>` on
/// [`KnowledgeRuntime`]. The type lives here so `tdw-mcp` and
/// `tdw-backend` can both read it without a circular dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum SweepFreshness {
    /// `auto_materialize = false` — the sweep is intentionally disabled.
    /// Proposals that reach `Ready` wait for operator `materialize` action.
    Disabled,

    /// The sweep is enabled but has not fired yet since daemon start.
    Pending,

    /// The last sweep completed (possibly with zero landings — that is OK,
    /// not an alarm condition).
    Ran {
        /// Epoch-ms timestamp of the last sweep run.
        last_run_ms: i64,
        /// Number of proposals materialized in the last sweep.
        landed: usize,
        /// Number of `Ready` proposals that failed TOCTOU re-validation and
        /// were rejected rather than landed.
        rejected: usize,
        /// First engine error from the last sweep, if any.  Set when
        /// `materialize_ready_capped` returns `Err(…)` (i.e. a hard engine
        /// failure mid-sweep, distinct from per-proposal TOCTOU rejection).
        /// `None` means the sweep completed without a hard error.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_error: Option<String>,
    },
}

/// Watchlist change-detection freshness snapshot (knowledge-system K-X5).
///
/// Updated by the cron task after each tick; read by `status()` without
/// blocking (uses `try_lock`). `None` when the watchlist scheduler is not
/// running — `KgStatus.watchlist_status` is omitted from JSON in that case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchlistFreshness {
    /// Total active watches across all principals.
    pub watch_count: usize,
    /// Unix-epoch ms of the last check tick. `0` means never checked.
    pub last_check_ms: i64,
    /// Cumulative alerts fired since process start.
    pub total_alerts_fired: u64,
    /// Human-readable note when zero watches are configured.
    #[serde(default)]
    pub note: String,
}

impl Default for WatchlistFreshness {
    fn default() -> Self {
        Self {
            watch_count: 0,
            last_check_ms: 0,
            total_alerts_fired: 0,
            note: "no watchlists configured".to_string(),
        }
    }
}

/// Live freshness snapshot for the open-question matching engine (K-X8).
///
/// Updated by the cron task after each tick; read by `status()` without
/// blocking (`try_lock`).  `None` in `KgStatus` when the scheduler is not
/// running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionsFreshness {
    /// Number of open (not yet resolved/dismissed) questions.
    pub open_count: usize,
    /// Unix-epoch ms of the last matching-engine tick.  `0` = never checked.
    pub last_check_ms: i64,
    /// Cumulative candidate-match alerts fired since process start.
    pub total_matches_fired: u64,
    /// Human-readable note when zero open questions are present.
    #[serde(default)]
    pub note: String,
}

impl Default for QuestionsFreshness {
    fn default() -> Self {
        Self {
            open_count: 0,
            last_check_ms: 0,
            total_matches_fired: 0,
            note: "no open questions".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// K-E2: knowledge system observability status (knowledge-system-2 K-E2)
// ---------------------------------------------------------------------------

/// Proposal counts broken down by [`ValidationStatus`].
///
/// Counts reflect ALL pending (non-materialized, non-rejected) proposals,
/// obtained via [`ProposalQueue::pending_counts_by_state`] — a single-pass
/// scan that is NOT subject to the `LIST_PAGE_DEFAULT`/`LIST_PAGE_MAX`
/// pagination cap, so these are always exact regardless of queue depth.
/// Available only when the proposal queue is attached.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgProposalCounts {
    /// Proposals in the `Draft` state (submitted; automated validators not yet
    /// run or still in progress).
    pub draft: usize,
    /// Proposals in the `Validated` state (automated validators passed; awaiting
    /// eval promotion or human approval).
    pub validated: usize,
    /// Proposals in the `Ready` state (approved; pending materialization into the
    /// graph/tag engines).
    pub ready: usize,
    /// Whether the runtime has OPERATOR authority (approve/reject/materialize
    /// actions enabled). A read-only agent-facing runtime reports `false`.
    pub operator_authority: bool,
    /// Lessons (K-R1) enqueued but not yet materialized — proposed behavior
    /// changes awaiting the eval gate. Additive: a subset view of the pending
    /// annotation proposals carrying the lesson marker.
    #[serde(default)]
    pub pending_lessons: usize,
    /// Lessons (K-R1) whose proposal materialized — installed, behavior-
    /// influencing. Reported truthfully (only materialized lessons count).
    #[serde(default)]
    pub active_lessons: usize,
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

/// Thesis summary line for `tdw.kg.status` (K-X7).
///
/// A lightweight observability signal: how many thesis nodes exist, and which
/// ids are at the extremes of the evidence balance.  These fields are computed
/// cheaply at status-collection time — **no full health scan is run**; only the
/// count of thesis-kind Finding nodes is derived from a bounded graph page walk
/// (capped at [`THESIS_STATUS_PAGE_CAP`] nodes).  Call `tdw.kg.thesis_health`
/// for full evidence counts on a specific thesis.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgThesesStatus {
    /// Number of thesis nodes (Finding nodes with `props.kind_hint = "thesis"`)
    /// visible in the bounded graph scan.
    pub count: usize,
    /// Id of the thesis with the most `supports` inbound edges (approximate:
    /// derived from the bounded scan page, not a full graph scan). `None` when
    /// no theses exist or when counts are tied.
    #[serde(default)]
    pub most_supported_id: Option<String>,
    /// Id of the thesis with the most `contradicts` inbound edges (approximate,
    /// same bounded scan). `None` when no theses exist or when counts are tied.
    #[serde(default)]
    pub most_contradicted_id: Option<String>,
    /// Present when the thesis count was bounded by the page cap and the true
    /// count may be higher.
    #[serde(default)]
    pub scan_note: Option<String>,
}

/// Page cap for the thesis-count scan in `status()`.  Large enough to cover
/// typical graphs; honest `scan_note` fires when exceeded.
pub const THESIS_STATUS_PAGE_CAP: usize = 4_096;

/// Full observability snapshot for one `KnowledgeRuntime` instance.
///
/// Collected by [`KnowledgeRuntime::status`] and surfaced identically by the
/// `tdw.kg.status` MCP tool, the `GET /api/v1/knowledge/status` REST endpoint,
/// and the `tdw kg status` CLI subcommand (K-E2).
///
/// **Honest notes on unavailable stats** are carried in the corresponding
/// `*_note` / `error` fields rather than omitted, so callers can distinguish
/// Cumulative contradiction-detection counts surfaced by [`KgStatus`] (K-M4).
///
/// Counts accumulate across all `tdw.kg.ingest` and `tdw.kg.proposals
/// materialize` calls since the server started.  Omitted from JSON when no
/// `ContradictionDetector` is wired (`contradiction_totals` field is `None`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgContradictionCounts {
    /// Total edges closed (temporally invalidated) across all scans.
    pub invalidated: usize,
    /// Total conflicts surfaced (user/manual provenance — not auto-closed).
    pub conflicts: usize,
}

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

    // --- Retrieval eval freshness / drift (autonomy-gap#9, K-L3) ---
    /// Live eval-freshness status from the scheduled self-eval (K-L3).
    ///
    /// Reports whether the last scheduled retrieval eval passed, regressed,
    /// staled, or has never run (pending / unconfigured).  `Unconfigured` is
    /// the honest loudly-visible default when no golden split is wired: the
    /// scheduler runs nothing and this field says so explicitly.
    pub eval_freshness: EvalFreshness,

    // --- Consolidation freshness (autonomy-gaps #2+#8, K-L2) ---
    /// Live consolidation-freshness status from the periodic consolidation tick
    /// (K-L2). Reports the last tick time, before/after memory counts, and how
    /// many plans were applied. `Pending` until the first tick fires.
    pub consolidation_freshness: ConsolidationFreshness,
    // --- Auto-materialization sweep (K-L4) ---
    /// Live freshness status for the gated auto-materialization sweep (K-L4).
    ///
    /// Reports whether the sweep is enabled (`Pending` / `Ran`) or explicitly
    /// disabled (`Disabled`). `Disabled` means `auto_materialize = false` in
    /// `[knowledge.proposals]` — the operator tool is then the only landing
    /// path.
    pub auto_materialize_freshness: SweepFreshness,

    // --- K-M4 contradiction detection totals ---
    /// Cumulative contradiction-detection counts since server start (K-M4).
    ///
    /// `None` when no `ContradictionDetector` is wired (field omitted from
    /// JSON).  When present, `invalidated` is the total number of
    /// functional-predicate edges auto-closed across all ingest and
    /// materialize scans; `conflicts` is the total number of user/manual-
    /// provenance contradictions surfaced (not auto-closed — require manual
    /// review).  Populated by the MCP layer; `runtime.status()` always
    /// returns `None` to keep `tdw-knowledge` decoupled from `tdw-infer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contradiction_totals: Option<KgContradictionCounts>,

    // --- Theses (K-X7) ---
    /// Thesis summary for the attached graph (K-X7).
    ///
    /// `None` when no graph engine is attached (no Finding nodes can exist
    /// without a graph).  When present, carries the count of thesis nodes
    /// (Finding nodes with `props.kind_hint = "thesis"`), plus the ids of
    /// the most evidence-rich (healthiest) and most-contradicted theses —
    /// both bounded to 1 id each (cheapest indicative signal without a full
    /// health scan).  An honest `scan_note` is included when the count is
    /// bounded by the graph page limit.
    #[serde(default)]
    pub theses: Option<KgThesesStatus>,

    // --- Scheduled feed freshness (K-L6) ---
    /// Per-feed freshness snapshots from all configured feeds (K-L6).
    ///
    /// Each entry is the last-known [`FeedFreshness`] for one feed, in
    /// registration order. Empty when no feeds are configured (status note
    /// makes this visible so operators notice the gap rather than reading
    /// silence as "healthy").
    #[serde(default)]
    pub feed_statuses: Vec<FeedFreshness>,
    /// Honest note when no feeds are configured.
    ///
    /// `"no feeds configured"` when `feed_statuses` is empty; empty string
    /// otherwise. Loud-by-design: operators see the absence explicitly.
    pub feed_note: String,

    // --- Watchlist change-detection freshness (K-X5) ---
    /// Live watchlist change-detection freshness snapshot (K-X5).
    ///
    /// `None` when the watchlist scheduler is not running — field is omitted
    /// from JSON so absence is explicit (loud by design). When present, carries
    /// the count of active watches, last-check timestamp, and cumulative alerts
    /// fired since daemon start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchlist_status: Option<WatchlistFreshness>,

    // --- Open-question matching-engine freshness (K-X8) ---
    /// Live open-question matching-engine freshness snapshot (K-X8).
    ///
    /// `None` when the question scheduler is not running — field is omitted
    /// from JSON so absence is explicit (loud by design). When present, carries
    /// the count of open questions, last-check timestamp, and cumulative
    /// candidate-match alerts fired since daemon start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions_status: Option<QuestionsFreshness>,
    // --- LLM extraction-as-proposals (K-M1) ---
    /// Live LLM extraction-as-proposals freshness snapshot (K-M1).
    ///
    /// `None` when the extraction stage is not wired (`[knowledge.extraction]
    /// enabled = false`, the default, or no production-grade model available).
    /// When present, carries the last-batch entity/relation enqueue counts,
    /// tokens consumed, and remaining daily budget.
    ///
    /// The differentiator: `Disabled` with reason `"stub model"` means a
    /// production-grade LLM is NOT wired — the deterministic pipeline runs
    /// unchanged.  `Disabled` with reason `"enabled = false"` means the
    /// operator has explicitly opted out.  Either way the field is loud so
    /// operators see the gap without reading silence as "healthy".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction_freshness: Option<crate::ExtractionFreshness>,

    // --- Skill lifecycle counts (K-R2) ---
    /// Live skill counts by lifecycle state (candidate / active / retired), K-R2.
    ///
    /// `None` when skill lifecycle is not wired (field omitted from JSON so the
    /// absence is explicit — loud by design). When present, reflects the last
    /// tournament tick's view of the [`SkillRegistry`](crate::skills::SkillRegistry).
    /// Surfaced ADDITIVELY: existing `tdw.kg.status` fields are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<crate::skills::SkillCounts>,
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
    #[allow(clippy::too_many_lines)] // K-L6 added feed-freshness snapshot; refactor deferred
    pub async fn status(&self) -> KgStatus {
        let versions_snapshot = self
            .versions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let vector_collection = crate::collection_name(&versions_snapshot.embedder_model);

        let graph_health = if let Some(graph) = self.graph.as_ref() {
            let probe = graph.edges(None, 0, 1).await;
            // Use the operator-supplied name when available; fall back to the
            // generic sentinel so the field is always a non-empty string.
            let backend_name = self
                .graph_name
                .clone()
                .unwrap_or_else(|| "graph-engine".to_string());
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
            // we report zero counts rather than deadlocking.
            queue_mutex.try_lock().map_or(
                KgProposalCounts {
                    draft: 0,
                    validated: 0,
                    ready: 0,
                    operator_authority: self.operator_authority,
                    pending_lessons: 0,
                    active_lessons: 0,
                },
                |queue| {
                    // Single-pass exact count — not subject to LIST_PAGE caps.
                    let (draft, validated, ready) = queue.pending_counts_by_state();
                    // K-R1: additive lesson counts (truthful — active counts
                    // only materialized lessons).
                    let lessons = queue.lesson_counts();
                    KgProposalCounts {
                        draft,
                        validated,
                        ready,
                        operator_authority: self.operator_authority,
                        pending_lessons: lessons.pending,
                        active_lessons: lessons.active,
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

        // Eval-freshness (K-L3): read the shared cell if wired, else
        // Unconfigured.  `try_lock` avoids blocking: if the eval worker is
        // mid-write we report the last known state (or Unconfigured) rather
        // than deadlocking the status handler.
        let eval_freshness =
            self.eval_freshness
                .as_ref()
                .map_or(EvalFreshness::Unconfigured, |cell| {
                    cell.try_lock()
                        .map_or(EvalFreshness::Unconfigured, |guard| guard.clone())
                });

        // Consolidation-freshness (K-L2): read the shared cell if wired, else
        // Pending (scheduler not yet attached or first tick has not fired).
        // `try_lock` avoids blocking: if the scheduler is mid-write we return
        // the last known state (or Pending) rather than deadlocking.
        let consolidation_freshness =
            self.consolidation_freshness
                .as_ref()
                .map_or(ConsolidationFreshness::Pending, |cell| {
                    cell.try_lock()
                        .map_or(ConsolidationFreshness::Pending, |guard| guard.clone())
                });

        // Auto-materialize sweep freshness (K-L4): read the shared cell if
        // wired. `None` cell → `Disabled` (sweep was not registered, meaning
        // `auto_materialize = false`). `try_lock` avoids blocking when the
        // sweep worker is mid-write — report the last known state instead.
        let auto_materialize_freshness =
            self.auto_materialize_freshness
                .as_ref()
                .map_or(SweepFreshness::Disabled, |cell| {
                    cell.try_lock()
                        .map_or(SweepFreshness::Pending, |guard| guard.clone())
                });

        // Thesis summary (K-X7): scan Finding nodes for kind_hint="thesis" using
        // a bounded page walk.  This is a lightweight count probe — no full
        // health scan.  Only runs when a graph engine is attached.
        let theses = if let Some(graph) = self.graph.as_ref() {
            let theses_status = collect_theses_status(graph).await;
            Some(theses_status)
        } else {
            None
        };

        // Feed freshness (K-L6): snapshot each cell with try_lock to avoid
        // blocking the status handler when a feed task is mid-write.
        let feed_statuses: Vec<FeedFreshness> = self
            .feed_freshness_cells
            .iter()
            .map(|cell| {
                cell.try_lock().map_or_else(
                    |_| FeedFreshness::Pending {
                        feed_id: "?".to_string(),
                    },
                    |g| g.clone(),
                )
            })
            .collect();
        let feed_note = if feed_statuses.is_empty() {
            "no feeds configured — knowledge acquisition is manual only".to_string()
        } else {
            String::new()
        };

        // Watchlist freshness (K-X5): read the shared cell if wired, else None.
        // `try_lock` avoids blocking: if the cron task is mid-write (WouldBlock)
        // or the lock is poisoned, return the default rather than deadlocking.
        let watchlist_status =
            self.watchlist_freshness
                .as_ref()
                .map(|cell| match cell.try_lock() {
                    Ok(guard) => guard.clone(),
                    Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().clone(),
                    Err(std::sync::TryLockError::WouldBlock) => WatchlistFreshness::default(),
                });

        // Questions freshness (K-X8): read the shared cell if wired, else None.
        // Same `try_lock` discipline as watchlist — never block `status()`.
        let questions_status =
            self.questions_freshness
                .as_ref()
                .map(|cell| match cell.try_lock() {
                    Ok(guard) => guard.clone(),
                    Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().clone(),
                    Err(std::sync::TryLockError::WouldBlock) => QuestionsFreshness::default(),
                });

        // Extraction freshness (K-M1): read the shared cell if wired, else None.
        // `try_lock` avoids blocking: if the indexer is mid-extraction we return
        // the last known state rather than deadlocking the status handler.
        let extraction_freshness =
            self.extraction_freshness
                .as_ref()
                .map(|cell| match cell.try_lock() {
                    Ok(guard) => guard.clone(),
                    Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().clone(),
                    Err(std::sync::TryLockError::WouldBlock) => crate::ExtractionFreshness::Pending,
                });

        // Skill counts (K-R2): read the shared cell if wired, else None.
        // `try_lock` avoids blocking when the tournament worker is mid-write —
        // a poisoned lock yields the inner snapshot, a contended one the default
        // (zeroed) counts rather than deadlocking the status handler.
        let skills = self
            .skill_counts
            .as_ref()
            .map(|cell| match cell.try_lock() {
                Ok(guard) => *guard,
                Err(std::sync::TryLockError::Poisoned(p)) => *p.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => crate::skills::SkillCounts::default(),
            });

        KgStatus {
            vector_collection,
            embedder_model: versions_snapshot.embedder_model.clone(),
            document_count_note: "VectorEngine has no count() in this version; \
                                  use Qdrant dashboard or lexical engine for a precise count"
                .to_string(),
            taxonomy_kind_count: EntityKind::ALL.len(),
            graph_health,
            versions: versions_snapshot,
            proposals,
            language_model_grade,
            eval_freshness,
            consolidation_freshness,
            auto_materialize_freshness,
            // Populated by the MCP layer when a ContradictionDetector is wired;
            // None here so runtime.status() stays decoupled from tdw-infer.
            contradiction_totals: None,
            theses,
            feed_statuses,
            feed_note,
            watchlist_status,
            questions_status,
            extraction_freshness,
            skills,
        }
    }
}

/// Rank evidence (supports/contradicts) across a set of thesis ids; returns
/// (`most_supported_id`, `most_contradicted_id`).
async fn rank_thesis_evidence(
    graph: &std::sync::Arc<dyn GraphEngine>,
    thesis_ids: &[(String, String)],
) -> (Option<String>, Option<String>) {
    use tdw_core::{Direction, TraversalFilter};

    let evidence_filter = TraversalFilter {
        rels: Some(vec!["supports".to_string(), "contradicts".to_string()]),
        direction: Direction::In,
        max_hops: 1,
        ..TraversalFilter::default()
    };

    let mut best_supported: Option<(String, usize)> = None;
    let mut most_contradicted: Option<(String, usize)> = None;

    for (thesis_id, _) in thesis_ids {
        let Ok(neighbors) = graph.neighbors(thesis_id, &evidence_filter).await else {
            continue;
        };
        let sup = neighbors
            .iter()
            .filter(|(e, _)| e.rel == "supports")
            .count();
        let con = neighbors
            .iter()
            .filter(|(e, _)| e.rel == "contradicts")
            .count();

        if sup > 0 {
            match &best_supported {
                None => best_supported = Some((thesis_id.clone(), sup)),
                Some((_, prev)) if sup > *prev => best_supported = Some((thesis_id.clone(), sup)),
                _ => {}
            }
        }
        if con > 0 {
            match &most_contradicted {
                None => most_contradicted = Some((thesis_id.clone(), con)),
                Some((_, prev)) if con > *prev => {
                    most_contradicted = Some((thesis_id.clone(), con));
                }
                _ => {}
            }
        }
    }

    (
        best_supported.map(|(id, _)| id),
        most_contradicted.map(|(id, _)| id),
    )
}

/// Collect thesis count + top-supported/top-contradicted ids from the graph.
///
/// Uses a bounded page walk over edges, filtering candidate nodes for
/// `props.kind_hint = "thesis"`.  Best-effort: graph probe failures return a
/// zero-count status rather than propagating the error.
async fn collect_theses_status(graph: &std::sync::Arc<dyn GraphEngine>) -> KgThesesStatus {
    // Page walk: collect up to THESIS_STATUS_PAGE_CAP node ids from the edge list.
    let mut thesis_ids: Vec<(String, String)> = Vec::new(); // (id, title)
    let mut offset = 0usize;
    let page_size = 256usize;
    let mut capped = false;

    'outer: loop {
        let Ok(page) = graph.edges(None, offset, page_size).await else {
            break;
        };
        if page.is_empty() {
            break;
        }
        for edge in &page {
            for candidate_id in [&edge.from, &edge.to] {
                if thesis_ids.iter().any(|(id, _)| id == candidate_id) {
                    continue;
                }
                if thesis_ids.len() >= THESIS_STATUS_PAGE_CAP {
                    capped = true;
                    break 'outer;
                }
                if let Ok(Some(node)) = graph.node(candidate_id).await
                    && node.kind == EntityKind::Finding
                    && node.props.get("kind_hint").and_then(|v| v.as_str()) == Some("thesis")
                {
                    let title = node
                        .props
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    thesis_ids.push((candidate_id.clone(), title));
                }
            }
        }
        offset += page.len();
        if offset >= THESIS_STATUS_PAGE_CAP * 4 {
            capped = true;
            break;
        }
    }

    let scan_note = if capped {
        Some(format!(
            "thesis scan bounded at {THESIS_STATUS_PAGE_CAP} nodes; true count may be higher"
        ))
    } else {
        None
    };

    let count = thesis_ids.len();
    if count == 0 {
        return KgThesesStatus {
            count: 0,
            most_supported_id: None,
            most_contradicted_id: None,
            scan_note,
        };
    }

    let (most_supported_id, most_contradicted_id) = rank_thesis_evidence(graph, &thesis_ids).await;

    KgThesesStatus {
        count,
        most_supported_id,
        most_contradicted_id,
        scan_note,
    }
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
        let runtime = vector_only_runtime()
            .with_graph(graph)
            .with_graph_name("in-memory");
        let status = runtime.status().await;
        let health = status.graph_health.expect("graph attached");
        assert!(health.reachable, "in-memory engine always reachable");
        assert!(health.error.is_none());
        assert_eq!(health.backend_name, "in-memory", "explicit name used");
    }

    #[tokio::test]
    async fn status_graph_without_name_falls_back_to_sentinel() {
        use tdw_storage_graph::InMemoryGraphEngine;
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // No with_graph_name — sentinel must be returned, not a type-name heuristic.
        let runtime = vector_only_runtime().with_graph(graph);
        let status = runtime.status().await;
        let health = status.graph_health.expect("graph attached");
        assert_eq!(
            health.backend_name, "graph-engine",
            "sentinel when no name supplied"
        );
    }

    #[tokio::test]
    async fn proposal_counts_are_exact_and_include_draft() {
        use std::sync::Arc;
        use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
        use tdw_storage_qdrant::InMemoryVectorEngine;

        use crate::proposals::{ProposalKind, ProposalQueue};
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_taxonomy::Adaptivity;

        // Build a full runtime so submit() can run validators.
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        // GraphTagEngine<G> stores G by value and needs G: GraphEngine — it
        // cannot take Arc<dyn GraphEngine>. Use two separate in-memory instances:
        // one (erased) for the runtime graph handle, one (concrete) for the tags
        // engine. The validators only need the tag engine to check tag-assign
        // shape; the graph instances can be independent for this test.
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let tags: Arc<dyn tdw_tags::TagEngine> =
            Arc::new(GraphTagEngine::new(InMemoryGraphEngine::default()));

        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        // Submit two TagDefine proposals. These pass the validator without
        // needing pre-existing graph nodes (the check is only grammar + uniqueness)
        // and land as Validated after auto-validators run.
        {
            let mut q = queue.lock().await;
            q.submit(
                "agent-a",
                Adaptivity::Learning,
                ProposalKind::TagDefine {
                    tag_id: "status-test:alpha".to_string(),
                    parent: None,
                },
                &graph,
                &tags,
                "2026-06-11",
            )
            .await
            .expect("submit 1");
            q.submit(
                "agent-a",
                Adaptivity::Learning,
                ProposalKind::TagDefine {
                    tag_id: "status-test:beta".to_string(),
                    parent: None,
                },
                &graph,
                &tags,
                "2026-06-11",
            )
            .await
            .expect("submit 2");
            drop(q);
        }

        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(graph)
            .with_proposals(queue);
        let status = runtime.status().await;
        let counts = status.proposals.expect("proposals attached");
        // Both proposals enter as Validated (Draft→Validated after auto-validate).
        assert_eq!(counts.draft, 0, "no draft proposals");
        assert_eq!(counts.validated, 2, "two validated proposals");
        assert_eq!(counts.ready, 0, "none ready yet");
    }

    // ── K-L3: eval_freshness field ────────────────────────────────────────────

    #[tokio::test]
    async fn status_eval_freshness_unconfigured_when_no_cell_attached() {
        let runtime = vector_only_runtime();
        let status = runtime.status().await;
        assert_eq!(
            status.eval_freshness,
            EvalFreshness::Unconfigured,
            "no cell → Unconfigured"
        );
    }

    #[tokio::test]
    async fn status_eval_freshness_reads_shared_cell() {
        use std::sync::Arc;
        let cell = Arc::new(tokio::sync::Mutex::new(EvalFreshness::Green {
            last_run_ms: 1_749_297_600_000,
            split_id: "golden-split-v1".to_string(),
            summary: "OK on split".to_string(),
        }));
        let runtime = vector_only_runtime().with_eval_freshness(cell.clone());
        let status = runtime.status().await;
        assert!(
            matches!(status.eval_freshness, EvalFreshness::Green { .. }),
            "cell Green → status Green"
        );
    }

    #[tokio::test]
    async fn status_eval_freshness_regressed_is_alarm() {
        use std::sync::Arc;
        let cell = Arc::new(tokio::sync::Mutex::new(EvalFreshness::Regressed {
            last_run_ms: 1_749_297_600_000,
            split_id: "golden-split-v1".to_string(),
            summary: "REGRESSION on split".to_string(),
            drift_key_changed: false,
        }));
        let runtime = vector_only_runtime().with_eval_freshness(cell);
        let status = runtime.status().await;
        assert!(
            status.eval_freshness.is_alarm(),
            "Regressed freshness must be an alarm"
        );
    }

    #[tokio::test]
    async fn status_serializes_eval_freshness_field() {
        let runtime = vector_only_runtime();
        let status = runtime.status().await;
        let json = serde_json::to_value(&status).expect("status serializes");
        assert!(
            json["eval_freshness"].is_object(),
            "eval_freshness must be a JSON object; got: {}",
            json["eval_freshness"]
        );
        assert_eq!(
            json["eval_freshness"]["state"],
            serde_json::Value::String("unconfigured".to_string()),
            "default state must serialize as 'unconfigured'"
        );
    }
}
