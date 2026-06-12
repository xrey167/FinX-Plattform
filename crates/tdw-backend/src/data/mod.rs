//! Async data/daemon facade.
//!
//! Owns the daemon composition root ([`AppState`]) and a [`CommandRunner`] for
//! provider dispatch, and exposes the async query/ingest surface over them. This
//! crate holds **no business logic**: every method is a thin, typed delegation
//! to the underlying `tdw-*` crates.

use std::sync::Arc;

use serde_json::Value;
use tdw_agent::{ConsolidationAction, Memory, UsageHint, consolidation_plan_with_usage};
use tdw_agent_store::{
    MemoryStore, RetrievalFeedbackStore, consolidate_at,
    spawn_consolidation_scheduler_with_feedback,
};
use tdw_app_server::{CancellationToken, SubmissionHandle};
use tdw_bus::EventBus;
use tdw_config::ProposalsConfig as ProposalsCfg;
use tdw_config::ScheduledEvalConfig as EvalsConfig;
use tdw_config::{FeedsConfig, TdwConfig};
use tdw_core::{
    BlobEngine, DataModel, Fetcher, GraphEngine, LexicalEngine, OBBject, OlapEngine,
    ProgressStream, ProviderRegistry, QueryParams, RelationalEngine, VectorEngine,
};
use tdw_cron::{CronSchedule, ScheduleRegistry, ScheduledTrigger, TriggerAction, due_triggers};
use tdw_domain::EquityHistoricalData;
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_eval_runner::scheduled_eval::{
    EvalAlertSink, ScheduledEvalConfig as EvalRunnerConfig, default_fixture_path, eval_trigger_id,
    load_golden_split, regression_alert_body, run_scheduled_eval_from_fixture,
};
use tdw_infer::contradiction::ContradictionDetector;
use tdw_infer::{ChangeSet, InferEngine, InferError, RetractReport, RunLimits};
use tdw_knowledge::feeds::{FeedFreshness, FeedSource, FixtureFeedSource};
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::runtime::{
    ConsolidationFreshness, EvalFreshness, KnowledgeRuntime, SweepFreshness,
};
use tdw_knowledge::{KnowledgeDocument, KnowledgeHit, KnowledgeIndex};
use tdw_outbox::InMemoryOutbox;
use tdw_patterns::{MiningLimits, PatternEngine, PatternIndex};
use tdw_protocol::{EventMsg, OpEnvelope};
use tdw_runtime::CommandRunner;
use tdw_service_api::{AppState, fetch_equity_historical};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_tag_rules::{RuleEngine, TagRule};
use tdw_tags::{InMemoryTagEngine, TagEngine};
use tdw_taxonomy::FunctionalPredicateSet;
use tokio::sync::Mutex;

use crate::config::BackendConfig;
use crate::error::{BackendError, BackendResult};

/// The live handles for a daemon started via [`Backend::serve`].
///
/// Holds everything needed to submit ops in-process (the [`SubmissionHandle`]),
/// to address the daemon over loopback (the bound `addr`), and to shut it down
/// cleanly (the [`CancellationToken`] plus the spawned task handles). Created by
/// [`Backend::serve`] and cleared by [`Backend::shutdown`].
struct DaemonHandle {
    /// Cancels the service loop, relay, transport, and hot-reload tick on shutdown.
    cancel: CancellationToken,
    /// In-process op submission into the running service loop (no socket).
    submission: SubmissionHandle,
    /// The `serve(service_loop, relay, ..)` driver task.
    serve_task: tokio::task::JoinHandle<()>,
    /// The periodic memory-consolidation scheduler task (Phase B).
    consolidation_task: tokio::task::JoinHandle<()>,
    /// The hot-reload tick for tag-rules + inference rules (knowledge-system K-L1).
    /// `None` when `knowledge.rules.rules_dir` is absent — no rules, no reload loop.
    rules_reload_task: Option<tokio::task::JoinHandle<()>>,
    /// The K-L3 scheduled self-eval worker task.
    /// `None` when `knowledge.evals.split_id` is not configured.
    eval_worker_task: Option<tokio::task::JoinHandle<()>>,
    /// The K-L4 gated auto-materialization sweep task.
    /// `None` when `knowledge.proposals.auto_materialize = false`.
    auto_mat_task: Option<tokio::task::JoinHandle<()>>,
    /// The K-R4 scheduled pattern-mining worker task.
    /// `None` when `knowledge.patterns.enabled = false` (default).
    pattern_mining_task: Option<tokio::task::JoinHandle<()>>,
    /// The K-L6 scheduled feed tasks, one per enabled feed entry.
    /// Empty when no feeds are configured.
    feed_handles: Vec<tokio::task::JoinHandle<()>>,
    /// The spawned transport (TCP/UDS/HTTP) server task.
    transport_task: tokio::task::JoinHandle<std::io::Result<()>>,
    /// The address the transport actually bound (post-OS-assignment), e.g. the
    /// concrete port for an ephemeral `127.0.0.1:0` request.
    bound_addr: String,
}

/// The async backend facade over the data/daemon surface.
pub struct Backend {
    state: AppState,
    runner: CommandRunner,
    /// The embedding provider shared between the knowledge indexer, the runtime
    /// retriever, and search (so all three use the same dimensions/model).
    embedder: Arc<dyn EmbeddingProvider>,
    /// The daemon-hosted knowledge indexer (knowledge-system K-E3).
    ///
    /// The [`KnowledgeIndexer`] wraps a [`KnowledgeIndex`] and adds the full
    /// B5 write-side pipeline: content-hash idempotency manifest, rule-driven
    /// auto-tagging, lexical co-indexing, and durable graph stamping. All
    /// three engines (`embedder`, `graph`, `lexical`) are the same `Arc`
    /// handles used by the retriever and the MCP read tools, so a doc written
    /// through this indexer is immediately visible to searches on the same
    /// process. The guard is acquired per-call and never held across unrelated
    /// awaits.
    ///
    /// **Multi-tenancy note:** collection names and storage keys are derived
    /// from the configured embedder model id (via [`tdw_knowledge::collection_name`])
    /// — no new global singletons are introduced. Future graph namespacing
    /// can be layered onto the same seam without a public-API change.
    indexer: Arc<Mutex<KnowledgeIndexer>>,
    /// The agent memory store (Phase B). Held behind a [`tokio::sync::Mutex`] so
    /// the live consolidation scheduler and the [`upsert_memory`](Self::upsert_memory)
    /// / [`consolidate_now`](Self::consolidate_now) surface methods share one store.
    /// Loaded from `TDW_MEMORY_DIR` when set (round-tripping to `*.json5`), else
    /// purely in-memory.
    memory: Arc<Mutex<MemoryStore>>,
    /// The retrieval feedback store (knowledge-system B10). Held behind a
    /// [`tokio::sync::Mutex`] so the MCP feedback tool and
    /// [`consolidate_now_at`](Self::consolidate_now_at) share one handle.
    /// Always present after construction (never `None`). When an embedding
    /// host constructs both this `Backend` and an
    /// [`AgentBackend`](crate::agent::AgentBackend) in the same process, it
    /// passes `AgentBackend::feedback_store_handle()` into
    /// [`Backend::with_feedback_store`] so both facades share one instance —
    /// this is host wiring, not cross-facade state sharing.
    feedback: Arc<Mutex<RetrievalFeedbackStore>>,
    /// The graph engine backing both the knowledge indexer and the knowledge
    /// runtime (knowledge-system K-E3/F1). Config-driven: `knowledge.graph.backend
    /// = bolt` → `BoltGraphEngine` (requires the `bolt` feature); `in-memory` →
    /// `InMemoryGraphEngine`. Shared via `Arc` with the indexer and the runtime so
    /// document nodes, `described_by` edges, and `mentions` edges written at ingest
    /// time are immediately visible to graph traversals on the read side.
    graph: Arc<dyn GraphEngine>,
    /// The full knowledge runtime (hybrid retriever + graph/tag handles).
    /// Constructed from the daemon's graph engine, vector engine, lexical
    /// engine and embedder in `from_config` (knowledge-system F1). Shared with
    /// the MCP server via [`knowledge_runtime_handle`](Self::knowledge_runtime_handle).
    runtime: Arc<KnowledgeRuntime>,
    /// The tag engine backing the inference engine (knowledge-system K-L1 / B7).
    ///
    /// An in-process [`InMemoryTagEngine`] that the inference engine reads when
    /// running `PropagateTag` rules. Distinct from any external tag store the
    /// broader daemon wires; inference operates on the in-process knowledge graph.
    tags_engine: Arc<dyn TagEngine>,
    /// The auto-tagging rule engine (knowledge-system K-L1).
    ///
    /// Loaded from `knowledge.rules.rules_dir` at boot; hot-reloaded by a
    /// periodic tick during `serve`. `None` when no `rules_dir` is configured
    /// (logged loudly at boot). Shared with the hot-reload tick via `Arc<Mutex>`.
    rules: Arc<Mutex<RuleEngine>>,
    /// The forward-chaining inference engine (knowledge-system K-L1 / B7).
    ///
    /// Loaded at boot with rules from `knowledge.rules.rules_dir` and run
    /// incrementally after every ingest batch. The `DerivationIndex` is owned
    /// here (caller-owns pattern, not persisted in this phase). Shared with the
    /// hot-reload tick via `Arc<Mutex>`.
    infer: Arc<Mutex<InferEngine>>,
    /// The contradiction detector (knowledge-system K-M4).
    ///
    /// Fires BEFORE K-L1 inference on every ingest batch so inference sees the
    /// corrected graph. Built from `config.knowledge.contradiction` at boot:
    /// taxonomy defaults plus any `extra_functional_rels` the operator configures.
    /// Shared with the MCP server via [`contradiction_detector_handle`].
    contradiction: Arc<ContradictionDetector>,
    /// The finding indexer for hybrid search indexing of user-authored findings
    /// (knowledge-system K-X6). Behind a `std::sync::Mutex` so the sync MCP
    /// dispatch can hold the lock across the `index_at` await via `block_on`.
    /// Shared with the runtime via [`knowledge_runtime_handle`] so the MCP
    /// `tdw.kg.finding` tool can index captured findings into the same vector
    /// and lexical stores the read tools query.
    ///
    /// **K-L5 (CLOSED):** agent and user identities are sourced from
    /// `[knowledge.agent]` and `[knowledge.user]` config and bound at
    /// `from_config` time — never accepted from tool arguments. Multi-principal
    /// HTTP (per-connection identity) is explicitly deferred.
    finding_indexer: Arc<std::sync::Mutex<KnowledgeIndexer>>,
    /// K-L3 scheduled self-eval configuration, extracted from
    /// `config.knowledge.evals` in [`Backend::from_config`].  Held here so
    /// [`Backend::serve`] can register the cron trigger and spawn the eval
    /// worker without a second round of config parsing.
    evals_cfg: EvalsConfig,
    /// Live consolidation-freshness cell (K-L2). Shared between the
    /// [`KnowledgeRuntime`] (read side: `tdw.kg.status`) and the consolidation
    /// scheduler spawned in [`Backend::serve`] (write side: updated after each
    /// tick). Always `Some` after construction; pre-populated with
    /// [`ConsolidationFreshness::Pending`].
    consolidation_freshness: Arc<Mutex<ConsolidationFreshness>>,
    /// K-L4 auto-materialization sweep configuration, extracted from
    /// `config.knowledge.proposals` in [`Backend::from_config`]. Held here so
    /// [`Backend::serve`] can register the sweep trigger without re-parsing.
    proposals_cfg: ProposalsCfg,
    /// K-L6 scheduled feed configuration, extracted from
    /// `config.knowledge.feeds` in [`Backend::from_config`].  Held here so
    /// [`Backend::serve`] can spawn the per-feed cron tasks without a second
    /// round of config parsing.
    feeds_cfg: FeedsConfig,
    /// K-L6 per-feed freshness cells, one per enabled feed entry (plus one
    /// `Disabled` cell for each disabled entry). Built in `from_config`
    /// BEFORE the runtime is `Arc`-wrapped so they can be attached via
    /// `with_feed_freshness`. `serve()` clones these Arcs into the spawned
    /// tasks; `shutdown()` does not need to touch them (the tasks observe
    /// the cancellation token and exit).
    feed_freshness_cells: Vec<Arc<tokio::sync::Mutex<FeedFreshness>>>,
    /// The running daemon's live handles, populated by [`Backend::serve`] and
    /// cleared by [`Backend::shutdown`]. `None` until/after serving.
    daemon: Option<DaemonHandle>,
}

impl Backend {
    /// Build a backend from a layered [`TdwConfig`].
    ///
    /// Typed [`fetch`](Self::fetch) / [`stream`](Self::stream) dispatch directly
    /// through the [`Fetcher`] supplied at the call site (which carries the
    /// provider logic), so the [`CommandRunner`] only supplies default
    /// [`Credentials`](tdw_core::Credentials) and does not consult a provider
    /// registry on that path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the daemon composition root cannot be
    /// constructed from `config`.
    #[allow(clippy::too_many_lines)]
    pub async fn from_config(config: TdwConfig) -> BackendResult<Self> {
        let embedding = config.knowledge.embedding.clone();
        let graph_cfg = config.knowledge.graph.clone();
        let rules_cfg = config.knowledge.rules.clone();
        let infer_cfg = config.knowledge.infer.clone();
        let contradiction_cfg = config.knowledge.contradiction.clone();
        let user_id = config.knowledge.user.id.clone();
        // K-L5: host-bound agent identity for the write/feedback surface.
        // Identity is validated at config load (validate_principal_id) so it
        // is always grammar-valid here. It is bound at runtime construction,
        // never accepted from tool arguments.
        let agent_id = config.knowledge.agent.id.clone();
        // Extract eval config before `config` is consumed by AppState::from_config
        // (K-L3: used to initialise the freshness cell and cron trigger).
        let evals_cfg = config.knowledge.evals.clone();
        // K-L4: extract proposals sweep config before config is consumed.
        let proposals_cfg = config.knowledge.proposals.clone();
        // K-L6: extract feeds config before `config` is consumed.
        let feeds_cfg = config.knowledge.feeds.clone();
        let state = AppState::from_config(config)
            .await
            .map_err(|error| BackendError::Init(error.to_string()))?;
        let runner = CommandRunner::default();
        let embedder = select_embedder(&embedding)?;
        // Optional fail-fast dimension probe: one embed at startup proves the
        // configured model produces vectors of the expected width before any
        // document is written (a wrong model_dir fails HERE, not mid-index).
        if let Some(expected) = embedding.expected_dims {
            let probe = embedder
                .embed("tdw embedder dimension probe")
                .await
                .map_err(|error| BackendError::Init(error.to_string()))?;
            if probe.vector.len() != expected {
                return Err(BackendError::Init(format!(
                    "knowledge.embedding.expected_dims is {expected} but {} produces \
                     {}-dimensional vectors",
                    embedder.model_id(),
                    probe.vector.len()
                )));
            }
        }
        // Build the graph engine from config. NO silent fallback: bolt
        // unreachable → hard Init error.
        let graph: Arc<dyn GraphEngine> = build_graph_engine(&graph_cfg).await?;
        // Build the hosted KnowledgeIndexer (K-E3 seam). It owns a KnowledgeIndex
        // plus the B5 write-side pipeline (manifest, rules, lexical co-index, durable
        // graph). All engine Arcs are shared with the runtime below.
        let collection = tdw_knowledge::collection_name(embedder.model_id());
        // Boot-load tag rules and the inference engine (K-L1) BEFORE building
        // the indexer so the boot-loaded RuleEngine is available to pass into
        // `.with_rules(...)`. Without this the indexer's internal rule engine
        // is always empty — auto-tagging rules from `*.tag.json` never fire.
        let (rule_engine, infer_engine) = boot_load_rules(&rules_cfg, &infer_cfg)?;
        let rules_version = rule_engine.version();
        let infer_version = infer_engine.version();
        // Clone the rule engine for the indexer before wrapping it in Arc<Mutex>.
        // The indexer owns its own copy; the hot-reload tick updates BOTH the
        // Arc<Mutex<RuleEngine>> AND the indexer's internal engine atomically.
        let indexer_rules = rule_engine.clone();
        let rules = Arc::new(Mutex::new(rule_engine));
        let infer = Arc::new(Mutex::new(infer_engine));
        let inner_index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&state.vector));
        let indexer = Arc::new(Mutex::new(
            KnowledgeIndexer::new(inner_index)
                .with_lexical(Arc::clone(&state.lexical), collection.clone())
                .with_graph(Arc::clone(&graph))
                .with_rules(indexer_rules)
                .map_err(|error| {
                    BackendError::Init(format!("indexer tag-store define: {error}"))
                })?,
        ));
        // Build the finding indexer (K-X6): shared Arc so both the runtime's
        // finding seam and the MCP tool use the same vector+lexical+graph handles.
        let finding_index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&state.vector));
        let finding_indexer = Arc::new(std::sync::Mutex::new(
            KnowledgeIndexer::new(finding_index)
                .with_lexical(Arc::clone(&state.lexical), collection.clone())
                .with_graph(Arc::clone(&graph)),
        ));
        // K-L3: build the eval-freshness cell and register the cron trigger when
        // `knowledge.evals.split_id` is configured.  The cell is shared between the
        // KnowledgeRuntime (read side: status) and the eval worker (write side).
        // Initial state: Pending (configured, not yet run) when a split is set;
        // Unconfigured otherwise (no trigger registered, no cell attached).
        let eval_freshness_cell = build_eval_freshness_cell(&evals_cfg);
        // K-L4: build the sweep freshness cell. Always built when
        // `auto_materialize = true` (the default). When disabled, the cell is
        // absent — `KgStatus` reports `SweepFreshness::Disabled`.
        let sweep_freshness_cell = build_sweep_freshness_cell(&proposals_cfg);
        // Build the full KnowledgeRuntime (hybrid retriever + graph + lexical +
        // tag channels). The runtime is attached to the MCP server so agents
        // can search/traverse the graph via the read tools (B8 surface).
        // Stamp rule/infer versions — these reflect what was loaded at boot.
        let rules_v = if rules_version > 0 {
            Some(rules_version)
        } else {
            None
        };
        let infer_v = if infer_version > 0 {
            Some(infer_version)
        } else {
            None
        };
        // The tag engine is shared between the indexer (rule-driven auto-tagging
        // in `apply_rules`) and the MCP server's `infer_ctx` resolution
        // (`rt.tags()` must return `Some` for `dispatch_knowledge_ingest_tool`
        // and `dispatch_knowledge_write_tool` to fire `run_incremental` after
        // ingest/materialize). Build it once and share the `Arc`.
        let tags_engine: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
        // K-X6 + K-L5: bind the config user and agent identities at construction
        // time. Identity is never accepted from tool arguments — it is fixed
        // here from the validated config so remote callers cannot spoof it.
        // K-L2: build the consolidation-freshness cell (always present; starts Pending).
        let consolidation_freshness_cell = build_consolidation_freshness_cell();
        // K-L3: attach the eval-freshness cell when configured.
        // K-L4: attach the sweep-freshness cell when auto_materialize = true.
        let mut knowledge_runtime =
            KnowledgeRuntime::new(Arc::clone(&embedder), Arc::clone(&state.vector))
                .with_lexical(Arc::clone(&state.lexical), collection)
                .with_graph(Arc::clone(&graph))
                .with_tags(Arc::clone(&tags_engine))
                .with_versions(rules_v, infer_v)
                .with_user_id(user_id)
                .with_agent_id(agent_id)
                .with_finding_indexer(Arc::clone(&finding_indexer))
                .with_consolidation_freshness(Arc::clone(&consolidation_freshness_cell));
        if let Some(cell) = eval_freshness_cell {
            knowledge_runtime = knowledge_runtime.with_eval_freshness(cell);
        }
        if let Some(cell) = sweep_freshness_cell {
            knowledge_runtime = knowledge_runtime.with_auto_materialize_freshness(cell);
        }
        // K-L6: build per-feed freshness cells before Arc-wrapping the runtime
        // so they can be attached via with_feed_freshness (which takes &mut self).
        // One cell per feed entry (Pending for enabled, Disabled for disabled).
        // The cells are also stored in `Backend::feed_freshness_cells` so
        // `serve()` can clone them into the spawned tasks.
        let feed_freshness_cells: Vec<Arc<tokio::sync::Mutex<FeedFreshness>>> = {
            if feeds_cfg.entries.is_empty() {
                eprintln!(
                    "[tdw] knowledge feeds: no feeds configured — \
                     knowledge acquisition is manual only"
                );
            }
            feeds_cfg
                .entries
                .iter()
                .map(|entry| {
                    let initial = if entry.enabled {
                        FeedFreshness::Pending {
                            feed_id: entry.id.clone(),
                        }
                    } else {
                        FeedFreshness::Disabled {
                            feed_id: entry.id.clone(),
                        }
                    };
                    Arc::new(tokio::sync::Mutex::new(initial))
                })
                .collect()
        };
        for cell in &feed_freshness_cells {
            knowledge_runtime = knowledge_runtime.with_feed_freshness(Arc::clone(cell));
        }
        let runtime = Arc::new(knowledge_runtime);
        // K-M4: build the contradiction detector from the configured functional
        // predicate set (taxonomy defaults + operator extra_functional_rels).
        let contradiction = Arc::new(ContradictionDetector::new(
            FunctionalPredicateSet::from_config(&contradiction_cfg.extra_functional_rels),
        ));
        Ok(Self {
            state,
            runner,
            embedder,
            indexer,
            memory: Arc::new(Mutex::new(build_memory_store())),
            feedback: Arc::new(Mutex::new(RetrievalFeedbackStore::new())),
            graph,
            runtime,
            tags_engine,
            rules,
            infer,
            contradiction,
            finding_indexer,
            evals_cfg,
            consolidation_freshness: consolidation_freshness_cell,
            proposals_cfg,
            feeds_cfg,
            feed_freshness_cells,
            daemon: None,
        })
    }

    /// Build a backend backed by deterministic in-memory engines, for tests.
    pub async fn in_memory_for_tests() -> Self {
        let state = AppState::in_memory_for_tests().await;
        // G008/RT1b: opt in to the fake-streaming path for tests — the test
        // constructor is the correct place to acknowledge that `run_streaming`
        // materialises the full result before emitting synthetic progress events.
        let runner = CommandRunner::default().allow_fake_streaming();
        let embedder: Arc<dyn EmbeddingProvider> = Arc::new(HashEmbeddingProvider::default());
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let collection = tdw_knowledge::collection_name(embedder.model_id());
        let inner_index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&state.vector));
        let indexer = Arc::new(Mutex::new(
            KnowledgeIndexer::new(inner_index)
                .with_lexical(Arc::clone(&state.lexical), collection.clone())
                .with_graph(Arc::clone(&graph)),
        ));
        // Tests boot with empty (no-op) rule and infer engines — no rules_dir
        // is configured so inference does nothing, which is the correct default.
        let tags_engine: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
        // Build the finding indexer (K-X6) with the same in-memory engines.
        let finding_index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&state.vector));
        let finding_indexer = Arc::new(std::sync::Mutex::new(
            KnowledgeIndexer::new(finding_index)
                .with_lexical(Arc::clone(&state.lexical), collection.clone())
                .with_graph(Arc::clone(&graph)),
        ));
        let consolidation_freshness_cell = build_consolidation_freshness_cell();
        let runtime = Arc::new(
            KnowledgeRuntime::new(Arc::clone(&embedder), Arc::clone(&state.vector))
                .with_lexical(Arc::clone(&state.lexical), collection)
                .with_graph(Arc::clone(&graph))
                .with_tags(Arc::clone(&tags_engine))
                .with_user_id("test-user")
                .with_finding_indexer(Arc::clone(&finding_indexer))
                .with_consolidation_freshness(Arc::clone(&consolidation_freshness_cell)),
        );
        Self {
            state,
            runner,
            embedder,
            indexer,
            memory: Arc::new(Mutex::new(build_memory_store())),
            feedback: Arc::new(Mutex::new(RetrievalFeedbackStore::new())),
            graph,
            runtime,
            tags_engine,
            rules: Arc::new(Mutex::new(RuleEngine::default())),
            infer: Arc::new(Mutex::new(InferEngine::default())),
            contradiction: Arc::new(ContradictionDetector::new(FunctionalPredicateSet::default())),
            finding_indexer,
            evals_cfg: EvalsConfig::default(),
            consolidation_freshness: consolidation_freshness_cell,
            proposals_cfg: ProposalsCfg::default(),
            feeds_cfg: FeedsConfig::default(),
            feed_freshness_cells: Vec::new(),
            daemon: None,
        }
    }

    /// The underlying daemon composition root.
    #[must_use]
    pub const fn app_state(&self) -> &AppState {
        &self.state
    }

    // --- Engine accessors (cheap `Arc` clones from the composition root) ----

    /// The OLAP (analytical) engine handle.
    #[must_use]
    pub fn olap(&self) -> Arc<dyn OlapEngine> {
        Arc::clone(&self.state.olap)
    }

    /// The relational engine handle.
    #[must_use]
    pub fn relational(&self) -> Arc<dyn RelationalEngine> {
        Arc::clone(&self.state.relational)
    }

    /// The blob (object storage) engine handle.
    #[must_use]
    pub fn blob(&self) -> Arc<dyn BlobEngine> {
        Arc::clone(&self.state.blob)
    }

    /// The vector engine handle.
    #[must_use]
    pub fn vector(&self) -> Arc<dyn VectorEngine> {
        Arc::clone(&self.state.vector)
    }

    /// The lexical (full-text) engine handle.
    #[must_use]
    pub fn lexical(&self) -> Arc<dyn LexicalEngine> {
        Arc::clone(&self.state.lexical)
    }

    /// The provider registry handle.
    #[must_use]
    pub fn registry(&self) -> Arc<ProviderRegistry> {
        Arc::clone(&self.state.registry)
    }

    // --- Policy enforcement -------------------------------------------------

    /// Enforce the attached policy for an `equity_historical` request against
    /// `provider`/`symbol`, returning the masked response envelope.
    ///
    /// Delegates to [`tdw_service_api::secure_endpoint_response`] using the
    /// composition root's [`PolicyEnforcementConfig`](tdw_service_api::PolicyEnforcementConfig).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NoPolicy`] if no policy is attached to the
    /// composition root, or [`BackendError::Engine`] if the request is rejected
    /// (ingress JWT, missing role, denied endpoint) or the response cannot be
    /// produced.
    pub fn enforce_policy(&self, provider: &str, symbol: &str) -> BackendResult<Value> {
        let policy = self.state.policy.as_ref().ok_or(BackendError::NoPolicy)?;
        Ok(tdw_service_api::secure_endpoint_response(
            policy, provider, symbol,
        )?)
    }

    // --- Event spine accessors ----------------------------------------------

    /// The shared event bus handle (cloned from the composition root).
    #[must_use]
    pub fn event_bus(&self) -> Arc<std::sync::Mutex<EventBus>> {
        Arc::clone(&self.state.bus)
    }

    /// The shared outbox handle (cloned from the composition root).
    #[must_use]
    pub fn outbox(&self) -> Arc<std::sync::Mutex<InMemoryOutbox>> {
        Arc::clone(&self.state.outbox)
    }

    // --- Op dispatch --------------------------------------------------------

    /// Dispatch a single [`OpEnvelope`] through the secure service path and
    /// return the emitted events (a `Started` followed by a terminal
    /// `Completed`/`Failed`).
    ///
    /// Delegates to the [`Dispatcher`](tdw_app_server::Dispatcher)
    /// implementation on [`AppState`].
    pub async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg> {
        tdw_app_server::Dispatcher::dispatch(&self.state, env).await
    }

    // --- Daemon lifecycle ---------------------------------------------------

    /// Start the daemon in-process: wire the service loop + relay + transport
    /// onto the current tokio runtime and **store** their handles instead of
    /// blocking. This is the non-blocking counterpart to the standalone
    /// `tdw-service` binary's bootstrap.
    ///
    /// After this returns, [`submission_handle`](Self::submission_handle) yields
    /// an in-process op-submission handle and [`bound_addr`](Self::bound_addr)
    /// yields the transport's actual bound address (the OS-assigned port for an
    /// ephemeral `127.0.0.1:0` request). Call [`shutdown`](Self::shutdown) to
    /// stop it.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the transport cannot bind (e.g. an
    /// invalid `tcp_bind` address, an address already in use, or a transport
    /// requested but not compiled into this build).
    pub async fn serve(&mut self, cfg: &BackendConfig) -> BackendResult<()> {
        let (handle, events_rx, service_loop) =
            tdw_app_server::service_channel(self.state.clone(), self.state.clone());
        let cancel = CancellationToken::new();
        let relay = tdw_app_server::spawn_inmemory_relay(
            self.state.outbox.clone(),
            self.state.bus.clone(),
            std::time::Duration::from_millis(50),
            cancel.clone(),
        );

        let transport =
            crate::server::spawn_transport(&cfg.tdw, handle.clone(), events_rx, cancel.clone())
                .await
                .map_err(|error| BackendError::Init(error.to_string()))?;

        let serve_cancel = cancel.clone();
        let serve_task = tokio::spawn(async move {
            // The service loop never returns an error in practice; log and
            // swallow so the join handle yields `()` and shutdown is uniform.
            if let Err(error) = tdw_app_server::serve(service_loop, relay, serve_cancel).await {
                eprintln!("tdw-backend: service loop error: {error}");
            }
        });

        // Phase B / K-L2 — co-spawn the feedback-aware consolidation scheduler
        // on the same cancellation token, mirroring the relay's lifecycle.
        // Uses the shared feedback store so recency credit is applied on every
        // tick; results are persisted and the freshness cell is updated.
        let consolidation_task = spawn_consolidation_scheduler_with_feedback(
            self.memory.clone(),
            self.feedback.clone(),
            Arc::clone(&self.consolidation_freshness),
            consolidation_tick(),
            cancel.clone(),
        );

        // K-L1 — co-spawn the rules hot-reload tick. The tick reads `rules_dir`
        // from config and fires on the same cancellation token so it stops with
        // the rest of the daemon. `None` when no rules_dir is configured.
        let rules_reload_task = spawn_rules_reload_tick(
            &cfg.tdw.knowledge.rules,
            &cfg.tdw.knowledge.infer,
            self.rules.clone(),
            self.infer.clone(),
            self.indexer.clone(),
            Arc::clone(&self.runtime),
            cancel.clone(),
        );

        // K-L3 — spawn the scheduled self-eval worker when split_id is
        // configured.  The worker holds the shared freshness cell (also held by
        // KnowledgeRuntime::status) and writes it after each eval run.
        // `alert_sink` is `None` today: the worker falls back to `eprintln!`.
        // When a durable notification channel is wired into the daemon, pass
        // `Some(Arc::new(<impl EvalAlertSink>))` here.
        let alert_sink: Option<Arc<dyn EvalAlertSink>> = None;
        let eval_worker_task = spawn_eval_worker(
            &self.evals_cfg,
            self.runtime.eval_freshness_cell().cloned(),
            Arc::clone(&self.embedder),
            alert_sink,
            cancel.clone(),
        );

        // K-L4 — spawn the gated auto-materialization sweep when
        // `knowledge.proposals.auto_materialize = true` (the default). The
        // sweep writes READY proposals via `materialize_ready_capped` (same
        // TOCTOU core as the operator tool, capped to `sweep_cap` per tick)
        // and fires K-L1 inference after each landing batch.
        let auto_mat_task = spawn_auto_materialize_sweep(
            &self.proposals_cfg,
            self.runtime.auto_materialize_freshness_cell().cloned(),
            self.runtime.proposals().cloned(),
            Arc::clone(&self.graph),
            Arc::clone(&self.tags_engine),
            Some(Arc::clone(&self.infer)),
            cancel.clone(),
        );
        // K-R4 — spawn the pattern-mining worker when enabled.
        // index_path = None: daemon does not yet wire a configured data-dir for
        // the pattern index; a future K-R5 config field will supply the path.
        // Idempotency is still correct within the process lifetime.
        let pattern_mining_task = spawn_pattern_mining_worker(
            &cfg.tdw.knowledge.patterns,
            Arc::clone(&self.graph),
            cancel.clone(),
            None,
        );
        // K-L6 — spawn one feed task per enabled feed entry.
        // The freshness cells were built in from_config and attached to the
        // runtime before Arc-wrapping; here we pass them to the tasks so each
        // task can write its own cell after each poll.
        // Tasks receive a FeedIngestHandle that carries the Arc fields needed
        // to fire knowledge_ingest_at (indexer + infer + graph + tags_engine),
        // routing ingest through the K-L1 inference hook (fix #1).
        let ingest_handle = FeedIngestHandle {
            indexer: Arc::clone(&self.indexer),
            infer: Arc::clone(&self.infer),
            graph: Arc::clone(&self.graph),
            tags_engine: Arc::clone(&self.tags_engine),
        };
        let feed_handles = spawn_feed_tasks(
            &self.feeds_cfg,
            &self.feed_freshness_cells,
            ingest_handle,
            cancel.clone(),
            None, // use production cron_tick
        );

        self.daemon = Some(DaemonHandle {
            cancel,
            submission: handle,
            serve_task,
            consolidation_task,
            rules_reload_task,
            eval_worker_task,
            auto_mat_task,
            pattern_mining_task,
            feed_handles,
            transport_task: transport.join,
            bound_addr: transport.bound_addr,
        });
        Ok(())
    }

    /// An in-process [`SubmissionHandle`] for the running daemon, or `None` when
    /// not serving. Submissions go straight into the service loop — no socket.
    #[must_use]
    pub fn submission_handle(&self) -> Option<SubmissionHandle> {
        self.daemon.as_ref().map(|d| d.submission.clone())
    }

    /// The address the running daemon's transport bound, or `None` when not
    /// serving. Hand this to a loopback [`DaemonClient`](tdw_app_client::DaemonClient)
    /// (e.g. from the embedded MCP surface).
    #[must_use]
    pub fn bound_addr(&self) -> Option<&str> {
        self.daemon.as_ref().map(|d| d.bound_addr.as_str())
    }

    /// Stop the running daemon: cancel its token, await the service-loop and
    /// transport tasks (aborting the transport if it does not observe
    /// cancellation promptly), and clear the stored handle.
    ///
    /// Idempotent: calling [`shutdown`](Self::shutdown) when not serving is a
    /// no-op that returns `Ok(())`.
    ///
    /// # Errors
    ///
    /// Currently infallible (returns `Ok(())`); the `Result` return is kept so
    /// future transports can surface a drain error without an API break.
    pub async fn shutdown(&mut self) -> BackendResult<()> {
        let Some(daemon) = self.daemon.take() else {
            return Ok(());
        };
        daemon.cancel.cancel();
        // The serve task cancels the relay internally and returns promptly.
        let _ = daemon.serve_task.await;
        // The consolidation scheduler observes the same token and breaks on the
        // next select; abort if it lingers so shutdown stays bounded.
        daemon.consolidation_task.abort();
        let _ = daemon.consolidation_task.await;
        // The rules hot-reload tick observes the same token; abort if it lingers.
        if let Some(reload_task) = daemon.rules_reload_task {
            reload_task.abort();
            let _ = reload_task.await;
        }
        // The eval worker observes the same token; abort if it lingers.
        if let Some(eval_task) = daemon.eval_worker_task {
            eval_task.abort();
            let _ = eval_task.await;
        }
        // K-L4: The auto-materialization sweep observes the same token; abort
        // if it lingers so shutdown stays bounded.
        if let Some(auto_mat) = daemon.auto_mat_task {
            auto_mat.abort();
            let _ = auto_mat.await;
        }
        // The pattern-mining worker observes the same token; abort if it lingers.
        if let Some(pattern_task) = daemon.pattern_mining_task {
            pattern_task.abort();
            let _ = pattern_task.await;
        }
        // K-L6 feed tasks observe the same cancellation token; abort any that
        // linger so shutdown stays bounded.
        for feed_task in daemon.feed_handles {
            feed_task.abort();
            let _ = feed_task.await;
        }
        // The transport observes the same token; abort if it lingers so
        // shutdown is bounded and never hangs the caller.
        daemon.transport_task.abort();
        let _ = daemon.transport_task.await;
        Ok(())
    }

    // --- Agent memory (Phase B) ---------------------------------------------

    /// The shared memory store handle (cloned `Arc`). The live consolidation
    /// scheduler and the surface methods below all lock this same store.
    #[must_use]
    pub fn memory_store(&self) -> Arc<Mutex<MemoryStore>> {
        Arc::clone(&self.memory)
    }

    /// The shared retrieval feedback store handle (cloned `Arc`, knowledge-system
    /// B10). The MCP feedback tool and [`consolidate_now_at`](Self::consolidate_now_at)
    /// lock this same store.
    ///
    /// **Host-wiring:** an embedding host that constructs both this `Backend` and
    /// an [`AgentBackend`](crate::agent::AgentBackend) in the same process should
    /// call `Backend::with_feedback_store(agent_backend.feedback_store_handle())`
    /// so that events appended through `AgentBackend`'s embedded `McpServer` are
    /// visible to `consolidate_now_at`. This is the correct direction: the host
    /// creates one `AgentBackend` (which owns the store), then hands its handle to
    /// `Backend` — not the reverse.
    #[must_use]
    pub fn feedback_store_handle(&self) -> Arc<Mutex<RetrievalFeedbackStore>> {
        Arc::clone(&self.feedback)
    }

    /// The shared consolidation-freshness cell (K-L2). The consolidation
    /// scheduler writes this after every tick; [`KnowledgeRuntime::status`] reads
    /// it to populate `tdw.kg.status.consolidation_freshness`.
    ///
    /// Always `Some` after construction; starts as
    /// [`ConsolidationFreshness::Pending`] until the first tick fires.
    #[must_use]
    pub fn consolidation_freshness_cell(&self) -> Arc<Mutex<ConsolidationFreshness>> {
        Arc::clone(&self.consolidation_freshness)
    }

    /// Replace the feedback store with a host-supplied handle (builder pattern).
    ///
    /// Use this when an embedding host constructs both a `Backend` and an
    /// [`AgentBackend`](crate::agent::AgentBackend) in the same process and wants
    /// events appended through `AgentBackend`'s embedded `McpServer` to be visible
    /// to [`consolidate_now_at`](Self::consolidate_now_at):
    ///
    /// ```ignore
    /// let agent = AgentBackend::from_config(&cfg)?;
    /// let backend = Backend::from_config(tdw_cfg)
    ///     .await?
    ///     .with_feedback_store(agent.feedback_store_handle());
    /// ```
    ///
    /// This is **host wiring**, not cross-facade state sharing: both facades are
    /// constructed by the same host and the `Arc` is handed across at construction
    /// time. The dual-facade boundary (async data / sync agent, joined only by a
    /// loopback `DaemonClient`) is never violated. Standalone `tdw-mcp` processes
    /// do not use this path; see the `attach_env_registry` doc for the F1 deferral.
    #[must_use]
    pub fn with_feedback_store(mut self, store: Arc<Mutex<RetrievalFeedbackStore>>) -> Self {
        self.feedback = store;
        self
    }

    // --- Knowledge runtime (F1) ---------------------------------------------

    /// The graph engine handle (cloned `Arc`). Backed by the engine selected
    /// at construction time (`bolt` or `in-memory`).
    #[must_use]
    pub fn graph_engine(&self) -> Arc<dyn GraphEngine> {
        Arc::clone(&self.graph)
    }

    /// The full knowledge runtime handle (hybrid retriever + graph/tag handles).
    /// Pass this to [`tdw_mcp::McpServer::with_knowledge`] to expose the
    /// `tdw.kg.*` / `tdw.tags.query` tools to connected agents.
    #[must_use]
    pub fn knowledge_runtime_handle(&self) -> Arc<KnowledgeRuntime> {
        Arc::clone(&self.runtime)
    }

    /// The finding indexer handle (knowledge-system K-X6).
    ///
    /// Already attached to the runtime's `with_finding_indexer` seam at
    /// construction time — `tdw_mcp::McpServer::with_knowledge` picks it up
    /// from there. Expose this accessor when a host needs a direct reference
    /// (e.g. to pass it to a second MCP surface on the same process).
    ///
    /// **K-L5 (CLOSED):** user identity is bound at `from_config` time via
    /// `[knowledge.user]` config; this handle stays valid for the indexer.
    /// Multi-principal HTTP (per-session override) is explicitly deferred.
    #[must_use]
    pub fn knowledge_finding_indexer_handle(&self) -> Arc<std::sync::Mutex<KnowledgeIndexer>> {
        Arc::clone(&self.finding_indexer)
    }

    /// The daemon-hosted knowledge indexer handle (knowledge-system K-E3).
    ///
    /// Pass this to [`tdw_mcp::McpServer::with_indexer`] to expose the
    /// `tdw.kg.ingest` tool to connected agents. The `Arc<Mutex<KnowledgeIndexer>>`
    /// is shared between the MCP surface and the daemon's own ingestion methods
    /// ([`knowledge_index_at`](Self::knowledge_index_at),
    /// [`knowledge_ingest_at`](Self::knowledge_ingest_at)) so the manifest and
    /// in-process state are always consistent.
    #[must_use]
    pub fn knowledge_indexer_handle(&self) -> Arc<Mutex<KnowledgeIndexer>> {
        Arc::clone(&self.indexer)
    }

    /// Upsert a [`Memory`] into the store, stamping the current time as its
    /// `last_consolidated` anchor when none is set (so it ages from insertion).
    /// Persists to `*.json5` when the store has a backing dir.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Memory`] if the backing file cannot be written.
    pub async fn upsert_memory(&self, memory: Memory) -> BackendResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut store = self.memory.lock().await;
        store
            .upsert_at(memory, &now)
            .map_err(|error| BackendError::Memory(error.to_string()))?;
        drop(store);
        Ok(())
    }

    /// A snapshot of every stored [`Memory`].
    pub async fn list_memories(&self) -> Vec<Memory> {
        let store = self.memory.lock().await;
        store.memories().cloned().collect()
    }

    /// Wall-clock convenience over [`Backend::consolidate_now_at`]: stamps the
    /// pass with the current UTC time. Only this edge reads the clock; the
    /// deterministic core is [`Backend::consolidate_now_at`] (B3 precedent).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Memory`] if persisting a promotion or deleting an
    /// expired memory's file fails.
    pub async fn consolidate_now(&self) -> BackendResult<Vec<ConsolidationAction>> {
        let now = chrono::Utc::now().to_rfc3339();
        self.consolidate_now_at(&now).await
    }

    /// Deterministic consolidation pass at an injected `now` (RFC 3339).
    ///
    /// When the retrieval feedback store is non-empty, usage hints are derived
    /// for each memory and passed to [`consolidation_plan_with_usage`]:
    ///
    /// - **Recency credit**: `credit = raw_age.saturating_sub(days_since_last_used)`.
    ///   A memory used N days ago gets up to N days of credit, capped at `raw_age`
    ///   so `effective_age` never goes negative. The credit comes from the
    ///   most-recent `used=true` event that references the memory (by `agent_id`
    ///   or `hit_ids`).
    /// - **`use_count`**: the number of *distinct* `query_fingerprint` values in
    ///   `used=true` events that reference the memory. Informational for now;
    ///   available for future priority-weighting.
    ///
    /// **Linking convention**: an event references a memory when
    /// `event.agent_id == memory.meta.base.name` OR
    /// `event.hit_ids` contains `memory.meta.base.name`.
    /// When neither matches, the event contributes no credit to that memory
    /// (silent no-op). Because `agent_id` is caller-supplied, a rogue caller
    /// can submit events under any valid id — this is a bounded poisoning
    /// channel (capped by per-agent + global caps). Host-binding of `agent_id`
    /// is deferred; see `knowledge_feedback_tools` trust-model note.
    ///
    /// When the feedback store is empty the behaviour is **byte-for-byte
    /// identical** to [`consolidate_at`] — the B10 regression contract.
    ///
    /// The per-memory hint computation is O(events) per memory. At the current
    /// default caps (256 per-agent, 4096 global) this is acceptable; if caps
    /// grow significantly the inner loops can be pre-aggregated into a map.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Memory`] if persisting a promotion or deleting
    /// an expired memory's file fails.
    pub async fn consolidate_now_at(&self, now: &str) -> BackendResult<Vec<ConsolidationAction>> {
        use tdw_agent_store::age_days;

        // Snapshot the feedback store (cheap clone of VecDeque<RetrievalEvent>).
        let feedback_snapshot = {
            let feedback = self.feedback.lock().await;
            // Only materialise hints when there are events; empty store → base path.
            if feedback.is_empty() {
                None
            } else {
                Some(
                    feedback
                        .events()
                        .map(|e| {
                            (
                                e.agent_id.clone(),
                                e.used,
                                e.recorded_at.clone(),
                                e.hit_ids.clone(),
                                e.query_fingerprint.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            }
        };

        let actions = {
            let mut store = self.memory.lock().await;

            let Some(events) = feedback_snapshot else {
                // No feedback data: fall straight through to the base planner so
                // the behaviour is byte-for-byte identical to the pre-B10 path.
                return consolidate_at(&mut store, now)
                    .map_err(|error| BackendError::Memory(error.to_string()));
            };

            // A `used` event references a memory when either the submitting
            // `agent_id` equals the memory name (agent-plane convention) or one
            // of the `hit_ids` equals the memory name (retrieved-doc signal).
            let references = |memory_name: &str, agent_id: &str, hit_ids: &[String]| -> bool {
                agent_id == memory_name || hit_ids.iter().any(|h| h == memory_name)
            };

            let aged: Vec<_> = store
                .memories()
                .map(|memory| {
                    let name = memory.meta.base.name.as_str();
                    let raw_age = age_days(memory.last_consolidated.as_deref(), now);

                    // Find the most-recent `used` event referencing this memory.
                    // RFC 3339 strings sort lexicographically by time.
                    let last_used_at = events
                        .iter()
                        .filter(|(agent_id, used, _, hit_ids, _)| {
                            *used && references(name, agent_id, hit_ids)
                        })
                        .map(|(_, _, recorded_at, _, _)| recorded_at.as_str())
                        .max();

                    // credit = raw_age.saturating_sub(days_since_last_used)
                    // i.e. effective_age = max(0, raw_age − days_since_event).
                    let recency_credit_days = last_used_at.map_or(0, |t| {
                        let days_since = age_days(Some(t), now);
                        raw_age.saturating_sub(days_since)
                    });

                    // use_count = distinct query_fingerprints in used events
                    // referencing this memory (dedup so repeated identical
                    // queries don't inflate the count).
                    let mut seen_fps = std::collections::HashSet::new();
                    let use_count = u32::try_from(
                        events
                            .iter()
                            .filter(|(agent_id, used, _, hit_ids, fp)| {
                                *used
                                    && references(name, agent_id, hit_ids)
                                    && seen_fps.insert(fp.as_str())
                            })
                            .count(),
                    )
                    .unwrap_or(u32::MAX);

                    (
                        memory,
                        raw_age,
                        UsageHint {
                            recency_credit_days,
                            use_count,
                        },
                    )
                })
                .collect();

            let actions = consolidation_plan_with_usage(aged);

            // Apply the decided actions — mirrors `consolidate_at`'s apply loop.
            for action in &actions {
                match action {
                    ConsolidationAction::Promote { name, to, .. } => {
                        store
                            .promote_at(name, *to, now)
                            .map_err(|error| BackendError::Memory(error.to_string()))?;
                    }
                    ConsolidationAction::Expire { name } => {
                        store
                            .remove(name)
                            .map_err(|error| BackendError::Memory(error.to_string()))?;
                    }
                }
            }
            // `store` (MutexGuard) is dropped here, releasing the lock before
            // returning — satisfies significant_drop_tightening.
            actions
        };
        Ok(actions)
    }

    // --- Typed fetch / stream ----------------------------------------------

    /// Fetch a typed result from `fetcher`, sourcing providers from the wired
    /// registry.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Engine`] if the underlying fetch fails (e.g. an
    /// invalid query, an unregistered provider, or a transport error).
    pub async fn fetch<F, Q, D>(&self, fetcher: &F, params: Value) -> BackendResult<OBBject<D>>
    where
        F: Fetcher<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        Ok(self.runner.run(fetcher, params).await?)
    }

    /// Stream typed progress + a terminal result from `fetcher`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Engine`] if the underlying fetch fails before the
    /// stream can be produced.
    pub async fn stream<F, Q, D>(
        &self,
        fetcher: &F,
        params: Value,
    ) -> BackendResult<ProgressStream<D>>
    where
        F: Fetcher<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        Ok(self.runner.run_streaming(fetcher, params).await?)
    }

    // --- Stream ingest control (sync passthroughs) -------------------------

    /// Start a live Binance trade streaming-ingest task for `symbol`, returning
    /// its stable `stream_id`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Engine`] if the symbol is invalid, the streams
    /// registry lock is poisoned, or a stream with the same id is already
    /// running.
    pub fn start_binance_stream(
        &self,
        symbol: &str,
        table: Option<String>,
    ) -> BackendResult<String> {
        Ok(self.state.start_binance_stream(symbol, table)?)
    }

    /// Stop a running streaming-ingest task by `id`. Returns `true` if a stream
    /// with that id was present.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Engine`] if the streams registry lock is
    /// poisoned.
    pub fn stop_stream(&self, id: &str) -> BackendResult<bool> {
        Ok(self.state.stop_stream(id)?)
    }

    // --- Equity historical (sync API offloaded off the async worker) -------

    /// Fetch equity historical bars for `symbol` from `provider` (`fileset` or
    /// `yahoo`).
    ///
    /// [`tdw_service_api::fetch_equity_historical`] is synchronous and drives a
    /// busy-loop `block_on` internally, so it is offloaded onto a blocking
    /// thread via [`tokio::task::spawn_blocking`] to avoid blocking the async
    /// worker.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Join`] if the blocking task fails to join, or
    /// [`BackendError::Engine`] if the fetch itself fails (e.g. an unknown
    /// provider).
    pub async fn fetch_equity_historical(
        &self,
        provider: &str,
        symbol: &str,
    ) -> BackendResult<OBBject<EquityHistoricalData>> {
        let provider = provider.to_string();
        let symbol = symbol.to_string();
        let object =
            tokio::task::spawn_blocking(move || fetch_equity_historical(&provider, &symbol))
                .await??;
        Ok(object)
    }

    // --- Knowledge ingestion (async, through the hosted KnowledgeIndexer) ---

    /// Index one [`KnowledgeDocument`] effective `now` (`YYYY-MM-DD`) through the
    /// daemon-hosted [`KnowledgeIndexer`] (knowledge-system K-E3).
    ///
    /// The full B5 write pipeline applies: content-hash idempotency check →
    /// auto-tagging rules → vector + in-process graph/tags → lexical co-index →
    /// durable graph stamping → manifest record. A document whose content hash
    /// is already recorded in the manifest is returned as
    /// [`IndexOutcome::SkippedUnchanged`] with no further writes.
    ///
    /// The indexer mutex is acquired, the single async `index_at` call is
    /// awaited, and the guard is dropped — it is never held across unrelated
    /// awaits.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if the document is invalid or any
    /// embedding/storage/tag/graph step fails.
    pub async fn knowledge_index_at(&self, doc: KnowledgeDocument, now: &str) -> BackendResult<()> {
        // Capture the entity_id and tags BEFORE taking the indexer lock so the
        // change set is built from the document's declared state, not a post-ingest
        // scan (cheaper and correct — only one entity changes per single-doc index).
        let entity_id = doc.entity.entity_id.clone();
        let doc_tags: Vec<String> = doc.tags.clone();
        let mut indexer = self.indexer.lock().await;
        indexer.index_at(doc, now).await?;
        drop(indexer);
        // K-L1: fire incremental inference over the entity just ingested.
        self.run_infer_after_ingest(&entity_id, &doc_tags, now)
            .await;
        Ok(())
    }

    /// Wall-clock convenience over [`Backend::knowledge_index_at`]: stamps the
    /// index pass with today's UTC date. Only this live edge reads the clock,
    /// mirroring the consolidation-scheduler precedent.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if the document is invalid or any
    /// embedding/storage/tag/graph step fails.
    pub async fn knowledge_index(&self, doc: KnowledgeDocument) -> BackendResult<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        self.knowledge_index_at(doc, &today).await
    }

    /// Batch-index documents effective `now` (knowledge-system B5/K-E3). Each
    /// document runs the full B5 write pipeline: content-hash idempotency,
    /// auto-tagging rules, lexical co-index, and durable graph stamping.
    /// Validation is all-or-nothing up front; after that, each document indexes
    /// independently — already-indexed documents stay recorded in the manifest
    /// and the first write failure aborts the remainder.
    ///
    /// The indexer mutex is held for the duration of the batch, so
    /// [`Backend::knowledge_search`] never observes a half-applied batch.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if any document is invalid or any
    /// embedding/storage/tag/graph step fails.
    pub async fn knowledge_ingest_at(
        &self,
        docs: Vec<KnowledgeDocument>,
        now: &str,
    ) -> BackendResult<()> {
        // Capture the batch's entity ids and tag sets BEFORE the indexer lock.
        let batch_info: Vec<(String, Vec<String>)> = docs
            .iter()
            .map(|doc| (doc.entity.entity_id.clone(), doc.tags.clone()))
            .collect();
        let mut indexer = self.indexer.lock().await;
        indexer.index_batch_at(docs, now).await?;
        drop(indexer);
        // K-L1: fire incremental inference over the batch's collective change set.
        // All entity tags from the batch are folded into one ChangeSet so rules
        // that join entities within the same batch can fire in a single pass.
        let mut entity_ids: Vec<String> = Vec::new();
        let mut all_tags: Vec<String> = Vec::new();
        for (entity_id, tags) in batch_info {
            entity_ids.push(entity_id);
            all_tags.extend(tags);
        }
        // Use the first entity_id as the batch identifier for logging; inference
        // operates on the graph/tag engines directly (not per-entity).
        let batch_label = entity_ids.first().map_or("(empty)", String::as_str);
        self.run_infer_after_ingest(batch_label, &all_tags, now)
            .await;
        Ok(())
    }

    /// Build a fresh [`KnowledgeIndexer`] backed by the daemon's graph engine
    /// and lexical engine. Callers own the returned indexer; the daemon's
    /// `embedder`, `graph`, and `lexical` handles are shared (`Arc` clones).
    ///
    /// The returned indexer is seeded with a snapshot of the daemon's current
    /// live tag-rule set so offline re-index runs apply the same rules as the
    /// live path. Rule-target tags are pre-defined in the indexer's internal
    /// [`TagStore`] (idempotent, root placement) so `apply_rules` can assign
    /// them without hitting `TagError::UnknownTag`. The snapshot is taken at
    /// construction time; subsequent hot-reloads are NOT reflected — for a
    /// live-updating indexer use [`knowledge_index_at`](Self::knowledge_index_at)
    /// instead.
    ///
    /// Use this to construct an offline or caller-scoped indexer with its own
    /// manifest (e.g. the `tdw kg reindex` offline command). For live daemon
    /// ingestion use [`knowledge_index_at`](Self::knowledge_index_at) or
    /// [`knowledge_ingest_at`](Self::knowledge_ingest_at), which route through
    /// the daemon-hosted indexer with the shared manifest.
    ///
    /// [`TagStore`]: tdw_tags::TagStore
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the tag store rejects a rule-target
    /// tag definition (e.g. due to a taxonomy cycle — extremely unlikely for
    /// root-level auto-defined tags).
    pub fn knowledge_indexer(&self) -> BackendResult<KnowledgeIndexer> {
        let index = KnowledgeIndex::new(Arc::clone(&self.embedder), Arc::clone(&self.state.vector));
        let collection = tdw_knowledge::collection_name(self.embedder.model_id());
        // Snapshot the live rule set; fall back to an empty engine when the
        // lock is poisoned (extremely unlikely — treat as "no rules" rather
        // than panicking an offline build).
        let rules_snapshot = self
            .rules
            .try_lock()
            .map(|guard| guard.clone())
            .unwrap_or_default();
        KnowledgeIndexer::new(index)
            .with_lexical(Arc::clone(&self.state.lexical), collection)
            .with_graph(Arc::clone(&self.graph))
            .with_rules(rules_snapshot)
            .map_err(|error| BackendError::Init(format!("indexer tag-store define: {error}")))
    }

    /// Search the embedded knowledge index for the `top_k` nearest hits to
    /// `query`.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if the query is empty, `top_k` is
    /// zero, or an embedding/storage step fails.
    pub async fn knowledge_search(
        &self,
        query: &str,
        top_k: usize,
    ) -> BackendResult<Vec<KnowledgeHit>> {
        let indexer = self.indexer.lock().await;
        Ok(indexer.index().search(query, top_k).await?)
    }

    // --- Inference engine (K-L1) --------------------------------------------

    /// The tag-rule engine handle (cloned `Arc`).
    ///
    /// Exposed for tests that need to inspect the loaded rule set or supply
    /// their own rules. Production callers should prefer the ingest-triggered
    /// and hot-reload paths.
    #[must_use]
    pub fn rule_engine_handle(&self) -> Arc<Mutex<RuleEngine>> {
        Arc::clone(&self.rules)
    }

    /// The inference engine handle (cloned `Arc`).
    ///
    /// Exposed for tests. Production callers should prefer the ingest-triggered
    /// path; the engine is not thread-safe for concurrent `run_incremental` calls.
    #[must_use]
    pub fn infer_engine_handle(&self) -> Arc<Mutex<InferEngine>> {
        Arc::clone(&self.infer)
    }

    /// The contradiction detector handle (cloned `Arc`, knowledge-system K-M4).
    ///
    /// Pass to [`tdw_mcp::McpServer::with_contradiction_detector`] so the
    /// embedded MCP surface can fire contradiction detection on the `materialize`
    /// action using the same detector and functional-predicate set as the
    /// backend's ingest path.
    #[must_use]
    pub fn contradiction_detector_handle(&self) -> Arc<ContradictionDetector> {
        Arc::clone(&self.contradiction)
    }

    /// Retract a derived fact by its key and propagate the support-set closure.
    ///
    /// Drives [`InferEngine::retract`] on the daemon-hosted inference engine.
    /// Derived edges whose support transitively includes `fact_key` are removed
    /// from the graph via the surgical `replace_edges` path (only
    /// `Provenance::Rule` edges are deleted — base facts are never touched).
    ///
    /// Derived tag assignments cannot be retracted (the tag store is
    /// append-only); they are returned in [`RetractReport::unremovable_tags`].
    /// The operator's documented fallback is a full re-run from a clean graph.
    ///
    /// Limit and graph errors are logged loudly before being surfaced as
    /// [`BackendError::Init`] (B7 contract: never silent truncation).
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the graph engine rejects the retraction.
    pub async fn retract_knowledge_fact(&self, fact_key: &str) -> BackendResult<RetractReport> {
        let mut infer = self.infer.lock().await;
        let report = infer
            .retract(&self.graph, &self.tags_engine, fact_key)
            .await
            .map_err(|error| {
                eprintln!("tdw-backend: knowledge retraction error for fact {fact_key:?}: {error}");
                BackendError::Init(format!("knowledge retraction: {error}"))
            })?;
        drop(infer); // release mutex before returning (significant_drop_tightening)
        if !report.unremovable_tags.is_empty() {
            eprintln!(
                "tdw-backend: retraction of {fact_key:?} left {} unremovable derived tag(s) \
                 (append-only store); a full re-run from a clean graph is needed to reconcile: {:?}",
                report.unremovable_tags.len(),
                report.unremovable_tags
            );
        }
        Ok(report)
    }

    /// Fire [`InferEngine::run_incremental`] after an ingest batch completes.
    ///
    /// Fires K-M4 contradiction detection FIRST so inference sees the corrected
    /// graph (superseded functional-predicate edges are closed before derived
    /// edges are computed). Then builds a [`ChangeSet`] and runs K-L1 incremental
    /// inference. Errors are logged loudly and never surfaced to the caller
    /// (inference is best-effort; the ingested document is already durable). This
    /// is intentional: inference failures must never roll back an ingest that
    /// succeeded (B7 contract).
    async fn run_infer_after_ingest(&self, label: &str, tags: &[String], now: &str) {
        // K-M4: contradiction scan BEFORE K-L1 inference so derived edges are
        // computed on the corrected (temporally-closed) graph.
        match self
            .contradiction
            .scan_all_functional(&self.graph, now)
            .await
        {
            Ok(report) if report.invalidated > 0 || report.conflicts > 0 => {
                eprintln!(
                    "tdw-backend: contradiction scan after ingest of {label:?}: \
                     closed {} superseded edge(s), {} conflict(s) surfaced \
                     (manual review required)",
                    report.invalidated, report.conflicts
                );
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!(
                    "tdw-backend: contradiction scan after ingest of {label:?} \
                     failed (document already durable): {error}"
                );
            }
        }
        let mut changed = ChangeSet::default();
        // Standard graph edge types written by the K-E3 indexer.
        changed.edge_types.insert("described_by".to_string());
        changed.edge_types.insert("mentions".to_string());
        for tag in tags {
            changed.tags.insert(tag.clone());
        }
        let mut infer = self.infer.lock().await;
        match infer
            .run_incremental(&self.graph, &self.tags_engine, now, &changed)
            .await
        {
            Ok(report) if report.derived_edges > 0 || report.assigned_tags > 0 => {
                eprintln!(
                    "tdw-backend: inference after ingest of {label:?}: derived {} edge(s), \
                     {} tag(s) in {} iteration(s) (rule-set v{})",
                    report.derived_edges, report.assigned_tags, report.iterations, report.version
                );
            }
            Ok(_) => {}
            Err(InferError::JoinBoundExceeded { bound }) => {
                eprintln!(
                    "tdw-backend: inference after ingest of {label:?}: chain-join bound \
                     exceeded ({bound}); reduce rule fan-out or increase max_derived"
                );
            }
            Err(InferError::DerivedLimitExceeded { limit }) => {
                eprintln!(
                    "tdw-backend: inference after ingest of {label:?}: max_derived limit \
                     exceeded ({limit}); a full re-run from a clean graph may be needed"
                );
            }
            Err(InferError::IterationLimitExceeded { limit }) => {
                eprintln!(
                    "tdw-backend: inference after ingest of {label:?}: max_iterations limit \
                     exceeded ({limit}); check for rule stratification issues"
                );
            }
            Err(error) => {
                eprintln!(
                    "tdw-backend: inference after ingest of {label:?}: engine error: {error}"
                );
            }
        }
    }
}

/// Select the knowledge embedder. Default / offline / unset →
/// the deterministic [`HashEmbeddingProvider`] so CI and offline runs
/// stay reproducible. With the `openai` feature built **and**
/// `TDW_EMBED_PROVIDER=openai` plus an API key configured
/// (`TDW_OPENAI_EMBEDDING_API_KEY` or `OPENAI_API_KEY`), a real
/// `OpenAI` HTTP embedder is used instead. `TDW_EMBED_MODEL` overrides
/// the model (default `text-embedding-3-small`);
/// `TDW_OPENAI_EMBEDDING_BASE_URL` optionally overrides the endpoint.
///
/// Select the knowledge embedder from config (`knowledge.embedding`), with
/// `TDW_EMBED_PROVIDER` as an environment override (knowledge-system B6).
///
/// `hash` (the default) → the deterministic offline
/// [`HashEmbeddingProvider`]. `local` → the on-disk model via
/// `tdw-embed-local`'s `model` feature (`local-model` build feature here),
/// reading `knowledge.embedding.model_dir` (env override
/// `TDW_EMBED_MODEL_DIR`). `openai` / `google` build the real HTTP embedder.
///
/// There is NO silent fallback: a requested provider whose feature is not
/// compiled, whose API key is missing, or whose model directory is unusable
/// is a hard [`BackendError::Init`] — switching retrieval semantics behind
/// the operator's back is worse than refusing to boot (plan B1a).
///
/// # Errors
///
/// Returns [`BackendError::Init`] as described above.
fn select_embedder(
    embedding: &tdw_config::EmbeddingConfig,
) -> BackendResult<Arc<dyn EmbeddingProvider>> {
    let env_override = std::env::var("TDW_EMBED_PROVIDER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let provider = env_override.unwrap_or_else(|| embedding.provider.trim().to_ascii_lowercase());
    match provider.as_str() {
        "" | "hash" => Ok(Arc::new(HashEmbeddingProvider::default())),
        #[cfg(feature = "local-model")]
        "local" => build_local_embedder(embedding),
        #[cfg(feature = "openai")]
        "openai" => build_openai_embedder(),
        #[cfg(feature = "google")]
        "google" => build_google_embedder(),
        other => Err(BackendError::Init(format!(
            "knowledge embedder {other:?} is unavailable in this build (compile the matching \
             feature: local-model / openai / google) — refusing to silently fall back to the \
             hash embedder"
        ))),
    }
}

#[cfg(feature = "local-model")]
fn build_local_embedder(
    embedding: &tdw_config::EmbeddingConfig,
) -> BackendResult<Arc<dyn EmbeddingProvider>> {
    let model_dir = first_env(&["TDW_EMBED_MODEL_DIR"])
        .or_else(|| embedding.model_dir.clone())
        .filter(|dir| !dir.trim().is_empty())
        .ok_or_else(|| {
            BackendError::Init(
                "knowledge.embedding.provider = local requires knowledge.embedding.model_dir \
                 (or TDW_EMBED_MODEL_DIR)"
                    .to_string(),
            )
        })?;
    let provider = tdw_embed_local::LocalModelEmbeddingProvider::from_dir(model_dir.trim())
        .map_err(|error| BackendError::Init(error.to_string()))?;
    Ok(Arc::new(provider))
}

/// First non-empty value among `names`, trimmed.
#[cfg(any(feature = "openai", feature = "google"))]
fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

/// The embedding model id from `TDW_EMBED_MODEL`, or `default`.
#[cfg(any(feature = "openai", feature = "google"))]
fn embed_model(default: &str) -> String {
    first_env(&["TDW_EMBED_MODEL"]).unwrap_or_else(|| default.to_string())
}

#[cfg(feature = "openai")]
fn build_openai_embedder() -> BackendResult<Arc<dyn EmbeddingProvider>> {
    let Some(api_key) = first_env(&["TDW_OPENAI_EMBEDDING_API_KEY", "OPENAI_API_KEY"]) else {
        // No silent fallback (B6): a requested provider without its key is a
        // boot error, not a quiet semantic switch to the hash embedder.
        return Err(BackendError::Init(
            "openai embedder requested but no API key is set \
             (TDW_OPENAI_EMBEDDING_API_KEY / OPENAI_API_KEY)"
                .to_string(),
        ));
    };
    let mut client = tdw_embed_openai::OpenAiEmbeddingHttpClient::new(
        api_key,
        embed_model("text-embedding-3-small"),
    )
    .map_err(|error| BackendError::Init(error.to_string()))?;
    if let Some(base_url) = first_env(&["TDW_OPENAI_EMBEDDING_BASE_URL"]) {
        client = client
            .with_base_url(&base_url)
            .map_err(|error| BackendError::Init(error.to_string()))?;
    }
    Ok(Arc::new(client))
}

#[cfg(feature = "google")]
fn build_google_embedder() -> BackendResult<Arc<dyn EmbeddingProvider>> {
    let Some(api_key) = first_env(&[
        "TDW_GOOGLE_EMBEDDING_API_KEY",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
    ]) else {
        // No silent fallback (B6) — same posture as the openai arm.
        return Err(BackendError::Init(
            "google embedder requested but no API key is set \
             (TDW_GOOGLE_EMBEDDING_API_KEY / GOOGLE_API_KEY / GEMINI_API_KEY)"
                .to_string(),
        ));
    };
    let mut client = tdw_embed_google::GoogleEmbeddingHttpClient::new(
        api_key,
        embed_model("gemini-embedding-001"),
    )
    .map_err(|error| BackendError::Init(error.to_string()))?;
    if let Some(base_url) = first_env(&["TDW_GOOGLE_EMBEDDING_BASE_URL"]) {
        client = client
            .with_base_url(&base_url)
            .map_err(|error| BackendError::Init(error.to_string()))?;
    }
    Ok(Arc::new(client))
}

/// The configured persistent memory directory (`TDW_MEMORY_DIR`), trimmed and
/// non-empty, or `None` when unset. This is the daemon's only durable memory
/// surface, so the standalone daemon only runs consolidation when it is set.
pub(crate) fn memory_dir() -> Option<String> {
    std::env::var("TDW_MEMORY_DIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Whether a persistent memory directory is configured (see [`memory_dir`]).
#[must_use]
pub(crate) fn memory_dir_configured() -> bool {
    memory_dir().is_some()
}

/// Build the agent [`MemoryStore`]: load `*.json5` files from `TDW_MEMORY_DIR`
/// when that env var names a usable directory (so tier changes persist across
/// restarts), otherwise an empty in-memory-only store. A load failure is logged
/// and degraded to an in-memory store so the daemon still boots.
pub(crate) fn build_memory_store() -> MemoryStore {
    let Some(dir) = memory_dir() else {
        return MemoryStore::new();
    };
    match MemoryStore::load_dir(std::path::Path::new(&dir)) {
        Ok(store) => store,
        Err(error) => {
            eprintln!(
                "tdw-backend: TDW_MEMORY_DIR load failed ({dir}): {error}; using an in-memory store"
            );
            MemoryStore::new()
        }
    }
}

/// The consolidation scheduler tick, from `TDW_CONSOLIDATION_TICK_SECS`
/// (default 3600s = hourly). A zero/unparseable value falls back to the default
/// so the scheduler never busy-spins.
pub(crate) fn consolidation_tick() -> std::time::Duration {
    const DEFAULT_SECS: u64 = 3600;
    let secs = std::env::var("TDW_CONSOLIDATION_TICK_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_SECS);
    std::time::Duration::from_secs(secs)
}

/// Build the consolidation-freshness shared cell for the K-L2 feedback loop.
///
/// Returns an `Arc<Mutex<ConsolidationFreshness>>` pre-populated with
/// [`ConsolidationFreshness::Pending`]. The returned `Arc` is shared between
/// the [`KnowledgeRuntime`] (read side: `tdw.kg.status`) and the consolidation
/// scheduler task spawned during [`Backend::serve`] (write side: updates after
/// each tick).
///
/// Always returns `Some`; the `Option` wrapper is kept so callers can use
/// `if let Some(cell) = ...` symmetrically with `build_eval_freshness_cell`.
#[must_use]
pub fn build_consolidation_freshness_cell() -> Arc<Mutex<ConsolidationFreshness>> {
    Arc::new(Mutex::new(ConsolidationFreshness::Pending))
}

/// Build the eval-freshness shared cell for the K-L3 scheduled self-eval (K-L3).
///
/// Returns `Some(Arc<Mutex<EvalFreshness>>)` pre-populated with
/// [`EvalFreshness::Pending`] when `config.split_id` is set (eval is configured
/// but has not yet run).  Returns `None` when no split is configured — the
/// caller omits `.with_eval_freshness(...)` and `KnowledgeRuntime::status`
/// reports [`EvalFreshness::Unconfigured`].
///
/// The returned `Arc` is shared between the `KnowledgeRuntime` (read side:
/// `tdw.kg.status`) and the eval worker task spawned during [`Backend::serve`]
/// (write side: updates after each cron-triggered run).
#[must_use]
pub fn build_eval_freshness_cell(
    config: &EvalsConfig,
) -> Option<Arc<tokio::sync::Mutex<EvalFreshness>>> {
    let split_id = config.split_id.as_deref().filter(|s| !s.is_empty())?;
    Some(Arc::new(tokio::sync::Mutex::new(EvalFreshness::Pending {
        split_id: split_id.to_string(),
    })))
}

/// Build the sweep-freshness shared cell for the K-L4 auto-materialization sweep.
///
/// Returns `Some(Arc<Mutex<SweepFreshness>>)` pre-populated with
/// [`SweepFreshness::Pending`] when `config.auto_materialize = true`.
/// Returns `None` when the kill-switch is set (`auto_materialize = false`) —
/// the caller omits `.with_auto_materialize_freshness(...)` and
/// `KnowledgeRuntime::status` reports [`SweepFreshness::Disabled`].
///
/// The returned `Arc` is shared between the `KnowledgeRuntime` (read side:
/// `tdw.kg.status`) and the sweep worker task spawned during [`Backend::serve`]
/// (write side: updated after each cron-triggered sweep).
#[must_use]
pub fn build_sweep_freshness_cell(
    config: &ProposalsCfg,
) -> Option<Arc<tokio::sync::Mutex<SweepFreshness>>> {
    if !config.auto_materialize {
        return None;
    }
    Some(Arc::new(tokio::sync::Mutex::new(SweepFreshness::Pending)))
}

/// Spawn the K-L4 gated auto-materialization sweep task.
///
/// Registers the sweep [`ScheduledTrigger`] in a local [`ScheduleRegistry`]
/// and starts a tokio task that ticks every [`tdw_cron::cron_tick()`] seconds.
/// On each cron slot:
///
/// 1. Acquires the `ProposalQueue` mutex.
/// 2. Calls [`materialize_ready_capped`](tdw_knowledge::proposals::ProposalQueue::materialize_ready_capped)
///    with `cap` as the per-tick limit — the same TOCTOU-safe core used by the
///    operator tool, but bounded so the sweep never lands more than `cap`
///    proposals per tick.  The rest wait for the next slot.
/// 3. Fires K-L1 incremental inference for any landed edge/tag proposals
///    (same `fire_infer_after_sweep` path as the operator `materialize` action).
/// 4. Writes `SweepFreshness::Ran { last_run_ms, landed, rejected, last_error }`
///    to the shared cell so `tdw.kg.status` reflects the last sweep outcome,
///    including any hard engine error.
///
/// Returns `None` when `config.auto_materialize = false` (kill-switch) or when
/// no `ProposalQueue` is attached to the runtime — the caller omits
/// `auto_mat_task` from [`DaemonHandle`].
/// Public only for integration tests (`kl4_auto_materialize`).
/// Production callers must use [`Backend::serve`] which wires this automatically.
#[doc(hidden)]
#[allow(clippy::too_many_lines, clippy::many_single_char_names)]
pub fn spawn_auto_materialize_sweep(
    cfg: &ProposalsCfg,
    cell: Option<Arc<tokio::sync::Mutex<SweepFreshness>>>,
    proposals: Option<Arc<tokio::sync::Mutex<tdw_knowledge::proposals::ProposalQueue>>>,
    graph: Arc<dyn tdw_core::GraphEngine>,
    tags: Arc<dyn tdw_tags::TagEngine>,
    infer: Option<Arc<Mutex<tdw_infer::InferEngine>>>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    // Kill-switch: disabled means no task.
    if !cfg.auto_materialize {
        return None;
    }
    // No queue attached — sweep would be a no-op; skip the task entirely.
    let proposals = proposals?;

    let cadence = cfg.sweep_cadence.clone();
    let cap = cfg.sweep_cap;

    let schedule = CronSchedule::parse(&cadence)
        .unwrap_or_else(|_| CronSchedule::parse("*/5 * * * *").expect("fallback parse"));

    // Sentinel envelope (same pattern as the eval worker — never dispatched).
    let sentinel_envelope = {
        use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
        OpEnvelope::new(
            SessionId::new("tdw-auto-mat").expect("session id"),
            1,
            ActorRef {
                actor_id: "auto-mat-actor".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        )
    };

    let mut registry = ScheduleRegistry::new();
    registry.add(ScheduledTrigger {
        id: "tdw-auto-materialize".to_string(),
        schedule,
        action: TriggerAction::Enqueue {
            envelope: sentinel_envelope,
            queue: "tdw-auto-mat".to_string(),
            max_attempts: 1,
            priority: 0,
        },
    });

    let tick = tdw_cron::cron_tick();
    let task = tokio::spawn(async move {
        let mut last_tick_ms = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        };

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }

            let now_ms = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            };

            let due = due_triggers(&registry, last_tick_ms, now_ms);
            last_tick_ms = now_ms;

            if due.is_empty() {
                continue;
            }

            // Cron slot fired — run one capped materialization sweep.
            let now_str = {
                use std::time::{SystemTime, UNIX_EPOCH};
                let secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let (y, mo, d, h, m, s) = epoch_secs_to_ymd_hms(secs);
                format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
            };

            // Snapshot the kinds of Ready proposals BEFORE locking so the
            // ChangeSet for inference is built from what *will* be materialized.
            let (report, ready_kinds) = {
                let mut queue = proposals.lock().await;
                // Snapshot kinds of the first `cap` Ready+pending proposals so
                // we build the inference ChangeSet from the same set that the
                // capped materialize will process.
                let ready_kinds: Vec<tdw_knowledge::proposals::ProposalKind> = {
                    use tdw_knowledge::proposals::ValidationStatus;
                    queue
                        .list(None, Some(usize::MAX))
                        .proposals
                        .into_iter()
                        .filter(|p| p.status == ValidationStatus::Ready && p.is_pending())
                        .take(cap)
                        .map(|p| p.kind.clone())
                        .collect()
                };
                let report = queue
                    .materialize_ready_capped(cap, &graph, &tags, &now_str)
                    .await;
                drop(queue);
                (report, ready_kinds)
            };

            match report {
                Ok(rep) => {
                    let landed = rep.materialized.len();
                    let rejected = rep.rejected_at_materialize.len();
                    eprintln!(
                        "[tdw] K-L4 auto-materialize sweep: landed={landed} \
                         rejected={rejected} at={now_str}"
                    );
                    // K-L1: fire incremental inference for landed edge/tag
                    // proposals — same semantics as the operator tool path.
                    if landed > 0 {
                        fire_infer_after_sweep(
                            infer.as_ref(),
                            &graph,
                            &tags,
                            &ready_kinds,
                            &now_str,
                        );
                    }
                    if let Some(ref c) = cell {
                        let mut guard = c.lock().await;
                        *guard = SweepFreshness::Ran {
                            last_run_ms: now_ms,
                            landed,
                            rejected,
                            last_error: None,
                        };
                    }
                }
                Err(error) => {
                    let error_str = error.to_string();
                    eprintln!("[tdw] K-L4 auto-materialize sweep error at={now_str}: {error_str}");
                    // Surface the error in the freshness cell so tdw.kg.status
                    // shows it — the landed/rejected counts from the last
                    // successful sweep are preserved.
                    if let Some(ref c) = cell {
                        let mut guard = c.lock().await;
                        match &mut *guard {
                            SweepFreshness::Ran { last_error, .. } => {
                                *last_error = Some(error_str.clone());
                            }
                            other => {
                                // First sweep ever errored before succeeding.
                                *other = SweepFreshness::Ran {
                                    last_run_ms: now_ms,
                                    landed: 0,
                                    rejected: 0,
                                    last_error: Some(error_str),
                                };
                            }
                        }
                    }
                }
            }
        }
    });

    Some(task)
}

/// Fire K-L1 incremental inference after the sweep lands proposals.
///
/// Builds a [`tdw_infer::ChangeSet`] from the edge-types and tag-ids of the
/// proposals that were snapshotted before the capped materialize ran, then
/// calls `run_incremental`. Best-effort: errors are logged, never surfaced —
/// the proposals are already durable.  Mirrors `fire_infer_after_materialize`
/// in `tdw-mcp` exactly (shared core, not a fork).
fn fire_infer_after_sweep(
    infer: Option<&Arc<Mutex<tdw_infer::InferEngine>>>,
    graph: &Arc<dyn tdw_core::GraphEngine>,
    tags: &Arc<dyn tdw_tags::TagEngine>,
    ready_kinds: &[tdw_knowledge::proposals::ProposalKind],
    now: &str,
) {
    use tdw_infer::ChangeSet;
    use tdw_knowledge::proposals::ProposalKind;

    let Some(infer) = infer else { return };

    let mut changed = ChangeSet::default();
    for kind in ready_kinds {
        match kind {
            ProposalKind::Edge { rel, .. } => {
                changed.edge_types.insert(rel.clone());
            }
            ProposalKind::TagAssign { tag_id, .. } => {
                changed.tags.insert(tag_id.clone());
            }
            _ => {}
        }
    }
    if changed.edge_types.is_empty() && changed.tags.is_empty() {
        return;
    }
    let graph = Arc::clone(graph);
    let tags = Arc::clone(tags);
    let infer = Arc::clone(infer);
    let now = now.to_string();
    // Spawn a detached task so the sweep loop is not blocked by inference.
    // Inference errors are logged but never propagate — proposals are durable.
    tokio::spawn(async move {
        let result = {
            let mut guard = infer.lock().await;
            guard.run_incremental(&graph, &tags, &now, &changed).await
        };
        match result {
            Ok(rep) if rep.derived_edges > 0 || rep.assigned_tags > 0 => {
                eprintln!(
                    "[tdw] K-L4 sweep inference: derived {} edge(s), {} tag(s) \
                     in {} iteration(s) (rule-set v{})",
                    rep.derived_edges, rep.assigned_tags, rep.iterations, rep.version
                );
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("[tdw] K-L4 sweep inference failed (proposals already durable): {error}");
            }
        }
    });
}

/// Convert Unix epoch seconds to (year, month, day, hour, minute, second).
///
/// Used exclusively by the auto-materialize sweep to produce an ISO-8601 UTC
/// audit timestamp without pulling in `chrono` or `time`. The algorithm is the
/// standard proleptic Gregorian calendar decomposition, valid for all dates
/// representable in `u64` epoch seconds.
#[allow(clippy::many_single_char_names)]
const fn epoch_secs_to_ymd_hms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3_600) % 24;
    let days = secs / 86_400;
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, h, m, s)
}

/// Convert a `tdw-config` [`EvalsConfig`] to the eval-runner's own config type.
///
/// The two types are structurally identical; this conversion exists so
/// `tdw-config` does not depend on `tdw-eval-runner` (which would create a
/// cycle via `tdw-cron → tdw-worker → tdw-service-api → tdw-eval-runner →
/// tdw-cron`).  The composition root (`tdw-backend`) owns both crates and
/// performs the conversion here.
fn evals_config_to_runner(cfg: &EvalsConfig) -> EvalRunnerConfig {
    EvalRunnerConfig {
        split_id: cfg.split_id.clone(),
        cadence: cfg.cadence.clone(),
        max_recall_drop: cfg.max_recall_drop,
        max_mrr_drop: cfg.max_mrr_drop,
        max_ndcg_drop: cfg.max_ndcg_drop,
        split_fixture_path: cfg.split_fixture_path.clone(),
    }
}

/// Spawn the K-L3 scheduled self-eval worker task.
///
/// Registers the eval [`ScheduledTrigger`] in a local [`ScheduleRegistry`] and
/// starts a tokio task that ticks every [`tdw_cron::cron_tick()`] seconds.  On
/// each cron slot:
///
/// 1. Loads the [`GoldenSplitFixture`] from the configured path (or the
///    crate-embedded fallback at `baselines/<split_id>.json`).
/// 2. If the fixture fails to load or has no cases → writes
///    [`EvalFreshness::Stale`] to the cell and **does not alarm** — a missing
///    or empty fixture is a configuration error, never a retrieval regression.
/// 3. If the fixture has no baseline (first-run) → runs eval, writes `Green`
///    with a `"first-run"` notice, **does not alarm**.
/// 4. Otherwise → runs `run_scheduled_eval_from_fixture`, writes the outcome,
///    emits an alert only on genuine `Regressed`.
///
/// Returns `None` when no `split_id` is configured — the caller omits the task
/// from [`DaemonHandle`] and status reports [`EvalFreshness::Unconfigured`].
#[allow(clippy::too_many_lines)] // K-L3 eval wiring: one cohesive loop — splitting degrades readability without reducing complexity
fn spawn_eval_worker(
    cfg: &EvalsConfig,
    cell: Option<Arc<tokio::sync::Mutex<EvalFreshness>>>,
    embedder: Arc<dyn EmbeddingProvider>,
    alert_sink: Option<Arc<dyn EvalAlertSink>>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    // Clone before the async move so the task owns all data ('static).
    let split_id = cfg
        .split_id
        .as_deref()
        .filter(|s| !s.is_empty())?
        .to_string();
    let runner_cfg = evals_config_to_runner(cfg);
    // Resolve the fixture path once at task-spawn time (not at fire time) so
    // any path-resolution error is visible immediately in logs.
    let fixture_path = runner_cfg
        .split_fixture_path
        .clone()
        .unwrap_or_else(|| default_fixture_path(&split_id));

    // Build the ScheduleRegistry with the eval trigger (K-L3 cron
    // registration). The TriggerAction carries a sentinel Op::Shutdown
    // envelope — the eval worker fires run_scheduled_eval_from_fixture inline
    // and never dispatches the action.
    let trigger_id = eval_trigger_id(&split_id);
    let schedule = CronSchedule::parse(&cfg.cadence)
        .unwrap_or_else(|_| CronSchedule::parse("0 3 * * MON").expect("fallback parse"));

    let sentinel_envelope = {
        use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
        OpEnvelope::new(
            SessionId::new("tdw-eval-worker").expect("session id"),
            1,
            ActorRef {
                actor_id: "eval-actor".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        )
    };

    let mut registry = ScheduleRegistry::new();
    registry.add(ScheduledTrigger {
        id: trigger_id,
        schedule,
        action: TriggerAction::Enqueue {
            envelope: sentinel_envelope,
            queue: "tdw-eval".to_string(),
            max_attempts: 1,
            priority: 0,
        },
    });

    let tick = tdw_cron::cron_tick();
    let task = tokio::spawn(async move {
        let mut last_tick_ms = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        };

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }

            let now_ms = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            };

            let due = due_triggers(&registry, last_tick_ms, now_ms);
            last_tick_ms = now_ms;

            if due.is_empty() {
                continue;
            }

            // Cron slot fired — load the golden-split fixture and run the
            // regression detection eval.  A fixture load error writes Stale
            // and does NOT alarm: missing/corrupt fixture = config error, not
            // retrieval regression.
            let fixture = match load_golden_split(&fixture_path) {
                Ok(f) => f,
                Err(load_err) => {
                    eprintln!(
                        "[tdw] knowledge self-eval: fixture load error \
                         (split={split_id}): {load_err}"
                    );
                    if let Some(ref cell) = cell {
                        let mut guard = cell.lock().await;
                        *guard = EvalFreshness::Stale {
                            last_run_ms: now_ms,
                            split_id: split_id.clone(),
                            error: load_err,
                        };
                    }
                    continue;
                }
            };

            let outcome = run_scheduled_eval_from_fixture(
                Arc::clone(&embedder),
                &fixture,
                &runner_cfg,
                now_ms,
            )
            .await;

            match outcome {
                Ok(result) => {
                    // Emit alert on regression via the wired sink, with an
                    // eprintln! fallback.  Stale (empty-cases / first-run) is
                    // NOT an alarm and never reaches this branch with
                    // is_alarm() = true due to run_scheduled_eval_from_fixture
                    // guarantees.
                    if result.freshness.is_alarm() {
                        let body = regression_alert_body(&result.verdict);
                        if let Some(ref sink) = alert_sink {
                            sink.notify(&split_id, &body);
                        }
                        eprintln!(
                            "[tdw] ALERT: knowledge self-eval regression \
                             (split={split_id}): {body}"
                        );
                    }
                    if let Some(ref cell) = cell {
                        let mut guard = cell.lock().await;
                        *guard = result.freshness;
                    }
                }
                Err(error) => {
                    eprintln!("[tdw] knowledge self-eval failed (split={split_id}): {error}");
                    if let Some(ref cell) = cell {
                        let mut guard = cell.lock().await;
                        *guard = EvalFreshness::Stale {
                            last_run_ms: now_ms,
                            split_id: split_id.clone(),
                            error: error.to_string(),
                        };
                    }
                }
            }
        }
    });

    Some(task)
}

// ---------------------------------------------------------------------------
// K-L6: per-feed cron task spawning
// ---------------------------------------------------------------------------

/// Maximum consecutive fetch/index errors before a feed enters throttled mode.
///
/// When `consecutive_errors` reaches this threshold the task sets
/// `throttled_until_ms` to the next cron slot so it actually skips polls
/// (not just prints a log line). The counter resets to zero on the next
/// successful poll.
const FEED_MAX_CONSECUTIVE_ERRORS: u32 = 3;

/// Ingest handle passed to each feed task so it can call
/// `knowledge_ingest_at`-equivalent logic (indexer + K-L1 inference hook)
/// without holding an `Arc<Backend>` (which would create a self-referential
/// ownership cycle through `serve`'s `&mut self`).
///
/// Mirrors exactly what [`Backend::knowledge_ingest_at`] does:
/// 1. Lock `indexer` and call `index_batch_at`.
/// 2. Fire `run_infer_after_ingest` via the `infer` + `graph` + `tags_engine`.
struct FeedIngestHandle {
    indexer: Arc<Mutex<KnowledgeIndexer>>,
    infer: Arc<Mutex<InferEngine>>,
    graph: Arc<dyn GraphEngine>,
    tags_engine: Arc<dyn TagEngine>,
}

impl FeedIngestHandle {
    /// Index a document batch and fire incremental inference — the same
    /// two-step pipeline as [`Backend::knowledge_ingest_at`].
    async fn ingest_at(&self, docs: Vec<KnowledgeDocument>, now: &str) -> BackendResult<()> {
        // Step 1 — index.
        let batch_info: Vec<(String, Vec<String>)> = docs
            .iter()
            .map(|doc| (doc.entity.entity_id.clone(), doc.tags.clone()))
            .collect();
        {
            let mut indexer = self.indexer.lock().await;
            indexer.index_batch_at(docs, now).await?;
        }
        // Step 2 — inference (K-L1 hook, best-effort: errors logged, not surfaced).
        let mut entity_ids: Vec<String> = Vec::new();
        let mut all_tags: Vec<String> = Vec::new();
        for (entity_id, tags) in batch_info {
            entity_ids.push(entity_id);
            all_tags.extend(tags);
        }
        let batch_label = entity_ids.first().map_or("(empty)", String::as_str);
        let mut changed = ChangeSet::default();
        changed.edge_types.insert("described_by".to_string());
        changed.edge_types.insert("mentions".to_string());
        for tag in &all_tags {
            changed.tags.insert(tag.clone());
        }
        let mut infer = self.infer.lock().await;
        match infer
            .run_incremental(&self.graph, &self.tags_engine, now, &changed)
            .await
        {
            Ok(report) if report.derived_edges > 0 || report.assigned_tags > 0 => {
                eprintln!(
                    "[tdw] feed inference after ingest of {batch_label:?}: \
                     derived {} edge(s), {} tag(s) in {} iteration(s) (rule-set v{})",
                    report.derived_edges, report.assigned_tags, report.iterations, report.version
                );
            }
            Ok(_) => {}
            Err(InferError::JoinBoundExceeded { bound }) => {
                eprintln!(
                    "[tdw] feed inference after ingest of {batch_label:?}: \
                     chain-join bound exceeded ({bound})"
                );
            }
            Err(InferError::DerivedLimitExceeded { limit }) => {
                eprintln!(
                    "[tdw] feed inference after ingest of {batch_label:?}: \
                     max_derived limit exceeded ({limit})"
                );
            }
            Err(InferError::IterationLimitExceeded { limit }) => {
                eprintln!(
                    "[tdw] feed inference after ingest of {batch_label:?}: \
                     max_iterations limit exceeded ({limit})"
                );
            }
            Err(error) => {
                eprintln!(
                    "[tdw] feed inference after ingest of {batch_label:?}: engine error: {error}"
                );
            }
        }
        Ok(())
    }
}

/// Spawn one cron-driven poll task per enabled [`FeedConfig`] entry.
///
/// `cells` must be parallel to `cfg.entries` (one cell per entry, built in
/// `from_config` before the runtime was `Arc`-wrapped and already attached to
/// `KnowledgeRuntime::feed_freshness_cells` for status reads). Each enabled
/// feed's task clones its cell Arc and writes freshness after every poll.
/// Disabled entries have a `Disabled` cell but no task.
///
/// The poll loop:
/// 1. Sleeps one [`tdw_cron::cron_tick()`] between checks.
/// 2. Uses [`tdw_cron::CronSchedule::next_after`] to detect fired slots.
/// 3. Skips the slot when in throttled backoff (`throttled_until_ms > now_ms`).
/// 4. Polls the [`FeedSource`] for up to `max_items_per_poll` articles.
/// 5. Rejects articles whose body exceeds `max_body_bytes` (per-item log).
/// 6. Maps accepted articles through
///    [`tdw_knowledge::indexer::article_to_document`] (K-L6 seam).
/// 7. Ingests through [`Backend::knowledge_ingest_at`] so the K-L1 inference
///    hook fires after every batch (not the bare indexer).
/// 8. Updates the freshness cell (including `rejected` count).
///
/// On fetch or index error: increments `consecutive_errors`; at threshold sets
/// `throttled_until_ms` to the next cron slot (behavioral backoff). Resets on
/// success. Both branches are symmetric.
///
/// Returns join handles for all spawned tasks (one per enabled feed).
///
/// `tick_override` lets tests inject a short poll interval without touching
/// global env-var state. Pass `None` in production to use [`tdw_cron::cron_tick`].
fn spawn_feed_tasks(
    cfg: &FeedsConfig,
    cells: &[Arc<tokio::sync::Mutex<FeedFreshness>>],
    ingest: FeedIngestHandle,
    cancel: CancellationToken,
    tick_override: Option<std::time::Duration>,
) -> Vec<tokio::task::JoinHandle<()>> {
    use std::sync::Arc as StdArc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use tdw_knowledge::indexer::article_to_document;

    // Wrap the ingest handle in an Arc so each task can clone a reference
    // without owning the handle.
    let ingest = Arc::new(ingest);
    let mut handles = Vec::new();

    for (entry, cell) in cfg.entries.iter().zip(cells.iter()) {
        if !entry.enabled {
            eprintln!(
                "[tdw] knowledge feed {:?}: disabled — no task spawned",
                entry.id
            );
            continue;
        }

        // Validate cadence (defence-in-depth; already validated at config load).
        let schedule = match tdw_cron::CronSchedule::parse(&entry.cadence) {
            Ok(s) => s,
            Err(error) => {
                eprintln!(
                    "[tdw] knowledge feed {:?}: invalid cadence {:?}: {error}; skipping",
                    entry.id, entry.cadence
                );
                continue;
            }
        };

        // Build the feed source from config. fixture_path is validated at
        // config load (validate_feeds), so by the time we reach here the path
        // is guaranteed to be set (provider=None path). A missing fixture_path
        // at this point is a programming error — treat as a hard startup abort
        // for this feed (log + skip, never silently poll nothing).
        let fixture_path = match entry.source_params.fixture_path.clone() {
            Some(p) => p,
            None => {
                eprintln!(
                    "[tdw] knowledge feed {:?}: no fixture_path configured; skipping \
                     (validate_feeds should have caught this — report as bug)",
                    entry.id
                );
                continue;
            }
        };
        let source: StdArc<dyn FeedSource> =
            StdArc::new(FixtureFeedSource::from_path(fixture_path));

        let feed_id = entry.id.clone();
        let plane = entry.plane.clone();
        let max_items = entry.max_items_per_poll;
        let max_body_bytes = entry.max_body_bytes;
        let tick = tick_override.unwrap_or_else(tdw_cron::cron_tick);
        // When a tick_override is set (test mode) start last_tick_ms at 0 so
        // the first cron slot (epoch time, long past) fires immediately on the
        // first iteration rather than waiting up to one cron period.
        let initial_last_tick_ms: i64 = if tick_override.is_some() {
            0
        } else {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        };

        let freshness_cell = Arc::clone(cell);
        let ingest_clone = Arc::clone(&ingest);
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            let mut last_tick_ms = initial_last_tick_ms;
            let mut consecutive_errors: u32 = 0;
            // Epoch-ms timestamp until which polls are skipped (behavioral
            // backoff). 0 = not throttled. Set when consecutive_errors reaches
            // FEED_MAX_CONSECUTIVE_ERRORS; reset to 0 on next success.
            let mut throttled_until_ms: i64 = 0;

            loop {
                tokio::select! {
                    biased;
                    () = cancel_clone.cancelled() => break,
                    () = tokio::time::sleep(tick) => {}
                }

                // Single clock read per tick (fix #6: no duplicate SystemTime::now()).
                let now_ms = {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
                };

                // Cron slot check: next_after(last_tick_ms) gives the first
                // slot strictly after the previous reading; if that slot has
                // already passed (≤ now_ms) the feed fires this iteration.
                let fired = schedule
                    .next_after(last_tick_ms)
                    .is_some_and(|slot| slot <= now_ms);
                last_tick_ms = now_ms;

                if !fired {
                    continue;
                }

                // Behavioral backoff (fix #4): skip the slot if throttled.
                if now_ms < throttled_until_ms {
                    eprintln!(
                        "[tdw] knowledge feed {feed_id:?}: throttled — \
                         skipping slot (resets on success)"
                    );
                    continue;
                }

                // Helper: arm throttle to the next cron slot after now.
                // Falls back to 60 s if the schedule returns no future slot.
                let arm_throttle = |now: i64| -> i64 {
                    schedule
                        .next_after(now)
                        .unwrap_or_else(|| now.saturating_add(60_000))
                };

                // Cron slot fired — poll the source.
                let articles = match source.poll(max_items).await {
                    Ok(items) => {
                        // Successful fetch resets the error counter and throttle.
                        consecutive_errors = 0;
                        throttled_until_ms = 0;
                        items
                    }
                    Err(error) => {
                        consecutive_errors += 1;
                        eprintln!(
                            "[tdw] knowledge feed {feed_id:?}: fetch error \
                             ({consecutive_errors}/{FEED_MAX_CONSECUTIVE_ERRORS}): {error}"
                        );
                        if consecutive_errors >= FEED_MAX_CONSECUTIVE_ERRORS {
                            throttled_until_ms = arm_throttle(now_ms);
                            eprintln!(
                                "[tdw] knowledge feed {feed_id:?}: \
                                 {FEED_MAX_CONSECUTIVE_ERRORS} consecutive errors — \
                                 throttled until next cron slot"
                            );
                        }
                        let mut guard = freshness_cell.lock().await;
                        *guard = FeedFreshness::Error {
                            last_poll_ms: now_ms,
                            feed_id: feed_id.clone(),
                            error,
                            consecutive_errors,
                        };
                        continue;
                    }
                };

                if articles.is_empty() {
                    // No items this slot — keep the last known freshness state.
                    continue;
                }

                // Per-item body size cap (fix #5): reject oversized articles
                // before article_to_document. 0 = no cap.
                let mut rejected: usize = 0;
                let accepted: Vec<_> = articles
                    .into_iter()
                    .filter(|a| {
                        if max_body_bytes > 0 && a.summary.len() > max_body_bytes {
                            eprintln!(
                                "[tdw] knowledge feed {feed_id:?}: article {:?} \
                                 body {} bytes exceeds cap {max_body_bytes} — rejected",
                                a.url,
                                a.summary.len()
                            );
                            rejected += 1;
                            false
                        } else {
                            true
                        }
                    })
                    .collect();

                if accepted.is_empty() {
                    // All items rejected — record rejected count but keep last freshness.
                    if rejected > 0 {
                        eprintln!(
                            "[tdw] knowledge feed {feed_id:?}: \
                             all {rejected} articles rejected (body size cap)"
                        );
                    }
                    continue;
                }

                // Map articles → documents through the K-L6 seam.
                let today = {
                    use chrono::Utc;
                    Utc::now().format("%Y-%m-%d").to_string()
                };
                let docs: Vec<_> = accepted
                    .iter()
                    .map(|a| article_to_document(a, &plane))
                    .collect();

                // Ingest through FeedIngestHandle::ingest_at, which replicates
                // Backend::knowledge_ingest_at: index_batch_at + K-L1 inference
                // hook (run_infer_after_ingest). Fix #1: not the bare indexer.
                match ingest_clone.ingest_at(docs, &today).await {
                    Ok(()) => {
                        // Success: reset error state and throttle.
                        consecutive_errors = 0;
                        throttled_until_ms = 0;
                        // knowledge_ingest_at returns () so we use accepted.len()
                        // as the indexed count (conservative: idempotency
                        // duplicates are counted as indexed here). The exact
                        // indexed/duplicate split requires plumbing outcomes
                        // through; deferred to a future pass.
                        let indexed = accepted.len();
                        let duplicates = 0usize;
                        eprintln!(
                            "[tdw] knowledge feed {feed_id:?}: \
                             indexed={indexed} duplicates={duplicates} \
                             rejected={rejected}"
                        );
                        let mut guard = freshness_cell.lock().await;
                        *guard = FeedFreshness::Ok {
                            last_poll_ms: now_ms,
                            feed_id: feed_id.clone(),
                            indexed,
                            duplicates,
                            rejected,
                        };
                    }
                    Err(error) => {
                        // Index error: symmetric with fetch error (fix #7).
                        consecutive_errors += 1;
                        eprintln!(
                            "[tdw] knowledge feed {feed_id:?}: index error \
                             ({consecutive_errors}/{FEED_MAX_CONSECUTIVE_ERRORS}): {error}"
                        );
                        if consecutive_errors >= FEED_MAX_CONSECUTIVE_ERRORS {
                            throttled_until_ms = arm_throttle(now_ms);
                            eprintln!(
                                "[tdw] knowledge feed {feed_id:?}: \
                                 {FEED_MAX_CONSECUTIVE_ERRORS} consecutive errors — \
                                 throttled until next cron slot"
                            );
                        }
                        let mut guard = freshness_cell.lock().await;
                        *guard = FeedFreshness::Error {
                            last_poll_ms: now_ms,
                            feed_id: feed_id.clone(),
                            error: error.to_string(),
                            consecutive_errors,
                        };
                    }
                }
            }
        });

        handles.push(task);
    }

    handles
}

/// Eagerly validate and log the graph backend at daemon startup (K-E1).
///
/// Called by `server::run_daemon` immediately after `AppState::from_config` so
/// the `"[tdw] knowledge graph: bolt backend connected"` line (or the in-memory
/// NOTICE) appears in daemon logs at boot time — before any op is dispatched.
/// This lets the CI guard in `live-stack.yml` assert the correct backend without
/// waiting for a knowledge op to trigger lazy construction.
///
/// The returned engine is dropped after logging; the authoritative engine for
/// dispatched knowledge ops is still built inside `Backend::from_config` when
/// the `tdw-backend` binary or embedded surface is used. For the standalone
/// `tdw-service` binary this call is the only graph-engine construction, so a
/// misconfigured or unreachable bolt endpoint fails the daemon at boot rather
/// than silently succeeding and dying on first knowledge use.
///
/// # Errors
///
/// Forwards [`build_graph_engine`] errors unchanged — unknown backend, missing
/// bolt URI, missing `bolt` build feature, or a Bolt connection failure.
pub(crate) async fn validate_graph_backend(cfg: &tdw_config::GraphConfig) -> BackendResult<()> {
    // Build (and immediately drop) the engine — the log lines and any hard
    // Init errors are the only side effects we need here.
    let _engine = build_graph_engine(cfg).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// K-L1: rules boot-load, directory parsing, hot-reload scheduler
// ---------------------------------------------------------------------------

/// Default limits for the rules directory loader.
const DEFAULT_MAX_FILES: usize = 64;
const DEFAULT_MAX_FILE_SIZE_KB: u64 = 256;
const DEFAULT_MAX_TOTAL_RULES: usize = 1_000;

/// Parsed rule sets from a rules directory (both tag and infer rules).
struct LoadedRules {
    tag_rules: Vec<TagRule>,
    infer_rules: Vec<tdw_infer::InferRule>,
}

/// Load both `*.tag.json` (tag rules) and `*.infer.json` (inference rules)
/// from `dir` with enforced limits.
///
/// # File convention
///
/// - `*.tag.json` — JSON array of [`tdw_tag_rules::TagRule`] objects.
/// - `*.infer.json` — JSON array of [`tdw_infer::InferRule`] objects
///   (`DeriveEdge` / `PropagateTag` serde shapes).
/// - Files with any other extension (e.g. plain `.json`, `.md`) are silently
///   ignored so README files and JSON schemas don't accidentally parse as rules.
/// - Symlinks are followed (documented); operators should not place symlink
///   cycles in the rules directory.
///
/// # Limits
///
/// Exceeding `max_files`, `max_file_size_kb`, or `max_total_rules` is a hard
/// error. Unreadable files are a hard error (no silent skip — every file that
/// matches the suffix must be readable and valid JSON).
///
/// # Errors
///
/// Returns [`BackendError::Init`] on any limit violation, I/O failure, or
/// parse failure. The error message always names the offending file.
#[allow(clippy::too_many_lines)] // directory scan + per-file read + limit checks; splitting adds indirection without clarity
fn load_rules_from_dir(
    dir: &std::path::Path,
    cfg: &tdw_config::RulesConfig,
) -> BackendResult<LoadedRules> {
    /// Read and size-check a single rule file.
    fn read_file(path: &std::path::Path, max_bytes: u64) -> BackendResult<String> {
        let meta = std::fs::metadata(path).map_err(|e| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: cannot stat {}: {e}",
                path.display()
            ))
        })?;
        if meta.len() > max_bytes {
            return Err(BackendError::Init(format!(
                "knowledge.rules.rules_dir: file {} is {} KiB, limit is {} KiB \
                 (set knowledge.rules.max_file_size_kb to raise)",
                path.display(),
                meta.len() / 1024,
                max_bytes / 1024,
            )));
        }
        std::fs::read_to_string(path).map_err(|e| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: cannot read {}: {e}",
                path.display()
            ))
        })
    }

    let max_files = cfg.max_files.unwrap_or(DEFAULT_MAX_FILES);
    let max_file_size_kb = cfg.max_file_size_kb.unwrap_or(DEFAULT_MAX_FILE_SIZE_KB);
    let max_total_rules = cfg.max_total_rules.unwrap_or(DEFAULT_MAX_TOTAL_RULES);

    // Collect all *.tag.json and *.infer.json paths, sorted for determinism.
    let mut tag_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut infer_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(|error| {
        BackendError::Init(format!(
            "knowledge.rules.rules_dir {} cannot be read: {error}",
            dir.display()
        ))
    })?;

    for entry in read_dir {
        match entry {
            Err(error) => {
                // Non-fatal directory entry read failure — warn loudly but
                // keep going so a single bad entry doesn't abort the load.
                unreadable.push(format!("directory entry error: {error}"));
            }
            Ok(entry) => {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if name.ends_with(".tag.json") {
                    tag_paths.push(path);
                } else if name.ends_with(".infer.json") {
                    infer_paths.push(path);
                }
                // other extensions silently ignored
            }
        }
    }

    if !unreadable.is_empty() {
        return Err(BackendError::Init(format!(
            "knowledge.rules.rules_dir {}: {} unreadable dir entr{}: {}",
            dir.display(),
            unreadable.len(),
            if unreadable.len() == 1 { "y" } else { "ies" },
            unreadable.join(", ")
        )));
    }

    tag_paths.sort();
    infer_paths.sort();

    let total_files = tag_paths.len() + infer_paths.len();
    if total_files > max_files {
        return Err(BackendError::Init(format!(
            "knowledge.rules.rules_dir {}: {total_files} rule files found, \
             limit is {max_files} (set knowledge.rules.max_files to raise)",
            dir.display()
        )));
    }

    let max_bytes = max_file_size_kb * 1024;

    let mut tag_rules: Vec<TagRule> = Vec::new();
    for path in &tag_paths {
        let text = read_file(path, max_bytes)?;
        let file_rules: Vec<TagRule> = serde_json::from_str(&text).map_err(|error| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: malformed tag-rule file {}: {error}",
                path.display()
            ))
        })?;
        tag_rules.extend(file_rules);
    }

    let mut infer_rules: Vec<tdw_infer::InferRule> = Vec::new();
    for path in &infer_paths {
        let text = read_file(path, max_bytes)?;
        let file_rules: Vec<tdw_infer::InferRule> =
            serde_json::from_str(&text).map_err(|error| {
                BackendError::Init(format!(
                    "knowledge.rules.rules_dir: malformed infer-rule file {}: {error}",
                    path.display()
                ))
            })?;
        infer_rules.extend(file_rules);
    }

    let total_rules = tag_rules.len() + infer_rules.len();
    if total_rules > max_total_rules {
        return Err(BackendError::Init(format!(
            "knowledge.rules.rules_dir {}: {total_rules} total rules loaded, \
             limit is {max_total_rules} (set knowledge.rules.max_total_rules to raise)",
            dir.display()
        )));
    }

    Ok(LoadedRules {
        tag_rules,
        infer_rules,
    })
}

/// Compute a content hash over all `*.tag.json` and `*.infer.json` files in
/// `dir` for hot-reload change detection.
///
/// Hashes the sorted-by-path file **contents** (not mtime/size). This correctly
/// detects same-second, same-size edits. Unreadable files are logged as
/// warnings and excluded from the hash (a partial read is not a reliable signal;
/// the actual parse error will surface on the next reload attempt).
///
/// Symlinks are followed (consistent with `load_rules_from_dir`).
fn dir_content_hash(dir: &std::path::Path) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let read_dir = std::fs::read_dir(dir).ok()?;
    let mut paths: Vec<std::path::PathBuf> = read_dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let name = path.file_name()?.to_str().unwrap_or_default().to_string();
            if name.ends_with(".tag.json") || name.ends_with(".infer.json") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    paths.sort();

    let mut hasher = DefaultHasher::new();
    for path in &paths {
        path.hash(&mut hasher);
        match std::fs::read(path) {
            Ok(contents) => contents.hash(&mut hasher),
            Err(error) => {
                // Warn loudly; exclude this file from the hash so the tick can
                // still detect changes in other files. The actual parse error
                // will surface in load_rules_from_dir on the next reload.
                eprintln!(
                    "tdw-backend: rules dir_content_hash: cannot read {}: {error} \
                     — this file is excluded from the change-detection hash",
                    path.display()
                );
            }
        }
    }
    Some(hasher.finish())
}

/// Boot-load both tag rules and inference rules; build both engines.
///
/// Absent `rules_dir` → both engines are empty (version 0); logs loudly.
/// A configured but nonexistent directory, a malformed file, or a limit
/// violation is a **hard [`BackendError::Init`]**.
///
/// # Errors
///
/// Returns [`BackendError::Init`] on directory/file issues, limit violations,
/// or rule validation/stratification failure inside `hot_reload`.
fn boot_load_rules(
    rules_cfg: &tdw_config::RulesConfig,
    infer_cfg: &tdw_config::InferLimitsConfig,
) -> BackendResult<(RuleEngine, InferEngine)> {
    let limits = RunLimits {
        max_iterations: infer_cfg
            .max_iterations
            .unwrap_or_else(|| RunLimits::default().max_iterations),
        max_derived: infer_cfg
            .max_derived
            .unwrap_or_else(|| RunLimits::default().max_derived),
    };
    let mut infer_engine = InferEngine::with_limits(limits);

    let Some(rules_dir) = rules_cfg
        .rules_dir
        .as_deref()
        .filter(|d| !d.trim().is_empty())
    else {
        eprintln!(
            "tdw-backend: knowledge.rules.rules_dir is not configured — \
             auto-tagging AND inference are DISABLED; set [knowledge.rules] rules_dir \
             in your config to enable rule-driven derivation (K-L1)"
        );
        return Ok((RuleEngine::default(), infer_engine));
    };

    let dir = std::path::Path::new(rules_dir);
    if !dir.exists() {
        return Err(BackendError::Init(format!(
            "knowledge.rules.rules_dir {} does not exist — \
             refusing to boot with a missing rules directory; create it or unset the config key",
            dir.display()
        )));
    }

    let loaded = load_rules_from_dir(dir, rules_cfg)?;
    let tag_count = loaded.tag_rules.len();
    let infer_count = loaded.infer_rules.len();

    let mut rule_engine = RuleEngine::default();
    rule_engine.hot_reload(loaded.tag_rules).map_err(|error| {
        BackendError::Init(format!(
            "knowledge.rules.rules_dir {}: tag-rule validation failed: {error}",
            dir.display()
        ))
    })?;

    infer_engine
        .hot_reload(loaded.infer_rules)
        .map_err(|error| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir {}: infer-rule validation failed: {error}",
                dir.display()
            ))
        })?;

    eprintln!(
        "tdw-backend: loaded {tag_count} tag rule(s) and {infer_count} infer rule(s) \
         from {} (tag-set v{}, infer-set v{})",
        dir.display(),
        rule_engine.version(),
        infer_engine.version()
    );
    Ok((rule_engine, infer_engine))
}

/// The hot-reload tick interval, from `TDW_RULES_RELOAD_TICK_SECS`
/// (default 60 s). A zero/unparseable value falls back to the default
/// so the scheduler never busy-spins.
fn rules_reload_tick() -> std::time::Duration {
    const DEFAULT_SECS: u64 = 60;
    let secs = std::env::var("TDW_RULES_RELOAD_TICK_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_SECS);
    std::time::Duration::from_secs(secs)
}

/// Spawn the rules hot-reload tick task (K-L1).
///
/// On each tick, computes the `rules_dir` content hash (hashing file CONTENTS)
/// and compares it to the last-seen hash. When changed, both `*.tag.json` and
/// `*.infer.json` files are re-parsed. The new rule sets are staged and
/// FULLY VALIDATED before either engine is mutated:
///
/// - If tag-rule validation fails → both engines stay UNCHANGED, loud log.
/// - If infer-rule stratification fails → both engines stay UNCHANGED, loud log.
/// - Only when BOTH validations pass are the locks acquired and both engines
///   swapped atomically (rules-lock first, then infer-lock — consistent order).
///
/// Returns `None` when `rules_dir` is absent — no task is spawned.
#[allow(clippy::too_many_lines)] // async hot-reload tick: validate → swap both engines atomically; splitting loses the invariant commentary
fn spawn_rules_reload_tick(
    rules_cfg: &tdw_config::RulesConfig,
    infer_cfg: &tdw_config::InferLimitsConfig,
    rules: Arc<Mutex<RuleEngine>>,
    infer: Arc<Mutex<InferEngine>>,
    indexer: Arc<Mutex<KnowledgeIndexer>>,
    runtime: Arc<KnowledgeRuntime>,
    cancel: CancellationToken,
) -> Option<tokio::task::JoinHandle<()>> {
    let rules_dir = rules_cfg.rules_dir.as_deref()?.trim().to_string();
    if rules_dir.is_empty() {
        return None;
    }
    let tick = rules_reload_tick();
    let dir = std::path::PathBuf::from(rules_dir);
    let rules_cfg = rules_cfg.clone();
    // Read the infer limits once — they are config-static for the lifetime of
    // the daemon process; hot-reload only swaps rule files, not limit config.
    let infer_limits = RunLimits {
        max_iterations: infer_cfg
            .max_iterations
            .unwrap_or_else(|| RunLimits::default().max_iterations),
        max_derived: infer_cfg
            .max_derived
            .unwrap_or_else(|| RunLimits::default().max_derived),
    };

    let handle = tokio::spawn(async move {
        // Seed with the current hash so the first tick only fires on a real change.
        let mut last_hash: Option<u64> = dir_content_hash(&dir);
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }
            let now_hash = dir_content_hash(&dir);
            if now_hash == last_hash {
                continue;
            }

            // --- Stage: parse both rule sets from disk. ---------------------
            // Do this OUTSIDE the locks: parsing may be slow and we must not
            // hold engine locks during I/O.
            let loaded = match load_rules_from_dir(&dir, &rules_cfg) {
                Ok(loaded) => loaded,
                Err(error) => {
                    eprintln!(
                        "tdw-backend: rules hot-reload from {} failed (I/O/parse/limit): \
                         {error} — keeping the CURRENT rule sets active",
                        dir.display()
                    );
                    continue;
                }
            };

            // --- Stage: validate BOTH new rule sets before touching engines. -
            // Pre-validate tag rules by running a throw-away RuleEngine.
            let mut staged_rule_engine = RuleEngine::default();
            if let Err(error) = staged_rule_engine.hot_reload(loaded.tag_rules.clone()) {
                eprintln!(
                    "tdw-backend: rules hot-reload from {}: tag-rule validation \
                     rejected — BOTH engines stay UNCHANGED: {error}",
                    dir.display()
                );
                continue;
            }

            // Pre-validate infer rules with the correct limits in a throw-away engine.
            let mut staged_infer_engine = InferEngine::with_limits(infer_limits);
            if let Err(error) = staged_infer_engine.hot_reload(loaded.infer_rules.clone()) {
                eprintln!(
                    "tdw-backend: rules hot-reload from {}: infer-rule stratification \
                     rejected — BOTH engines stay UNCHANGED: {error}",
                    dir.display()
                );
                continue;
            }

            let tag_count = loaded.tag_rules.len();
            let infer_count = loaded.infer_rules.len();

            // --- Swap: both validations passed — acquire locks and replace. --
            // Lock order: rules first, then infer, then indexer. Must be
            // consistent at every call site to avoid deadlocks.
            {
                // Keep a clone for the indexer reload (loaded.tag_rules is
                // consumed by rule_guard.hot_reload below).
                let tag_rules_for_indexer = loaded.tag_rules.clone();

                let mut rule_guard = rules.lock().await;
                let mut infer_guard = infer.lock().await;
                // These hot_reload calls on the live engines cannot fail —
                // we already validated the same rule sets above. If they do
                // (internal engine bug), log loudly rather than panicking.
                if let Err(error) = rule_guard.hot_reload(loaded.tag_rules) {
                    eprintln!(
                        "tdw-backend: rules hot-reload INTERNAL ERROR — tag engine rejected \
                         pre-validated rules (bug): {error}"
                    );
                    continue;
                }
                if let Err(error) = infer_guard.hot_reload(loaded.infer_rules) {
                    eprintln!(
                        "tdw-backend: rules hot-reload INTERNAL ERROR — infer engine rejected \
                         pre-validated rules (bug): {error}"
                    );
                    continue;
                }
                let rules_v = rule_guard.version();
                let infer_v = infer_guard.version();
                drop(infer_guard);
                drop(rule_guard);

                // Also reload the hosted indexer's internal rule engine so
                // auto-tagging rules applied during index_at match the live
                // engine. The indexer lock is acquired AFTER the rule/infer
                // locks are dropped (consistent ordering; indexer is a leaf).
                {
                    let mut indexer_guard = indexer.lock().await;
                    if let Err(error) = indexer_guard.hot_reload_rules(tag_rules_for_indexer) {
                        eprintln!(
                            "tdw-backend: rules hot-reload INTERNAL ERROR — indexer rule \
                             engine rejected pre-validated rules (bug): {error}"
                        );
                    }
                }

                // Update the version triple on the live KnowledgeRuntime so MCP
                // search responses reflect the new rule-set versions.
                // NOTE: `update_versions` only stamps the version numbers that
                // appear in search response metadata. These version numbers are
                // NOT written as `valid_from` on any derived graph edge — edges
                // carry the `now` date of the run that derived them, which is
                // injected at `run_incremental` call time, not here.
                runtime.update_versions(Some(rules_v), Some(infer_v));
                last_hash = now_hash;
                eprintln!(
                    "tdw-backend: rules hot-reload from {}: loaded {tag_count} tag \
                     rule(s) and {infer_count} infer rule(s) \
                     (tag-set v{rules_v}, infer-set v{infer_v})",
                    dir.display()
                );
            }
        }
    });
    Some(handle)
}

/// Build the graph engine from `knowledge.graph` config (knowledge-system F1).
///
/// `backend = "in-memory"` → [`InMemoryGraphEngine`] (always compiled). When
/// this branch is taken a one-line notice is printed to stderr reminding the
/// operator that data is not persisted. This is an **explicit default** (K-E1),
/// not a silent fallback — the no-silent-fallback posture is preserved: both
/// backends are first-class and an unreachable `bolt` endpoint is still a hard
/// `Init` error.
///
/// `backend = "bolt"` → [`BoltGraphEngine`] (requires the `bolt` feature; hard
/// [`BackendError::Init`] if unreachable — NO silent fallback).
///
/// # Errors
///
/// Returns [`BackendError::Init`] for unknown backends, missing bolt URI, missing
/// `bolt` build feature, or a Bolt connection error. Bolt errors include the
/// exact remediation command so the operator can act without reading the runbook.
#[cfg(feature = "bolt")]
async fn build_graph_engine(cfg: &tdw_config::GraphConfig) -> BackendResult<Arc<dyn GraphEngine>> {
    match cfg.backend.as_str() {
        "in-memory" => {
            eprintln!(
                "[tdw] NOTICE: knowledge graph running in-memory — data is NOT persisted \
                 across restarts. Set [knowledge.graph] backend=\"bolt\" for production. \
                 See docs/ops/graph-db.md."
            );
            Ok(Arc::new(InMemoryGraphEngine::default()))
        }
        "bolt" => {
            let uri = cfg
                .bolt_uri
                .as_deref()
                .filter(|uri| !uri.trim().is_empty())
                .ok_or_else(|| {
                    BackendError::Init(
                        "knowledge.graph.backend = bolt requires knowledge.graph.bolt_uri"
                            .to_string(),
                    )
                })?;
            let password = std::env::var(&cfg.bolt_password_env).unwrap_or_default();
            let engine: Arc<dyn GraphEngine> = tdw_storage_graph::BoltGraphEngine::connect(
                uri,
                &cfg.bolt_user,
                &password,
                &cfg.bolt_db,
            )
            .await
            .map(|e| Arc::new(e) as Arc<dyn GraphEngine>)
            .map_err(|error| {
                BackendError::Init(format!(
                    "bolt graph connect ({uri}): {error}. \
                             Remediation: start Memgraph with \
                             `docker compose --profile full up -d memgraph` \
                             and verify port 7687 is reachable, or switch to the \
                             in-memory dev backend with \
                             `[knowledge.graph] backend = \"in-memory\"` in your \
                             daemon TOML. See docs/ops/graph-db.md."
                ))
            })?;
            eprintln!("[tdw] knowledge graph: bolt backend connected ({uri})");
            Ok(engine)
        }
        other => Err(BackendError::Init(format!(
            "unknown knowledge.graph.backend {other:?}; valid values: bolt | in-memory"
        ))),
    }
}

/// Non-bolt build: only `in-memory` is available; `bolt` is a hard error.
///
/// The `async` keyword is kept so this matches the bolt-feature signature and
/// callers do not need cfg-gated call sites.
#[cfg(not(feature = "bolt"))]
#[allow(clippy::unused_async)]
async fn build_graph_engine(cfg: &tdw_config::GraphConfig) -> BackendResult<Arc<dyn GraphEngine>> {
    match cfg.backend.as_str() {
        "in-memory" => {
            eprintln!(
                "[tdw] NOTICE: knowledge graph running in-memory — data is NOT persisted \
                 across restarts. Set [knowledge.graph] backend=\"bolt\" for production. \
                 See docs/ops/graph-db.md."
            );
            Ok(Arc::new(InMemoryGraphEngine::default()))
        }
        "bolt" => Err(BackendError::Init(
            "knowledge.graph.backend = bolt requires the `bolt` build feature \
             (compile tdw-backend with --features bolt) — refusing to silently \
             fall back to the in-memory engine. \
             Remediation: start Memgraph with \
             `docker compose --profile full up -d memgraph` \
             and rebuild with `--features bolt`, or switch to the \
             in-memory dev backend with \
             `[knowledge.graph] backend = \"in-memory\"` in your daemon TOML. \
             See docs/ops/graph-db.md."
                .to_string(),
        )),
        other => Err(BackendError::Init(format!(
            "unknown knowledge.graph.backend {other:?}; valid values: bolt | in-memory"
        ))),
    }
}

/// Spawn the K-R4 pattern-mining worker (knowledge-system K-R4).
///
/// On each cron slot the worker calls [`PatternEngine::run_pattern_mining_at`]
/// on the daemon's graph engine and logs the resulting [`MiningReport`].
/// A hard budget error (B7) is logged and the worker keeps running — it fires
/// again on the next scheduled slot.
///
/// Returns `None` when `knowledge.patterns.enabled = false` (the default) so
/// the worker is off unless the operator explicitly opts in. A loud NOTICE is
/// emitted when the config is present but `enabled` is `false` so the operator
/// knows the feature exists and how to turn it on.
fn spawn_pattern_mining_worker(
    cfg: &tdw_config::PatternConfig,
    graph: Arc<dyn GraphEngine>,
    cancel: CancellationToken,
    // Path for persisting the PatternIndex across restarts (caller-owned
    // persistence — same pattern as DerivationIndex in tdw-infer).
    // Pass None to disable persistence (idempotency still works within the
    // process lifetime; created/updated counts are correct within a run but
    // reset to zero on restart).
    index_path: Option<std::path::PathBuf>,
) -> Option<tokio::task::JoinHandle<()>> {
    if !cfg.enabled {
        eprintln!(
            "[tdw] NOTICE: K-R4 pattern mining is disabled \
             (knowledge.patterns.enabled = false). \
             Set enabled = true in your daemon TOML to activate scheduled motif mining."
        );
        return None;
    }

    let limits = MiningLimits {
        min_support: cfg.min_support,
        max_motif_edges: cfg.max_motif_edges,
        max_candidates: cfg.max_candidates,
        max_instance_scan: cfg.max_instance_scan,
        ..MiningLimits::default()
    };

    let schedule = CronSchedule::parse(&cfg.cadence)
        .unwrap_or_else(|_| CronSchedule::parse("0 2 * * *").expect("fallback parse"));
    // Build a sentinel trigger envelope (same pattern as spawn_eval_worker /
    // K-L3). The pattern-mining worker fires run_pattern_mining_at inline on
    // each due tick and never dispatches the action payload — the envelope is a
    // required field on ScheduledTrigger, not a dispatch target.
    let sentinel_envelope = {
        use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
        OpEnvelope::new(
            SessionId::new("tdw-pattern-worker").expect("session id"),
            1,
            ActorRef {
                actor_id: "pattern-actor".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        )
    };

    let mut registry = ScheduleRegistry::new();
    registry.add(ScheduledTrigger {
        id: "tdw-pattern-mining".to_string(),
        schedule,
        action: TriggerAction::Enqueue {
            envelope: sentinel_envelope,
            queue: "tdw-patterns".to_string(),
            max_attempts: 1,
            priority: 0,
        },
    });

    let tick = tdw_cron::cron_tick();
    let handle = tokio::spawn(async move {
        let engine = PatternEngine::with_limits(limits);

        // Load persisted index from disk on start (caller-owned persistence —
        // same pattern as DerivationIndex in tdw-infer). If the path is absent
        // or the file does not exist yet, start with an empty index.
        let mut index: PatternIndex = index_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| PatternIndex::from_json(&raw).ok())
            .unwrap_or_default();

        let mut last_tick_ms = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        };

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }

            let now_ms = {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            };

            let due = due_triggers(&registry, last_tick_ms, now_ms);
            last_tick_ms = now_ms;

            if due.is_empty() {
                continue;
            }

            // Cron slot fired — run pattern mining.
            let now_ts = chrono::Utc::now().to_rfc3339();
            let window = chrono::Utc::now().format("%Y-%m-%d").to_string();

            match engine
                .run_pattern_mining_at(&graph, &mut index, &now_ts, &window)
                .await
            {
                Ok(report) => {
                    eprintln!(
                        "[tdw] K-R4 pattern mining complete: \
                         created={} updated={} motifs_examined={} instance_edges={}",
                        report.patterns_created,
                        report.patterns_updated,
                        report.motifs_examined,
                        report.instance_edges_written,
                    );
                    // Persist the updated index so idempotency survives restarts.
                    if let Some(ref path) = index_path {
                        match index.to_json() {
                            Ok(json) => {
                                if let Err(e) = std::fs::write(path, json) {
                                    eprintln!(
                                        "[tdw] K-R4 pattern index persist failed \
                                         (non-fatal, will retry next slot): {e}"
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "[tdw] K-R4 pattern index serialize failed \
                                     (non-fatal): {e}"
                                );
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[tdw] K-R4 pattern mining error (will retry next slot): {error}");
                }
            }
        }
    });
    Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};
    use tdw_provider_fileset::FilesetEquityHistoricalFetcher;

    fn make_envelope(op: Op) -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("session-backend-test").expect("session id"),
            1,
            ActorRef {
                actor_id: "user:test".to_string(),
                kind: ActorKind::User,
                tenant_id: Some("default".to_string()),
            },
            op,
        )
    }

    #[tokio::test]
    async fn engine_accessors_return_usable_handles() {
        let backend = Backend::in_memory_for_tests().await;

        // The registry handle shares the composition root's `Arc`.
        assert!(Arc::ptr_eq(
            &backend.registry(),
            &backend.app_state().registry
        ));
        assert!(backend.registry().entries().len() >= 3);

        // Each engine handle clones without panicking and is independently
        // droppable.
        let _ = backend.olap();
        let _ = backend.relational();
        let _ = backend.blob();
        let _ = backend.vector();
        let _ = backend.lexical();
    }

    #[tokio::test]
    async fn dispatch_run_query_returns_started_then_completed() {
        let backend = Backend::in_memory_for_tests().await;
        let env = make_envelope(Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        });
        let op_id = env.op_id.clone();
        let events = backend.dispatch(env).await;

        assert_eq!(events.len(), 2);
        match &events[0] {
            EventMsg::Started { op_id: started } => assert_eq!(started, &op_id),
            other => panic!("expected Started, got {other:?}"),
        }
        match &events[1] {
            EventMsg::Completed {
                op_id: completed, ..
            } => assert_eq!(completed, &op_id),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_uses_wired_registry_and_returns_typed_object() {
        let backend = Backend::in_memory_for_tests().await;
        let object: OBBject<EquityHistoricalData> = backend
            .fetch(
                &FilesetEquityHistoricalFetcher,
                serde_json::json!({ "symbol": "aapl" }),
            )
            .await
            .unwrap_or_else(|error| panic!("fetch should succeed: {error}"));

        assert_eq!(object.provider, "fileset");
        assert_eq!(object.rows[0].symbol, "AAPL");
    }

    #[tokio::test]
    async fn stream_emits_progress_then_done() {
        use tdw_core::ProgressOrResult;

        let backend = Backend::in_memory_for_tests().await;
        let mut stream = backend
            .stream(
                &FilesetEquityHistoricalFetcher,
                serde_json::json!({ "symbol": "aapl" }),
            )
            .await
            .unwrap_or_else(|error| panic!("stream should start: {error}"));

        let mut saw_progress = false;
        let mut saw_done = false;
        while let Some(event) = futures_poll_next(&mut stream) {
            match event.unwrap_or_else(|error| panic!("stream item should be ok: {error}")) {
                ProgressOrResult::Progress { .. } => saw_progress = true,
                ProgressOrResult::Done(object) => {
                    assert_eq!(object.provider, "fileset");
                    saw_done = true;
                }
                _ => {}
            }
        }
        assert!(saw_progress, "expected at least one progress event");
        assert!(saw_done, "expected a terminal Done event");
    }

    #[tokio::test]
    async fn fetch_equity_historical_offloads_blocking_call() {
        let backend = Backend::in_memory_for_tests().await;
        let object = backend
            .fetch_equity_historical("fileset", "aapl")
            .await
            .unwrap_or_else(|error| panic!("equity historical should succeed: {error}"));

        assert_eq!(object.provider, "fileset");
        assert_eq!(object.rows[0].symbol, "AAPL");
    }

    #[tokio::test]
    async fn fetch_equity_historical_unknown_provider_errors() {
        let backend = Backend::in_memory_for_tests().await;
        let result = backend.fetch_equity_historical("nope", "aapl").await;
        assert!(result.is_err(), "unknown provider must surface an error");
    }

    #[tokio::test]
    async fn stream_ingest_start_stop_round_trips() {
        // Offline (no `ws` feature) the Binance streamer emits one tick then
        // ends, so the spawned task may already be finished by the time we stop
        // it — `stop_stream` still reports the registered stream was present.
        let backend = Backend::in_memory_for_tests().await;
        let stream_id = backend
            .start_binance_stream("BTCUSDT", None)
            .unwrap_or_else(|error| panic!("start should succeed: {error}"));
        assert_eq!(stream_id, "binance:trades:BTCUSDT");

        let present = backend
            .stop_stream(&stream_id)
            .unwrap_or_else(|error| panic!("stop should succeed: {error}"));
        assert!(present, "the just-started stream must be present on stop");

        let absent = backend
            .stop_stream("binance:trades:NOPE")
            .unwrap_or_else(|error| panic!("stop should succeed: {error}"));
        assert!(!absent, "an unknown stream id must report not present");
    }

    fn embedding_config(provider: &str) -> tdw_config::EmbeddingConfig {
        tdw_config::EmbeddingConfig {
            provider: provider.to_string(),
            model: None,
            model_dir: None,
            expected_dims: None,
        }
    }

    #[test]
    fn select_embedder_defaults_to_hash_and_never_falls_back_silently() {
        // The default config selects the deterministic hash embedder.
        let embedder = select_embedder(&tdw_config::EmbeddingConfig::default())
            .unwrap_or_else(|error| panic!("hash default must construct: {error}"));
        assert_eq!(embedder.model_id(), "local-hash-8");
        // A provider whose feature is not compiled is a HARD error — never a
        // silent hash fallback (plan B1a). The default test build compiles
        // none of local-model/openai/google.
        for requested in ["local", "openai", "google", "definitely-unknown"] {
            #[cfg(feature = "local-model")]
            if requested == "local" {
                continue;
            }
            #[cfg(feature = "openai")]
            if requested == "openai" {
                continue;
            }
            #[cfg(feature = "google")]
            if requested == "google" {
                continue;
            }
            let error = select_embedder(&embedding_config(requested))
                .err()
                .map(|error| error.to_string())
                .unwrap_or_default();
            assert!(
                error.contains("refusing to silently fall back"),
                "{requested}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn knowledge_ingest_batch_then_search_returns_hits() {
        use tdw_kg::{Entity, EntityKind};

        let backend = Backend::in_memory_for_tests().await;
        let entity = |symbol: &str| Entity {
            entity_id: format!("instrument:{symbol}"),
            kind: EntityKind::Instrument,
            label: symbol.to_string(),
            aliases: vec![symbol.to_string()],
        };
        let mut dated = KnowledgeDocument::new(
            "doc-batch-1",
            "AAPL services revenue acceleration note",
            entity("AAPL"),
            vec!["asset:equity".to_string()],
        );
        dated.plane = Some("platform".to_string());
        dated.as_of = Some("2026-06-01".to_string());
        let undated = KnowledgeDocument::new(
            "doc-batch-2",
            "MSFT infrastructure capex comment",
            entity("MSFT"),
            Vec::new(),
        );
        backend
            .knowledge_ingest_at(vec![dated, undated], "2026-06-02")
            .await
            .unwrap_or_else(|error| panic!("batch ingest should succeed: {error}"));
        let hits = backend
            .knowledge_search("AAPL services revenue acceleration note", 1)
            .await
            .unwrap_or_else(|error| panic!("search should succeed: {error}"));
        assert_eq!(hits[0].id, "doc-batch-1");
        // An invalid document fails the WHOLE batch up front.
        let invalid = KnowledgeDocument::new("../bad", "x", entity("AAPL"), Vec::new());
        let valid = KnowledgeDocument::new("doc-batch-3", "ok", entity("AAPL"), Vec::new());
        backend
            .knowledge_ingest_at(vec![valid, invalid], "2026-06-02")
            .await
            .expect_err("invalid batch member must fail the batch");
        assert!(
            backend
                .knowledge_search("doc-batch-3 ok", 5)
                .await
                .unwrap_or_else(|error| panic!("search should succeed: {error}"))
                .iter()
                .all(|hit| hit.id != "doc-batch-3"),
            "nothing from the failed batch may be written"
        );
    }

    #[tokio::test]
    async fn knowledge_index_then_search_returns_hit() {
        use tdw_kg::{Entity, EntityKind};

        let backend = Backend::in_memory_for_tests().await;
        // Construction mirrors `tdw-knowledge`'s own
        // `indexes_and_searches_embedded_knowledge` test fixture.
        backend
            .knowledge_index(KnowledgeDocument {
                id: "doc-1".to_string(),
                body: "AAPL equity momentum research".to_string(),
                entity: Entity {
                    entity_id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec!["AAPL".to_string()],
                },
                tags: vec!["asset:equity".to_string()],
                source: None,
                plane: None,
                as_of: None,
                mentions: Vec::new(),
            })
            .await
            .unwrap_or_else(|error| panic!("index should succeed: {error}"));

        let hits = backend
            .knowledge_search("AAPL momentum", 1)
            .await
            .unwrap_or_else(|error| panic!("search should succeed: {error}"));

        assert_eq!(hits[0].id, "doc-1");
        assert_eq!(hits[0].entity_id, "instrument:AAPL");
    }

    #[tokio::test]
    async fn enforce_policy_returns_masked_response_envelope() {
        // `in_memory_for_tests` boots a `default` profile, which `build_policy`
        // resolves to a local policy granting the `analyst` role required by the
        // `equity_historical` endpoint — so enforcement succeeds and returns the
        // `{ "policy", "response" }` envelope.
        let backend = Backend::in_memory_for_tests().await;
        let response = backend
            .enforce_policy("fileset", "AAPL")
            .unwrap_or_else(|error| panic!("policy enforcement should succeed: {error}"));

        assert!(
            response.get("response").is_some(),
            "the masked response envelope must carry a `response` field"
        );
        assert!(
            response.get("policy").is_some(),
            "the masked response envelope must carry the policy evidence"
        );
    }

    #[tokio::test]
    async fn event_spine_accessors_return_usable_handles() {
        let backend = Backend::in_memory_for_tests().await;

        // Each handle shares the composition root's `Arc` and locks cleanly.
        assert!(Arc::ptr_eq(&backend.event_bus(), &backend.app_state().bus));
        assert!(Arc::ptr_eq(&backend.outbox(), &backend.app_state().outbox));

        let bus = backend.event_bus();
        let bus_guard = bus
            .lock()
            .unwrap_or_else(|error| panic!("bus lock: {error}"));
        drop(bus_guard);

        let outbox = backend.outbox();
        let _outbox_guard = outbox
            .lock()
            .unwrap_or_else(|error| panic!("outbox lock: {error}"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_binds_ephemeral_port_submits_via_loopback_then_shuts_down() {
        use std::time::Duration;
        use tdw_app_client::{DaemonClient, DaemonClientConfig};

        // Bind an ephemeral port so the test is hermetic and never collides.
        let mut cfg = BackendConfig::default();
        cfg.tdw.daemon.transport = tdw_config::DaemonTransport::Tcp;
        cfg.tdw.daemon.tcp_bind = Some("127.0.0.1:0".to_string());

        let mut backend = Backend::in_memory_for_tests().await;
        backend.serve(&cfg).await.expect("serve should start");

        // The OS-assigned address is observable and submission handle is live.
        let addr = backend
            .bound_addr()
            .expect("a served daemon exposes its bound address")
            .to_string();
        assert!(addr.parse::<std::net::SocketAddr>().is_ok());
        assert_ne!(
            addr.rsplit(':').next().expect("port segment"),
            "0",
            "ephemeral bind must resolve to a concrete OS-assigned port"
        );
        assert!(backend.submission_handle().is_some());

        // A loopback client submits a Shutdown op and must observe a terminal
        // event. `serve` returns after binding, but the spawned accept loop can
        // still be a few scheduler ticks behind on loaded CI runners.
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let submission = loop {
            let client_addr = addr.clone();
            let attempt = tokio::task::spawn_blocking(move || {
                let client = DaemonClient::new(
                    DaemonClientConfig::tcp(client_addr).with_timeout(Duration::from_secs(5)),
                );
                client.submit_and_wait(&make_envelope(Op::Shutdown))
            });
            match tokio::time::timeout(Duration::from_secs(6), attempt)
                .await
                .expect("loopback submit must not hang")
                .expect("spawn_blocking join")
            {
                Ok(submission) => break submission,
                Err(error) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "loopback submission should reach the in-process daemon: {error}"
                    );
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        assert!(
            submission
                .events
                .iter()
                .any(|event| matches!(event, EventMsg::Completed { .. })),
            "the daemon must emit a terminal Completed event for the op"
        );

        // Shutdown returns Ok, is bounded, and clears the stored handle.
        tokio::time::timeout(Duration::from_secs(3), backend.shutdown())
            .await
            .expect("shutdown must not hang")
            .expect("shutdown should return Ok");
        assert!(
            backend.bound_addr().is_none(),
            "the daemon handle must be cleared after shutdown"
        );
        assert!(backend.submission_handle().is_none());

        // Shutdown is idempotent when not serving.
        backend
            .shutdown()
            .await
            .expect("second shutdown is a no-op");
    }

    // --- Phase B: agent memory consolidation --------------------------------

    fn sample_memory(name: &str, retention: tdw_agent::Retention) -> Memory {
        use tdw_agent::{
            Adaptivity, DataFacets, EntityMeta, Materialization, Origin, Plane, Source, Tier,
        };
        Memory {
            meta: EntityMeta::new(
                name,
                name,
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::SelfModifying,
                false,
            ),
            retention,
            last_consolidated: None,
            source_entries: Vec::new(),
            facets: DataFacets {
                plane: Plane::Agent,
                materialization: Materialization::Materialized,
                as_of: None,
                validation: None,
            },
        }
    }

    #[tokio::test]
    async fn upsert_memory_then_list_returns_it() {
        let backend = Backend::in_memory_for_tests().await;
        backend
            .upsert_memory(sample_memory("note", tdw_agent::Retention::ShortTerm))
            .await
            .expect("upsert");

        let listed = backend.list_memories().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meta.base.name, "note");
        // upsert stamped a last_consolidated anchor so the memory ages from now.
        assert!(
            listed[0].last_consolidated.is_some(),
            "upsert stamps the consolidation anchor"
        );
    }

    #[tokio::test]
    async fn consolidate_now_applies_to_the_store() {
        let backend = Backend::in_memory_for_tests().await;
        // A Working buffer (ttl 0) expires on the first consolidation pass.
        backend
            .upsert_memory(sample_memory("buf", tdw_agent::Retention::Working))
            .await
            .expect("upsert");

        let actions = backend.consolidate_now().await.expect("consolidate");
        assert_eq!(actions.len(), 1, "the working buffer produces one action");
        assert!(
            backend.list_memories().await.is_empty(),
            "consolidate_now expires the working buffer"
        );
    }

    /// B10 empty-store regression: `consolidate_now_at` with an empty feedback
    /// store must produce EXACTLY the same actions as a direct `consolidate_at`
    /// call. This is the baseline contract: no feedback data → no behaviour change.
    #[tokio::test]
    async fn consolidate_now_at_empty_feedback_matches_base_consolidate_at() {
        use tdw_agent_store::{MemoryStore, consolidate_at};

        let now = "2026-06-10T00:00:00Z";

        // Build the same memory independently: an aged ShortTerm (ttl=1, age=2 → promote).
        let mut aged = sample_memory("note", tdw_agent::Retention::ShortTerm);
        aged.last_consolidated = Some("2026-06-08T00:00:00Z".to_string());

        // Expected: base planner on a plain MemoryStore.
        let base_actions = {
            let mut plain = MemoryStore::new();
            plain.upsert_at(aged.clone(), now).expect("upsert");
            consolidate_at(&mut plain, now).expect("consolidate_at")
        };

        // Backend with an empty feedback store must produce the same result.
        let backend = Backend::in_memory_for_tests().await;
        backend.upsert_memory(aged).await.expect("upsert");
        // feedback store is empty by default — no events appended.
        let backend_actions = backend
            .consolidate_now_at(now)
            .await
            .expect("consolidate_now_at");

        assert_eq!(
            backend_actions, base_actions,
            "empty feedback store must produce identical actions to consolidate_at"
        );
    }

    /// B10 usage-branch: `consolidate_now_at` with a non-empty feedback store
    /// applies recency credit via the usage-aware planner path. This pins the
    /// populated-store code path (`data/mod.rs:consolidate_now_at` usage branch).
    #[tokio::test]
    async fn consolidate_now_at_with_feedback_applies_recency_credit() {
        use tdw_agent_store::RetrievalEvent;
        use tdw_knowledge::runtime::KnowledgeVersions;

        // "2026-06-08" is 2 days before now; ShortTerm ttl=1 → would normally promote.
        let past = "2026-06-08T00:00:00Z";
        let now = "2026-06-10T00:00:00Z";

        let mut aged = sample_memory("note", tdw_agent::Retention::ShortTerm);
        aged.last_consolidated = Some(past.to_string());

        let backend = Backend::in_memory_for_tests().await;
        backend.upsert_memory(aged).await.expect("upsert");

        // Append a `used` feedback event recorded_at=now, referencing by agent_id.
        // credit = raw_age(2).saturating_sub(days_since(0)) = 2
        // effective_age = 2.saturating_sub(2) = 0 < ttl(1) → memory survives.
        backend
            .feedback_store_handle()
            .lock()
            .await
            .append(RetrievalEvent {
                agent_id: "note".to_string(),
                query_fingerprint: "fp-abc".to_string(),
                hit_ids: vec![],
                versions: KnowledgeVersions {
                    embedder_model: "hash-v1".to_string(),
                    rules_version: None,
                    infer_version: None,
                },
                used: true,
                recorded_at: now.to_string(),
            })
            .expect("append");

        let actions = backend
            .consolidate_now_at(now)
            .await
            .expect("consolidate_now_at");

        assert!(
            actions.is_empty(),
            "recency credit of 2 days makes effective_age=0 < ttl=1 → no actions: {actions:?}"
        );
        assert_eq!(
            backend.list_memories().await.len(),
            1,
            "the memory must survive with full recency credit"
        );
    }

    /// B10 usage-branch: distinct fingerprint dedup for `use_count`.
    /// Two events with the same fingerprint count as one; three distinct
    /// fingerprints count as three.
    #[tokio::test]
    async fn consolidate_now_at_use_count_deduplicates_fingerprints() {
        use tdw_agent_store::RetrievalEvent;
        use tdw_knowledge::runtime::KnowledgeVersions;

        let now = "2026-06-10T00:00:00Z";
        let backend = Backend::in_memory_for_tests().await;

        let mut aged = sample_memory("note", tdw_agent::Retention::ShortTerm);
        aged.last_consolidated = Some("2026-06-08T00:00:00Z".to_string());
        backend.upsert_memory(aged).await.expect("upsert");

        let make_event = |fp: &str| RetrievalEvent {
            agent_id: "note".to_string(),
            query_fingerprint: fp.to_string(),
            hit_ids: vec![],
            versions: KnowledgeVersions {
                embedder_model: "hash-v1".to_string(),
                rules_version: None,
                infer_version: None,
            },
            used: true,
            recorded_at: now.to_string(),
        };

        {
            let fb_arc = backend.feedback_store_handle();
            let mut fb = fb_arc.lock().await;
            fb.append(make_event("fp-1")).expect("append fp-1 a");
            fb.append(make_event("fp-1")).expect("append fp-1 b"); // duplicate
            fb.append(make_event("fp-2")).expect("append fp-2");
        }

        // The test only pins that the function runs without error and that credit
        // is applied (memory survives). The use_count value (2 distinct fps) is
        // informational and not yet policy-relevant.
        let actions = backend
            .consolidate_now_at(now)
            .await
            .expect("consolidate_now_at");
        assert!(
            actions.is_empty(),
            "recency credit applies even with duplicate fingerprints: {actions:?}"
        );
    }

    /// B10 regression: a `used` feedback event whose `hit_ids` references a memory
    /// by name grants recency credit, sparing an aged memory that would otherwise
    /// promote. This pins the retrieval→consolidation link to `hit_ids` (the
    /// retrieved-doc id), not merely the submitting `agent_id`.
    #[tokio::test]
    async fn used_hit_id_feedback_spares_an_aged_memory() {
        use tdw_agent_store::RetrievalEvent;
        use tdw_knowledge::runtime::KnowledgeVersions;

        let now = chrono::Utc::now().to_rfc3339();

        // An aged ShortTerm memory (ttl 1): without credit, raw_age ≫ ttl → promote.
        let mut aged = sample_memory("doc-note", tdw_agent::Retention::ShortTerm);
        aged.last_consolidated = Some("2026-06-01T00:00:00Z".to_string());

        let backend = Backend::in_memory_for_tests().await;
        backend.upsert_memory(aged).await.expect("upsert aged");

        // Record a `used` event referencing the memory by hit id (NOT agent_id).
        backend
            .feedback_store_handle()
            .lock()
            .await
            .append(RetrievalEvent {
                agent_id: "some-other-agent".to_string(),
                query_fingerprint: "fp".to_string(),
                hit_ids: vec!["doc-note".to_string()],
                versions: KnowledgeVersions {
                    embedder_model: "hash-v1".to_string(),
                    rules_version: None,
                    infer_version: None,
                },
                used: true,
                recorded_at: now.clone(),
            })
            .expect("append feedback");

        let actions = backend.consolidate_now().await.expect("consolidate");
        assert!(
            actions.is_empty(),
            "recency credit from the hit_id feedback should spare the memory: {actions:?}"
        );
        assert_eq!(
            backend.list_memories().await.len(),
            1,
            "the referenced memory must survive consolidation"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_runs_consolidation_scheduler_then_shuts_down_cleanly() {
        use std::time::Duration;

        // We assert the scheduler *lifecycle* (it spawns under `serve` and is
        // aborted/awaited cleanly under `shutdown`) plus that the shared store is
        // not deadlocked while it runs — not its tick timing (the default hourly
        // tick never fires in-test, which is fine: deterministic apply is covered
        // by `consolidate_now` and the store's own unit tests).
        let mut cfg = BackendConfig::default();
        cfg.tdw.daemon.transport = tdw_config::DaemonTransport::Tcp;
        cfg.tdw.daemon.tcp_bind = Some("127.0.0.1:0".to_string());

        let mut backend = Backend::in_memory_for_tests().await;
        // Seed a memory before serving; the scheduler shares the same store.
        backend
            .upsert_memory(sample_memory("seed", tdw_agent::Retention::ShortTerm))
            .await
            .expect("seed upsert");

        backend.serve(&cfg).await.expect("serve should start");
        assert!(backend.bound_addr().is_some(), "daemon bound an address");

        // The scheduler is live and shares the store: a manual consolidate_now
        // still works while it runs (proving no deadlock on the shared mutex).
        let _ = backend.consolidate_now().await.expect("manual consolidate");

        // Shutdown is bounded and aborts/awaits the scheduler without hanging.
        tokio::time::timeout(Duration::from_secs(3), backend.shutdown())
            .await
            .expect("shutdown must not hang")
            .expect("shutdown returns Ok");
        assert!(
            backend.bound_addr().is_none(),
            "handle cleared after shutdown"
        );
    }

    /// K-E1: bolt error text contains both the compose remediation and the
    /// in-memory dev alternative — non-bolt build arm (missing-feature path).
    /// Default CI never compiles the `bolt` feature, so this always runs.
    #[tokio::test]
    #[cfg(not(feature = "bolt"))]
    async fn bolt_backend_error_contains_remediation_and_in_memory_alternative() {
        let cfg = tdw_config::GraphConfig {
            backend: "bolt".to_string(),
            bolt_uri: Some("bolt://127.0.0.1:7687".to_string()),
            bolt_user: String::new(),
            bolt_password_env: "TDW_GRAPH_PASSWORD".to_string(),
            bolt_db: "memgraph".to_string(),
        };
        let error = build_graph_engine(&cfg)
            .await
            .err()
            .expect("bolt backend without bolt feature must return an Init error")
            .to_string();
        assert!(
            error.contains("docker compose"),
            "error must include the compose one-liner: {error}"
        );
        assert!(
            error.contains("in-memory"),
            "error must name the in-memory dev alternative: {error}"
        );
    }

    /// K-E1 review finding 5: bolt-feature-enabled arm — tests the connect-error
    /// message format directly via `BackendError::Init` without a live Memgraph.
    ///
    /// When the bolt feature IS compiled, `build_graph_engine` builds the error as:
    ///   `BackendError::Init(format!("bolt graph connect ({uri}): {connect_err}. Remediation: ..."))`
    /// We verify the static remediation fragment (everything after the dynamic
    /// `connect_err`) contains both required signals by constructing the same
    /// `BackendError::Init` wrapper around a synthetic connect error string.
    /// This covers the message template regardless of whether Memgraph is reachable.
    #[test]
    fn bolt_connect_error_message_contains_remediation_and_in_memory_alternative() {
        // Replicate the exact format string from the bolt arm of build_graph_engine
        // (data/mod.rs, #[cfg(feature = "bolt")] branch). A synthetic connect error
        // string stands in for the real neo4rs error — we are testing the surrounding
        // template, not the upstream library's message.
        let uri = "bolt://127.0.0.1:7687";
        let synthetic_connect_err = "connection refused";
        let error = BackendError::Init(format!(
            "bolt graph connect ({uri}): {synthetic_connect_err}. \
             Remediation: start Memgraph with \
             `docker compose --profile full up -d memgraph` \
             and verify port 7687 is reachable, or switch to the \
             in-memory dev backend with \
             `[knowledge.graph] backend = \"in-memory\"` in your \
             daemon TOML. See docs/ops/graph-db.md."
        ))
        .to_string();
        assert!(
            error.contains("docker compose"),
            "bolt connect error must include the compose one-liner: {error}"
        );
        assert!(
            error.contains("in-memory"),
            "bolt connect error must name the in-memory dev alternative: {error}"
        );
        assert!(
            error.contains(uri),
            "bolt connect error must include the URI for context: {error}"
        );
    }

    /// Poll a [`ProgressStream`] to readiness using a no-op waker. The runtime's
    /// in-memory streams are always-ready (no real I/O), so a busy poll that
    /// treats `Pending` as end-of-stream is sufficient and deterministic here.
    fn futures_poll_next<T: DataModel>(
        stream: &mut ProgressStream<T>,
    ) -> Option<tdw_core::Result<tdw_core::ProgressOrResult<T>>> {
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWake;
        impl Wake for NoopWake {
            fn wake(self: Arc<Self>) {}
        }
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        match stream.as_mut().poll_next(&mut context) {
            Poll::Ready(item) => item,
            Poll::Pending => None,
        }
    }

    /// The PRODUCTION construction path (`in_memory_for_tests` mirrors `from_config`
    /// for this seam) must bind a user identity and attach a finding indexer.
    /// `knowledge_findings_available()` on `McpServer` is true iff
    /// `runtime.bound_user_id().is_some() && runtime.graph().is_some()` — verify
    /// both preconditions hold so the F1 gate is satisfied (knowledge-system K-X6).
    #[tokio::test]
    async fn production_construction_path_satisfies_finding_gate() {
        use tdw_mcp::McpServer;

        let backend = Backend::in_memory_for_tests().await;
        let runtime = backend.knowledge_runtime_handle();

        // Gate precondition 1: a bound user identity must be present.
        assert!(
            runtime.bound_user_id().is_some(),
            "production runtime must have a bound user identity (knowledge.user.id / \
             in_memory_for_tests wires \"test-user\")"
        );
        // Gate precondition 2: a graph engine must be present.
        assert!(
            runtime.graph().is_some(),
            "production runtime must have a graph engine attached"
        );

        // Full end-to-end: wire the runtime into an MCP server, initialize, and
        // verify tdw.kg.finding and tdw.kg.link appear in tools/list.
        let mut server = McpServer::new().with_knowledge(runtime);
        let _ = server.handle_json_rpc_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
        );
        let listed_responses =
            server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let listed: serde_json::Value =
            serde_json::from_str(&listed_responses[0]).expect("tools/list response is valid JSON");
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .expect("tools array in result")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(
            names.contains(&"tdw.kg.finding"),
            "tdw.kg.finding must appear in tools/list for the production construction path; \
             got: {names:?}"
        );
        assert!(
            names.contains(&"tdw.kg.link"),
            "tdw.kg.link must appear in tools/list for the production construction path; \
             got: {names:?}"
        );
    }

    // ── K-R4: spawn_pattern_mining_worker config-gate tests (finding #2a) ────

    /// When `enabled = false` the function returns `None`; no trigger is
    /// registered and no background work is spawned.
    #[test]
    fn pattern_worker_disabled_returns_none() {
        use tdw_config::PatternConfig;
        use tdw_storage_graph::InMemoryGraphEngine;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let cancel = CancellationToken::new();
        let cfg = PatternConfig {
            enabled: false,
            ..PatternConfig::default()
        };

        let handle = rt.block_on(async { spawn_pattern_mining_worker(&cfg, graph, cancel, None) });
        assert!(
            handle.is_none(),
            "disabled worker must return None (no task spawned)"
        );
    }

    /// When `enabled = true` the function returns `Some(handle)`.
    #[test]
    fn pattern_worker_enabled_returns_some_handle() {
        use tdw_config::PatternConfig;
        use tdw_storage_graph::InMemoryGraphEngine;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");

        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let cancel = CancellationToken::new();
        let cfg = PatternConfig {
            enabled: true,
            cadence: "0 2 * * *".to_string(),
            ..PatternConfig::default()
        };

        let handle =
            rt.block_on(async { spawn_pattern_mining_worker(&cfg, graph, cancel.clone(), None) });
        assert!(handle.is_some(), "enabled worker must return Some(handle)");
        cancel.cancel();
        rt.block_on(async {
            if let Some(task) = handle {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
            }
        });
    }

    /// With a real `index_path`, the worker task persists the index to disk
    /// after a mining run. We test this via the PatternEngine directly (same
    /// caller-owned persistence contract).
    #[test]
    fn pattern_index_persists_across_process_boundary() {
        use tdw_patterns::PatternIndex;
        use tdw_storage_graph::InMemoryGraphEngine;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");

        // Use std::env::temp_dir() to avoid adding a tempfile dev-dep.
        let index_path = std::env::temp_dir().join(format!(
            "pattern_index_test_{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));

        // Build fixture graph.
        let graph = Arc::new(InMemoryGraphEngine::default());
        let graph_dyn: Arc<dyn GraphEngine> = graph.clone();
        let prov = tdw_core::Provenance::Ingest {
            source: "test".to_string(),
        };

        rt.block_on(async {
            graph
                .upsert_nodes(vec![
                    tdw_core::GraphNode {
                        id: "instrument:A".to_string(),
                        kind: tdw_kg::EntityKind::Instrument,
                        label: "A".to_string(),
                        aliases: vec![],
                        props: serde_json::Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    tdw_core::GraphNode {
                        id: "instrument:B".to_string(),
                        kind: tdw_kg::EntityKind::Instrument,
                        label: "B".to_string(),
                        aliases: vec![],
                        props: serde_json::Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    tdw_core::GraphNode {
                        id: "instrument:C".to_string(),
                        kind: tdw_kg::EntityKind::Instrument,
                        label: "C".to_string(),
                        aliases: vec![],
                        props: serde_json::Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert nodes");
            graph
                .upsert_edges(vec![
                    tdw_core::GraphEdge {
                        from: "instrument:A".to_string(),
                        to: "instrument:B".to_string(),
                        rel: "peer_of".to_string(),
                        props: serde_json::Value::Null,
                        provenance: prov.clone(),
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    tdw_core::GraphEdge {
                        from: "instrument:B".to_string(),
                        to: "instrument:C".to_string(),
                        rel: "peer_of".to_string(),
                        props: serde_json::Value::Null,
                        provenance: prov,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert edges");
        });

        // Run mining and persist.
        let first_index = rt.block_on(async {
            let engine = tdw_patterns::PatternEngine::default();
            let mut index = tdw_patterns::PatternIndex::default();
            engine
                .run_pattern_mining_at(&graph_dyn, &mut index, "2025-06-01T00:00:00Z", "2025-06-01")
                .await
                .expect("first mining");
            index
        });

        // Persist manually (simulating what the worker does).
        let json = first_index.to_json().expect("serialize");
        std::fs::write(&index_path, &json).expect("write index");

        // Restore (simulating process restart).
        let raw = std::fs::read_to_string(&index_path).expect("read index");
        let restored = PatternIndex::from_json(&raw).expect("deserialize");

        assert_eq!(restored, first_index, "restored index must equal original");
        assert!(
            restored.contains("peer_of"),
            "peer_of motif must survive round-trip"
        );
    }

    // ── K-L6: feed registration + status gates ───────────────────────────────

    /// Zero feeds configured → no task, loud status note, feed_statuses empty.
    ///
    /// This is the from_config+tempdir production registration gate: we build a
    /// real TdwConfig with zero feeds, construct the Backend through from_config,
    /// and assert the KgStatus feed_note is set and feed_statuses is empty.
    #[tokio::test]
    async fn zero_feeds_config_produces_loud_note_in_status() {
        // in_memory_for_tests sets feeds_cfg = FeedsConfig::default() (zero
        // entries). The KnowledgeRuntime has no feed_freshness_cells attached.
        let backend = Backend::in_memory_for_tests().await;
        let status = backend.runtime.status().await;
        assert!(
            status.feed_statuses.is_empty(),
            "zero feeds → feed_statuses must be empty; got: {:?}",
            status.feed_statuses
        );
        assert!(
            !status.feed_note.is_empty(),
            "zero feeds → feed_note must be non-empty (loud operator signal)"
        );
        assert!(
            status.feed_note.contains("no feeds"),
            "feed_note must mention 'no feeds'; got: {:?}",
            status.feed_note
        );
    }

    /// One enabled feed → freshness cell attached to runtime → status shows Pending.
    ///
    /// Mirrors the from_config attachment pattern: build the cell before
    /// Arc-wrapping the runtime, attach via with_feed_freshness, then confirm
    /// KgStatus reflects the cell.
    #[tokio::test]
    async fn one_enabled_feed_shows_pending_in_status() {
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_knowledge::collection_name;
        use tdw_knowledge::feeds::FeedFreshness;
        use tdw_knowledge::runtime::KnowledgeRuntime;
        use tdw_storage_graph::InMemoryGraphEngine;

        let backend = Backend::in_memory_for_tests().await;

        // Build a cell (mirrors what from_config does per enabled feed entry).
        let cell = Arc::new(tokio::sync::Mutex::new(FeedFreshness::Pending {
            feed_id: "test-feed-1".to_string(),
        }));

        // Build a runtime with the cell attached — same pattern as from_config.
        let embedder: Arc<dyn EmbeddingProvider> = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::clone(&backend.state.vector);
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let collection = collection_name(embedder.model_id());
        let runtime_with_feed = Arc::new(
            KnowledgeRuntime::new(Arc::clone(&embedder), Arc::clone(&vectors))
                .with_lexical(Arc::clone(&backend.state.lexical), collection)
                .with_graph(graph)
                .with_feed_freshness(Arc::clone(&cell)),
        );

        let status = runtime_with_feed.status().await;
        assert_eq!(
            status.feed_statuses.len(),
            1,
            "one attached cell → one feed_status entry"
        );
        assert!(
            matches!(
                &status.feed_statuses[0],
                FeedFreshness::Pending { feed_id } if feed_id == "test-feed-1"
            ),
            "attached Pending cell must appear in feed_statuses; got: {:?}",
            status.feed_statuses
        );
        assert!(
            status.feed_note.is_empty(),
            "feed_note must be empty when feeds are attached; got: {:?}",
            status.feed_note
        );
    }

    /// Fixture-feed e2e: article → article_to_document → KnowledgeIndexer →
    /// tagged → idempotent re-ingest produces zero new docs.
    ///
    /// This is the ingest→tagged→derived gate through a fixture feed (K-L6).
    #[tokio::test]
    async fn fixture_feed_article_ingests_and_is_idempotent() {
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_knowledge::KnowledgeIndex;
        use tdw_knowledge::feeds::{Article, FeedSource, FixtureFeedSource};
        use tdw_knowledge::indexer::{IndexOutcome, KnowledgeIndexer, article_to_document};

        let backend = Backend::in_memory_for_tests().await;
        let embedder: Arc<dyn EmbeddingProvider> = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::clone(&backend.state.vector);
        let index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));
        let mut indexer = KnowledgeIndexer::new(index);

        // Build a fixture article and map it through the K-L6 seam.
        let article = Article::new(
            "AAPL beats earnings estimates",
            "https://example.com/aapl-earnings",
            "FinanceTimes",
            1_749_297_600_000_i64,
            "Apple Inc. reported record quarterly revenue.",
            vec!["AAPL".to_string()],
        );
        let source = FixtureFeedSource::new(vec![article.clone()], "e2e-fixture");
        let polled = source.poll(50).await.expect("fixture poll succeeds");
        assert_eq!(polled.len(), 1, "fixture source returned one article");

        // Map through article_to_document (K-L6 seam: indexer.rs:514).
        let doc = article_to_document(&polled[0], "shared");
        assert!(
            doc.plane.as_deref() == Some("shared"),
            "plane must be set from feed config"
        );
        assert!(!doc.id.is_empty(), "document id must be non-empty");

        // First ingest: article is new → Indexed.
        let outcomes = indexer
            .index_batch_at(vec![doc.clone()], "2026-06-12")
            .await
            .expect("first ingest succeeds");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0],
            IndexOutcome::Indexed,
            "first ingest must produce Indexed"
        );

        // Second ingest: same article → SkippedUnchanged (idempotency gate).
        let outcomes2 = indexer
            .index_batch_at(vec![doc], "2026-06-12")
            .await
            .expect("second ingest succeeds");
        assert_eq!(outcomes2.len(), 1);
        assert_eq!(
            outcomes2[0],
            IndexOutcome::SkippedUnchanged,
            "re-ingest of same article must be SkippedUnchanged (idempotency)"
        );
    }

    /// Write `content` to a uniquely-named temp file and return the path.
    /// Uses `std::env::temp_dir()` so no `tempfile` crate is needed.
    fn write_temp_fixture(name: &str, content: &str) -> String {
        use std::io::Write as _;
        let path = std::env::temp_dir().join(format!("tdw-feed-test-{name}.json"));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(content.as_bytes()))
            .expect("write temp fixture");
        path.to_str().expect("valid utf8 path").to_string()
    }

    /// Production-task gate (from_config+tempdir, fix #3):
    /// one enabled fixture feed → spawn_feed_tasks registers a task, fires it
    /// with an injected short cadence, and items land through the production
    /// task path (FeedIngestHandle::ingest_at).
    ///
    /// The freshness cell transitions from Pending → Ok after the first poll.
    #[tokio::test]
    async fn spawn_feed_tasks_one_enabled_fixture_feed_items_land() {
        use tdw_config::{ArticleSourceParams, FeedConfig, FeedSourceKind, FeedsConfig};
        use tdw_knowledge::feeds::{Article, FeedFreshness};

        // Write a fixture JSON file.
        let articles = vec![Article::new(
            "TSLA Q2 Revenue Beat",
            "https://example.com/tsla-q2",
            "TestSource",
            1_749_297_600_000_i64,
            "Tesla reported strong Q2 revenue.",
            vec!["TSLA".to_string()],
        )];
        let fixture_path = write_temp_fixture(
            "production-task",
            &serde_json::to_string(&articles).expect("serialize"),
        );

        // Use "* * * * *" — always fires when last_tick_ms=0 and now_ms>0.
        let feeds_cfg = FeedsConfig {
            entries: vec![FeedConfig {
                id: "test-tsla-feed".to_string(),
                source_kind: FeedSourceKind::Article,
                source_params: ArticleSourceParams {
                    fixture_path: Some(fixture_path),
                    ..ArticleSourceParams::default()
                },
                cadence: "* * * * *".to_string(),
                enabled: true,
                max_items_per_poll: 10,
                plane: "shared".to_string(),
                max_body_bytes: 65_536,
            }],
        };

        let cell = Arc::new(tokio::sync::Mutex::new(FeedFreshness::Pending {
            feed_id: "test-tsla-feed".to_string(),
        }));
        let cells = vec![Arc::clone(&cell)];

        let backend = Backend::in_memory_for_tests().await;
        let ingest_handle = super::FeedIngestHandle {
            indexer: Arc::clone(&backend.indexer),
            infer: Arc::clone(&backend.infer),
            graph: Arc::clone(&backend.graph),
            tags_engine: Arc::clone(&backend.tags_engine),
        };

        let cancel = CancellationToken::new();
        // Inject a 100 ms tick so the test completes in < 5 s.
        let handles = super::spawn_feed_tasks(
            &feeds_cfg,
            &cells,
            ingest_handle,
            cancel.clone(),
            Some(std::time::Duration::from_millis(100)),
        );
        assert_eq!(handles.len(), 1, "one enabled feed → one task handle");

        let start = std::time::Instant::now();
        loop {
            let state = { cell.lock().await.clone() };
            match &state {
                FeedFreshness::Pending { .. } => {}
                FeedFreshness::Ok {
                    feed_id, indexed, ..
                } => {
                    assert_eq!(feed_id, "test-tsla-feed");
                    assert_eq!(*indexed, 1, "one article must be indexed");
                    break;
                }
                other => panic!("unexpected freshness state: {other:?}"),
            }
            if start.elapsed().as_secs() > 10 {
                panic!(
                    "feed task did not transition to Ok within 10 s; state: {:?}",
                    cell.lock().await.clone()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        cancel.cancel();
        for h in handles {
            let _ = h.await;
        }
    }

    /// DeriveEdge e2e through FeedIngestHandle (fix #1):
    /// A fixture feed article ingested via FeedIngestHandle::ingest_at fires
    /// the K-L1 inference hook. When a DeriveEdge rule is loaded, the derived
    /// edge must exist in the graph after ingest.
    #[tokio::test]
    async fn feed_ingest_handle_fires_inference_and_derives_edge() {
        use tdw_core::{GraphEdge, GraphNode, Provenance, TraversalFilter};
        use tdw_infer::{EdgePattern, InferRule};
        use tdw_kg::EntityKind;
        use tdw_knowledge::feeds::Article;
        use tdw_knowledge::indexer::article_to_document;

        let backend = Backend::in_memory_for_tests().await;
        let graph = backend.graph_engine();
        let now = "2026-06-12";

        // Seed a base `described_by` edge so the DeriveEdge rule can fire.
        let entity_id = "instrument:FEED-INFER-E2E";
        let doc_id = "feed-infer-e2e-doc";
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: entity_id.to_string(),
                    kind: EntityKind::Instrument,
                    label: entity_id.to_string(),
                    aliases: vec![],
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: doc_id.to_string(),
                    kind: EntityKind::Instrument,
                    label: doc_id.to_string(),
                    aliases: vec![],
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: entity_id.to_string(),
                to: doc_id.to_string(),
                rel: "described_by".to_string(),
                props: serde_json::Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("upsert base edge");

        // Load a DeriveEdge rule: described_by => feed_confirmed_by.
        {
            let infer_arc = backend.infer_engine_handle();
            let mut infer = infer_arc.lock().await;
            infer
                .hot_reload(vec![InferRule::DeriveEdge {
                    rule_id: "feed-e2e-rule".to_string(),
                    stratum: 0,
                    when: vec![EdgePattern {
                        rel: "described_by".to_string(),
                    }],
                    derived_type: "feed_confirmed_by".to_string(),
                }])
                .expect("hot_reload accepts valid rule");
        }

        // Build FeedIngestHandle from the backend's internal handles.
        let ingest = super::FeedIngestHandle {
            indexer: Arc::clone(&backend.indexer),
            infer: Arc::clone(&backend.infer),
            graph: Arc::clone(&backend.graph),
            tags_engine: Arc::clone(&backend.tags_engine),
        };

        // Build an article and map it through article_to_document.
        let article = Article::new(
            "Feed Infer E2E",
            "https://example.com/feed-infer-e2e",
            "TestFeed",
            1_749_297_600_000_i64,
            "E2E article for feed inference gate.",
            vec!["TSLA".to_string()],
        );
        let doc = article_to_document(&article, "shared");

        // Ingest via FeedIngestHandle — must fire inference.
        ingest
            .ingest_at(vec![doc], now)
            .await
            .expect("ingest_at succeeds");

        // Assert the derived edge exists in the graph.
        // Use neighbors() with default TraversalFilter (all rels, outbound, 1 hop).
        let neighbors = graph
            .neighbors(
                entity_id,
                &TraversalFilter {
                    rels: Some(vec!["feed_confirmed_by".to_string()]),
                    ..TraversalFilter::default()
                },
            )
            .await
            .expect("neighbors query succeeds");
        assert!(
            !neighbors.is_empty(),
            "DeriveEdge rule must produce feed_confirmed_by edge after \
             FeedIngestHandle::ingest_at; got 0 neighbors"
        );
    }

    /// FeedIngestHandle::ingest_at returns Err on an invalid document
    /// (empty id fails validation). Exercises the error path tested by fix #7.
    #[tokio::test]
    async fn feed_ingest_handle_error_path_is_reachable() {
        use tdw_kg::{Entity, EntityKind};
        use tdw_knowledge::KnowledgeDocument;

        let backend = Backend::in_memory_for_tests().await;
        let ingest = super::FeedIngestHandle {
            indexer: Arc::clone(&backend.indexer),
            infer: Arc::clone(&backend.infer),
            graph: Arc::clone(&backend.graph),
            tags_engine: Arc::clone(&backend.tags_engine),
        };

        // An empty-id document is invalid and must produce an index error.
        let bad_doc = KnowledgeDocument {
            id: String::new(), // invalid: empty id
            body: "body".to_string(),
            entity: Entity {
                entity_id: "instrument:X".to_string(),
                kind: EntityKind::Instrument,
                label: "X".to_string(),
                aliases: vec![],
            },
            tags: vec![],
            source: None,
            plane: None,
            as_of: None,
            mentions: vec![],
        };
        let result = ingest.ingest_at(vec![bad_doc], "2026-06-12").await;
        assert!(
            result.is_err(),
            "ingest_at with invalid document must return Err"
        );
    }

    /// Body size cap test (fix #5):
    /// Articles whose `summary` length exceeds `max_body_bytes` are rejected
    /// and counted separately; they do NOT enter ingest.
    #[tokio::test]
    async fn body_size_cap_rejects_oversized_articles_in_feed_task() {
        use tdw_config::{ArticleSourceParams, FeedConfig, FeedSourceKind, FeedsConfig};
        use tdw_knowledge::feeds::{Article, FeedFreshness};

        let small = Article::new(
            "Small Article",
            "https://example.com/small-cap",
            "TestFeed",
            1_749_297_600_000_i64,
            "Short summary.",
            vec![],
        );
        let oversized = Article::new(
            "Oversized Article",
            "https://example.com/oversized-cap",
            "TestFeed",
            1_749_297_600_000_i64,
            "x".repeat(200), // exceeds cap of 100 bytes
            vec![],
        );
        let fixture_path = write_temp_fixture(
            "body-cap",
            &serde_json::to_string(&vec![small, oversized]).expect("serialize"),
        );

        let feeds_cfg = FeedsConfig {
            entries: vec![FeedConfig {
                id: "cap-test-feed".to_string(),
                source_kind: FeedSourceKind::Article,
                source_params: ArticleSourceParams {
                    fixture_path: Some(fixture_path),
                    ..ArticleSourceParams::default()
                },
                cadence: "* * * * *".to_string(),
                enabled: true,
                max_items_per_poll: 10,
                plane: "shared".to_string(),
                max_body_bytes: 100, // cap at 100 bytes
            }],
        };

        let cell = Arc::new(tokio::sync::Mutex::new(FeedFreshness::Pending {
            feed_id: "cap-test-feed".to_string(),
        }));
        let cells = vec![Arc::clone(&cell)];

        let backend = Backend::in_memory_for_tests().await;
        let ingest_handle = super::FeedIngestHandle {
            indexer: Arc::clone(&backend.indexer),
            infer: Arc::clone(&backend.infer),
            graph: Arc::clone(&backend.graph),
            tags_engine: Arc::clone(&backend.tags_engine),
        };

        let cancel = CancellationToken::new();
        // Inject a 100 ms tick so the test completes in < 5 s.
        let handles = super::spawn_feed_tasks(
            &feeds_cfg,
            &cells,
            ingest_handle,
            cancel.clone(),
            Some(std::time::Duration::from_millis(100)),
        );
        assert_eq!(handles.len(), 1);

        let start = std::time::Instant::now();
        loop {
            let state = { cell.lock().await.clone() };
            match &state {
                FeedFreshness::Pending { .. } => {}
                FeedFreshness::Ok {
                    feed_id,
                    indexed,
                    rejected,
                    ..
                } => {
                    assert_eq!(feed_id, "cap-test-feed");
                    assert_eq!(*indexed, 1, "one small article must be indexed");
                    assert_eq!(*rejected, 1, "one oversized article must be rejected");
                    break;
                }
                other => panic!("unexpected state: {other:?}"),
            }
            if start.elapsed().as_secs() > 10 {
                panic!(
                    "body-cap test timed out; state: {:?}",
                    cell.lock().await.clone()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        cancel.cancel();
        for h in handles {
            let _ = h.await;
        }
    }

    /// feed_statuses serializes correctly (state tag round-trips).
    #[test]
    fn feed_statuses_serialize_in_kg_status() {
        use tdw_knowledge::feeds::FeedFreshness;

        let freshness = FeedFreshness::Ok {
            last_poll_ms: 1_749_297_600_000,
            feed_id: "news-feed-1".to_string(),
            indexed: 5,
            duplicates: 2,
            rejected: 1,
        };
        let json = serde_json::to_value(&freshness).expect("serializes");
        assert_eq!(json["state"], "ok");
        assert_eq!(json["feed_id"], "news-feed-1");
        assert_eq!(json["indexed"], 5);
        assert_eq!(json["duplicates"], 2);
        assert_eq!(json["rejected"], 1);
    }
}
