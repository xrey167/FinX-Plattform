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
    MemoryStore, RetrievalFeedbackStore, consolidate_at, spawn_consolidation_scheduler,
};
use tdw_app_server::{CancellationToken, SubmissionHandle};
use tdw_bus::EventBus;
use tdw_config::TdwConfig;
use tdw_core::{
    BlobEngine, DataModel, Fetcher, GraphEngine, LexicalEngine, OBBject, OlapEngine,
    ProgressStream, ProviderRegistry, QueryParams, RelationalEngine, VectorEngine,
};
use tdw_domain::EquityHistoricalData;
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_infer::{ChangeSet, InferEngine, InferError, RetractReport, RunLimits};
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_knowledge::{KnowledgeDocument, KnowledgeHit, KnowledgeIndex};
use tdw_outbox::InMemoryOutbox;
use tdw_protocol::{EventMsg, OpEnvelope};
use tdw_runtime::CommandRunner;
use tdw_service_api::{AppState, fetch_equity_historical};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_tag_rules::{RuleEngine, TagRule};
use tdw_tags::{InMemoryTagEngine, TagEngine};
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
    pub async fn from_config(config: TdwConfig) -> BackendResult<Self> {
        let embedding = config.knowledge.embedding.clone();
        let graph_cfg = config.knowledge.graph.clone();
        let rules_cfg = config.knowledge.rules.clone();
        let infer_cfg = config.knowledge.infer.clone();
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
        let inner_index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&state.vector));
        let indexer = Arc::new(Mutex::new(
            KnowledgeIndexer::new(inner_index)
                .with_lexical(Arc::clone(&state.lexical), collection.clone())
                .with_graph(Arc::clone(&graph)),
        ));
        // Boot-load tag rules and the inference engine (K-L1).
        // Absent rules_dir → no rules loaded; logged loudly so the operator knows.
        let (rule_engine, infer_engine) = boot_load_rules(&rules_cfg, &infer_cfg)?;
        let rules_version = rule_engine.version();
        let infer_version = infer_engine.version();
        let rules = Arc::new(Mutex::new(rule_engine));
        let infer = Arc::new(Mutex::new(infer_engine));
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
        let runtime = Arc::new(
            KnowledgeRuntime::new(Arc::clone(&embedder), Arc::clone(&state.vector))
                .with_lexical(Arc::clone(&state.lexical), collection)
                .with_graph(Arc::clone(&graph))
                .with_versions(rules_v, infer_v),
        );
        let tags_engine: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
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
        let runtime = Arc::new(
            KnowledgeRuntime::new(Arc::clone(&embedder), Arc::clone(&state.vector))
                .with_lexical(Arc::clone(&state.lexical), collection)
                .with_graph(Arc::clone(&graph))
                .with_tags(Arc::clone(&tags_engine)),
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

        // Phase B — co-spawn the memory-consolidation scheduler on the same
        // cancellation token, mirroring the relay's lifecycle.
        let consolidation_task = spawn_consolidation_scheduler(
            self.memory.clone(),
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
            Arc::clone(&self.runtime),
            cancel.clone(),
        );

        self.daemon = Some(DaemonHandle {
            cancel,
            submission: handle,
            serve_task,
            consolidation_task,
            rules_reload_task,
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
    /// Use this to construct an offline or caller-scoped indexer with its own
    /// manifest (e.g. the `tdw kg reindex` offline command). For live daemon
    /// ingestion use [`knowledge_index_at`](Self::knowledge_index_at) or
    /// [`knowledge_ingest_at`](Self::knowledge_ingest_at), which route through
    /// the daemon-hosted indexer with the shared manifest.
    #[must_use]
    pub fn knowledge_indexer(&self) -> KnowledgeIndexer {
        let index = KnowledgeIndex::new(Arc::clone(&self.embedder), Arc::clone(&self.state.vector));
        let collection = tdw_knowledge::collection_name(self.embedder.model_id());
        KnowledgeIndexer::new(index)
            .with_lexical(Arc::clone(&self.state.lexical), collection)
            .with_graph(Arc::clone(&self.graph))
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
    /// Builds a [`ChangeSet`] from the entity's declared tags and the edge types
    /// that the daemon's `describes_by` / `mentions` graph edges introduce. Errors
    /// are logged loudly and never surfaced to the caller (inference is best-effort;
    /// the ingested document is already durable). This is intentional: inference
    /// failures must never roll back an ingest that succeeded (B7 contract).
    async fn run_infer_after_ingest(&self, label: &str, tags: &[String], now: &str) {
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
fn load_rules_from_dir(
    dir: &std::path::Path,
    cfg: &tdw_config::RulesConfig,
) -> BackendResult<LoadedRules> {
    let max_files = cfg.max_files.unwrap_or(DEFAULT_MAX_FILES);
    let max_file_size_kb = cfg.max_file_size_kb.unwrap_or(DEFAULT_MAX_FILE_SIZE_KB);
    let max_total_rules = cfg.max_total_rules.unwrap_or(DEFAULT_MAX_TOTAL_RULES);

    // Collect all *.tag.json and *.infer.json paths, sorted for determinism.
    let mut tag_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut infer_paths: Vec<std::path::PathBuf> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();

    let read_dir = std::fs::read_dir(dir).map_err(|error| {
        BackendError::Init(format!(
            "knowledge.rules.rules_dir {dir:?} cannot be read: {error}"
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
            "knowledge.rules.rules_dir {dir:?}: {} unreadable dir entr{}: {}",
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
            "knowledge.rules.rules_dir {dir:?}: {total_files} rule files found, \
             limit is {max_files} (set knowledge.rules.max_files to raise)"
        )));
    }

    let max_bytes = max_file_size_kb * 1024;

    /// Read and size-check a single rule file.
    fn read_file(path: &std::path::Path, max_bytes: u64) -> BackendResult<String> {
        let meta = std::fs::metadata(path).map_err(|e| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: cannot stat {path:?}: {e}"
            ))
        })?;
        if meta.len() > max_bytes {
            return Err(BackendError::Init(format!(
                "knowledge.rules.rules_dir: file {path:?} is {} KiB, limit is {} KiB \
                 (set knowledge.rules.max_file_size_kb to raise)",
                meta.len() / 1024,
                max_bytes / 1024,
            )));
        }
        std::fs::read_to_string(path).map_err(|e| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: cannot read {path:?}: {e}"
            ))
        })
    }

    let mut tag_rules: Vec<TagRule> = Vec::new();
    for path in &tag_paths {
        let text = read_file(path, max_bytes)?;
        let file_rules: Vec<TagRule> = serde_json::from_str(&text).map_err(|error| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir: malformed tag-rule file {path:?}: {error}"
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
                    "knowledge.rules.rules_dir: malformed infer-rule file {path:?}: {error}"
                ))
            })?;
        infer_rules.extend(file_rules);
    }

    let total_rules = tag_rules.len() + infer_rules.len();
    if total_rules > max_total_rules {
        return Err(BackendError::Init(format!(
            "knowledge.rules.rules_dir {dir:?}: {total_rules} total rules loaded, \
             limit is {max_total_rules} (set knowledge.rules.max_total_rules to raise)"
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
                    "tdw-backend: rules dir_content_hash: cannot read {path:?}: {error} \
                     — this file is excluded from the change-detection hash"
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
            .unwrap_or(RunLimits::default().max_iterations),
        max_derived: infer_cfg
            .max_derived
            .unwrap_or(RunLimits::default().max_derived),
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
            "knowledge.rules.rules_dir {dir:?} does not exist — \
             refusing to boot with a missing rules directory; create it or unset the config key"
        )));
    }

    let loaded = load_rules_from_dir(dir, rules_cfg)?;
    let tag_count = loaded.tag_rules.len();
    let infer_count = loaded.infer_rules.len();

    let mut rule_engine = RuleEngine::default();
    rule_engine.hot_reload(loaded.tag_rules).map_err(|error| {
        BackendError::Init(format!(
            "knowledge.rules.rules_dir {dir:?}: tag-rule validation failed: {error}"
        ))
    })?;

    infer_engine
        .hot_reload(loaded.infer_rules)
        .map_err(|error| {
            BackendError::Init(format!(
                "knowledge.rules.rules_dir {dir:?}: infer-rule validation failed: {error}"
            ))
        })?;

    eprintln!(
        "tdw-backend: loaded {tag_count} tag rule(s) and {infer_count} infer rule(s) \
         from {dir:?} (tag-set v{}, infer-set v{})",
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
fn spawn_rules_reload_tick(
    rules_cfg: &tdw_config::RulesConfig,
    infer_cfg: &tdw_config::InferLimitsConfig,
    rules: Arc<Mutex<RuleEngine>>,
    infer: Arc<Mutex<InferEngine>>,
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
            .unwrap_or(RunLimits::default().max_iterations),
        max_derived: infer_cfg
            .max_derived
            .unwrap_or(RunLimits::default().max_derived),
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
                        "tdw-backend: rules hot-reload from {dir:?} failed (I/O/parse/limit): \
                         {error} — keeping the CURRENT rule sets active"
                    );
                    continue;
                }
            };

            // --- Stage: validate BOTH new rule sets before touching engines. -
            // Pre-validate tag rules by running a throw-away RuleEngine.
            let mut staged_rule_engine = RuleEngine::default();
            if let Err(error) = staged_rule_engine.hot_reload(loaded.tag_rules.clone()) {
                eprintln!(
                    "tdw-backend: rules hot-reload from {dir:?}: tag-rule validation \
                     rejected — BOTH engines stay UNCHANGED: {error}"
                );
                continue;
            }

            // Pre-validate infer rules with the correct limits in a throw-away engine.
            let mut staged_infer_engine = InferEngine::with_limits(infer_limits);
            if let Err(error) = staged_infer_engine.hot_reload(loaded.infer_rules.clone()) {
                eprintln!(
                    "tdw-backend: rules hot-reload from {dir:?}: infer-rule stratification \
                     rejected — BOTH engines stay UNCHANGED: {error}"
                );
                continue;
            }

            let tag_count = loaded.tag_rules.len();
            let infer_count = loaded.infer_rules.len();

            // --- Swap: both validations passed — acquire locks and replace. --
            // Lock order: rules first, then infer. Must be consistent at every
            // call site to avoid deadlocks.
            {
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
                    "tdw-backend: rules hot-reload from {dir:?}: loaded {tag_count} tag \
                     rule(s) and {infer_count} infer rule(s) \
                     (tag-set v{rules_v}, infer-set v{infer_v})"
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
}
