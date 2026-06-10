//! Async data/daemon facade.
//!
//! Owns the daemon composition root ([`AppState`]) and a [`CommandRunner`] for
//! provider dispatch, and exposes the async query/ingest surface over them. This
//! crate holds **no business logic**: every method is a thin, typed delegation
//! to the underlying `tdw-*` crates.

use std::sync::Arc;

use serde_json::Value;
use tdw_agent::{ConsolidationAction, Memory};
use tdw_agent_store::{MemoryStore, consolidate_at, spawn_consolidation_scheduler};
use tdw_app_server::{CancellationToken, SubmissionHandle};
use tdw_bus::EventBus;
use tdw_config::TdwConfig;
use tdw_core::{
    BlobEngine, DataModel, Fetcher, LexicalEngine, OBBject, OlapEngine, ProgressStream,
    ProviderRegistry, QueryParams, RelationalEngine, VectorEngine,
};
use tdw_domain::EquityHistoricalData;
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::{KnowledgeDocument, KnowledgeHit, KnowledgeIndex};
use tdw_outbox::InMemoryOutbox;
use tdw_protocol::{EventMsg, OpEnvelope};
use tdw_runtime::CommandRunner;
use tdw_service_api::{AppState, fetch_equity_historical};
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
    /// Cancels the service loop, relay, and transport on shutdown.
    cancel: CancellationToken,
    /// In-process op submission into the running service loop (no socket).
    submission: SubmissionHandle,
    /// The `serve(service_loop, relay, ..)` driver task.
    serve_task: tokio::task::JoinHandle<()>,
    /// The periodic memory-consolidation scheduler task (Phase B).
    consolidation_task: tokio::task::JoinHandle<()>,
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
    /// The async knowledge index. Held behind a [`tokio::sync::Mutex`] because
    /// [`KnowledgeIndex::index_document`] takes `&mut self` and is async; the
    /// guard is acquired per-call and never held across unrelated awaits.
    index: Arc<Mutex<KnowledgeIndex>>,
    /// The agent memory store (Phase B). Held behind a [`tokio::sync::Mutex`] so
    /// the live consolidation scheduler and the [`upsert_memory`](Self::upsert_memory)
    /// / [`consolidate_now`](Self::consolidate_now) surface methods share one store.
    /// Loaded from `TDW_MEMORY_DIR` when set (round-tripping to `*.json5`), else
    /// purely in-memory.
    memory: Arc<Mutex<MemoryStore>>,
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
        let state = AppState::from_config(config)
            .await
            .map_err(|error| BackendError::Init(error.to_string()))?;
        let runner = CommandRunner::default();
        let index = Arc::new(Mutex::new(KnowledgeIndex::new(
            select_embedder()?,
            state.vector.clone(),
        )));
        Ok(Self {
            state,
            runner,
            index,
            memory: Arc::new(Mutex::new(build_memory_store())),
            daemon: None,
        })
    }

    /// Build a backend backed by deterministic in-memory engines, for tests.
    pub async fn in_memory_for_tests() -> Self {
        let state = AppState::in_memory_for_tests().await;
        let runner = CommandRunner::default();
        let index = Arc::new(Mutex::new(KnowledgeIndex::new(
            Arc::new(HashEmbeddingProvider::default()),
            state.vector.clone(),
        )));
        Self {
            state,
            runner,
            index,
            memory: Arc::new(Mutex::new(build_memory_store())),
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

        self.daemon = Some(DaemonHandle {
            cancel,
            submission: handle,
            serve_task,
            consolidation_task,
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

    /// Run one consolidation pass over the store at the current time, applying and
    /// persisting tier changes, and return the actions applied.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Memory`] if persisting a promotion or deleting an
    /// expired memory's file fails.
    pub async fn consolidate_now(&self) -> BackendResult<Vec<ConsolidationAction>> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut store = self.memory.lock().await;
        consolidate_at(&mut store, &now).map_err(|error| BackendError::Memory(error.to_string()))
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

    // --- Knowledge index (async, behind a per-call mutex) ------------------

    /// Index a [`KnowledgeDocument`] effective `now` (a `YYYY-MM-DD` date) into
    /// the embedded knowledge index — the deterministic, injected-clock seam
    /// (knowledge-system B3).
    ///
    /// The index mutex is acquired, the single async `index_document_at` call is
    /// awaited, and the guard is dropped — it is never held across unrelated
    /// awaits.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if the document is invalid or an
    /// embedding/storage/tag step fails.
    pub async fn knowledge_index_at(&self, doc: KnowledgeDocument, now: &str) -> BackendResult<()> {
        let mut index = self.index.lock().await;
        index.index_document_at(doc, now).await?;
        drop(index);
        Ok(())
    }

    /// Wall-clock convenience over [`Backend::knowledge_index_at`]: stamps tag
    /// assignments with today's UTC date. Only this live edge reads the clock,
    /// mirroring the consolidation-scheduler precedent.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if the document is invalid or an
    /// embedding/storage/tag step fails.
    pub async fn knowledge_index(&self, doc: KnowledgeDocument) -> BackendResult<()> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        self.knowledge_index_at(doc, &today).await
    }

    /// Batch-index documents effective `now` (knowledge-system B5). One
    /// `embed_batch` round-trip embeds the whole batch (native batch
    /// endpoints on API embedders), then each document indexes through the
    /// same write path as [`Backend::knowledge_index_at`] — the BARE index
    /// path (vector + in-process graph/tags). Rules, lexical co-index,
    /// durable-graph stamping, and manifest idempotency live on
    /// `tdw_knowledge::indexer::KnowledgeIndexer`, which is not yet hosted by
    /// the daemon (wiring planned with the B8 knowledge runtime). Validation
    /// is all-or-nothing up front; the index mutex is held for the duration
    /// of the batch, so [`Backend::knowledge_search`] never observes a
    /// half-applied batch.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Knowledge`] if any document is invalid or an
    /// embedding/storage/tag step fails.
    pub async fn knowledge_ingest_at(
        &self,
        docs: Vec<KnowledgeDocument>,
        now: &str,
    ) -> BackendResult<()> {
        let mut index = self.index.lock().await;
        index.index_documents_at(docs, now).await?;
        drop(index);
        Ok(())
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
        let index = self.index.lock().await;
        Ok(index.search(query, top_k).await?)
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
/// Select the embedder for the shared knowledge index from `TDW_EMBED_PROVIDER`.
///
/// Default / unset / `hash` / `local` → the deterministic offline
/// [`HashEmbeddingProvider`]. `openai` / `google` build the real HTTP embedder
/// when the matching feature is compiled and an API key is present; otherwise a
/// **warning is logged** and the hash embedder is used (the daemon still boots
/// rather than silently degrading without a trace). The real arms are
/// `#[cfg]`-gated so the default build never compiles reqwest.
///
/// # Errors
///
/// Returns [`BackendError::Init`] if a real provider is requested, its key is
/// present, but the client cannot be constructed (e.g. an invalid base URL).
// In the default (no-feature) build only the hash/unknown arms remain, all `Ok`,
// so the `Result` looks unwrappable there — keep the allow.
#[allow(clippy::unnecessary_wraps)]
fn select_embedder() -> BackendResult<Arc<dyn EmbeddingProvider>> {
    let provider = std::env::var("TDW_EMBED_PROVIDER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match provider.as_deref() {
        None | Some("hash" | "local") => Ok(Arc::new(HashEmbeddingProvider::default())),
        #[cfg(feature = "openai")]
        Some("openai") => build_openai_embedder(),
        #[cfg(feature = "google")]
        Some("google") => build_google_embedder(),
        Some(other) => {
            eprintln!(
                "tdw-backend: TDW_EMBED_PROVIDER={other} is unavailable in this build \
                 (is the matching feature compiled?); using the offline hash embedder"
            );
            Ok(Arc::new(HashEmbeddingProvider::default()))
        }
    }
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
        eprintln!(
            "tdw-backend: TDW_EMBED_PROVIDER=openai but no API key \
             (TDW_OPENAI_EMBEDDING_API_KEY / OPENAI_API_KEY); using the offline hash embedder"
        );
        return Ok(Arc::new(HashEmbeddingProvider::default()));
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
        eprintln!(
            "tdw-backend: TDW_EMBED_PROVIDER=google but no API key \
             (TDW_GOOGLE_EMBEDDING_API_KEY / GOOGLE_API_KEY / GEMINI_API_KEY); \
             using the offline hash embedder"
        );
        return Ok(Arc::new(HashEmbeddingProvider::default()));
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
