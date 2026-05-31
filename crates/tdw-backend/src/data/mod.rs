//! Async data/daemon facade.
//!
//! Owns the daemon composition root ([`AppState`]) and a [`CommandRunner`] for
//! provider dispatch, and exposes the async query/ingest surface over them. This
//! crate holds **no business logic**: every method is a thin, typed delegation
//! to the underlying `tdw-*` crates.

use std::sync::Arc;

use serde_json::Value;
use tdw_config::TdwConfig;
use tdw_core::{
    BlobEngine, DataModel, Fetcher, LexicalEngine, OBBject, OlapEngine, ProgressStream,
    ProviderRegistry, QueryParams, RelationalEngine, VectorEngine,
};
use tdw_domain::EquityHistoricalData;
use tdw_protocol::{EventMsg, OpEnvelope};
use tdw_runtime::CommandRunner;
use tdw_service_api::{AppState, fetch_equity_historical};

use crate::error::{BackendError, BackendResult};

/// The async backend facade over the data/daemon surface.
pub struct Backend {
    state: AppState,
    runner: CommandRunner,
}

impl Backend {
    /// Build a backend from a layered [`TdwConfig`].
    ///
    /// The provider [`CommandRunner`] is sourced from the composition root's
    /// registry (`AppState::registry`) so typed [`fetch`](Self::fetch) /
    /// [`stream`](Self::stream) calls dispatch against the same real providers
    /// the daemon serves, not an empty default registry.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the daemon composition root cannot be
    /// constructed from `config`.
    pub async fn from_config(config: TdwConfig) -> BackendResult<Self> {
        let state = AppState::from_config(config)
            .await
            .map_err(|error| BackendError::Init(error.to_string()))?;
        let runner = CommandRunner::new((*state.registry).clone());
        Ok(Self { state, runner })
    }

    /// Build a backend backed by deterministic in-memory engines, for tests.
    ///
    /// The runner is wired to the in-memory `AppState`'s registry, exactly as in
    /// [`from_config`](Self::from_config), so tests exercise the real fetch path.
    pub async fn in_memory_for_tests() -> Self {
        let state = AppState::in_memory_for_tests().await;
        let runner = CommandRunner::new((*state.registry).clone());
        Self { state, runner }
    }

    /// The underlying daemon composition root.
    #[must_use]
    pub fn app_state(&self) -> &AppState {
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
        assert!(Arc::ptr_eq(&backend.registry(), &backend.app_state().registry));
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
            .fetch(&FilesetEquityHistoricalFetcher, serde_json::json!({ "symbol": "aapl" }))
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
            .stream(&FilesetEquityHistoricalFetcher, serde_json::json!({ "symbol": "aapl" }))
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
