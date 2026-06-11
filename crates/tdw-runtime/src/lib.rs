#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use serde_json::Value;
use tdw_core::{
    Credentials, DataModel, Fetcher, ProgressOrResult, ProgressStream, ProviderRegistry,
    QueryParams, RegistryEntry, Result,
};

/// A data-fetch runner that dispatches [`Fetcher`] calls and optionally wraps
/// results in a progress stream.
///
/// # G008 theme 10 — `RT1b`: fake-streaming gate
///
/// [`CommandRunner::run_streaming`] fully materialises the fetch result before
/// emitting synthetic `0 %` / `100 %` progress markers — it is **not** real
/// incremental streaming.  Callers that call `run_streaming` without opting in
/// receive [`tdw_core::Error::Provider`] naming the bypass flag, so a
/// misconfigured path that expects real streaming fails loudly rather than
/// silently delivering pre-fetched data.
///
/// To restore the fetch-then-wrap behaviour (e.g. for an integration test that
/// wants to exercise chunk-handling without a real streaming source) call
/// [`CommandRunner::allow_fake_streaming`] on the builder, or set
/// `TDW_ALLOW_FAKE_STREAMING=1` in the environment.  A warning is printed to
/// stderr so the bypass is never invisible.
#[derive(Clone, Debug, Default)]
pub struct CommandRunner {
    registry: ProviderRegistry,
    creds: Credentials,
    /// Explicit opt-in to the fetch-then-wrap fake-streaming path (`RT1b` gate).
    allow_fake_streaming: bool,
}

impl CommandRunner {
    #[must_use]
    pub fn new(registry: ProviderRegistry) -> Self {
        Self {
            registry,
            creds: Credentials::default(),
            allow_fake_streaming: false,
        }
    }

    /// Opt in to the fetch-then-wrap streaming path (`RT1b` gate bypass).
    ///
    /// Use only in tests or in code paths that explicitly accept that no
    /// real incremental streaming will occur.  In production, prefer a real
    /// streaming fetcher implementation instead.
    #[must_use]
    pub const fn allow_fake_streaming(mut self) -> Self {
        self.allow_fake_streaming = true;
        self
    }

    #[must_use]
    pub fn with_credentials(mut self, creds: Credentials) -> Self {
        self.creds = creds;
        self
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn register_provider(&mut self, entry: RegistryEntry) -> Result<()> {
        self.registry.register(entry)
    }

    #[must_use]
    pub fn registered_providers(&self) -> &[RegistryEntry] {
        self.registry.entries()
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn run<F, Q, D>(&self, fetcher: &F, params: Value) -> Result<tdw_core::OBBject<D>>
    where
        F: Fetcher<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        fetcher.fetch(params, &self.creds).await
    }

    /// Fetch via `fetcher` and wrap the result in a synthetic progress stream.
    ///
    /// # G008 theme 10 — `RT1b`: fake-streaming gate
    ///
    /// This method **fully materialises** the fetch result before emitting any
    /// stream items.  The emitted `Progress { fraction: 0.0 }` / `Progress {
    /// fraction: 1.0 }` markers are cosmetic: no data arrives incrementally.
    /// Callers that assume real streaming will receive the whole payload in one
    /// shot, defeating any backpressure or progressive-render logic they apply.
    ///
    /// To make a misconfigured production path fail loudly instead of silently,
    /// this method requires explicit opt-in: construct the runner with
    /// [`CommandRunner::allow_fake_streaming`], or set
    /// `TDW_ALLOW_FAKE_STREAMING=1` in the environment.  Without either, this
    /// returns [`tdw_core::Error::Provider`] naming the bypass flag.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::Provider`] when neither the builder flag nor
    /// `TDW_ALLOW_FAKE_STREAMING=1` is set.  Returns other [`tdw_core::Error`]
    /// variants if the underlying fetch fails.
    pub async fn run_streaming<F, Q, D>(
        &self,
        fetcher: &F,
        params: Value,
    ) -> Result<ProgressStream<D>>
    where
        F: Fetcher<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        // G008/`RT1b`: refuse to silently fake-stream unless the caller opted in.
        let env_allow = std::env::var("TDW_ALLOW_FAKE_STREAMING").as_deref() == Ok("1");
        if !self.allow_fake_streaming && !env_allow {
            return Err(tdw_core::Error::Provider(
                "run_streaming performs a full blocking fetch then wraps the \
                 result in synthetic 0%/100% progress markers — it is not real \
                 incremental streaming. Call .allow_fake_streaming() on the \
                 builder, or set TDW_ALLOW_FAKE_STREAMING=1, to acknowledge \
                 this and enable the fetch-then-wrap path (for tests or \
                 compatibility shims only)."
                    .to_string(),
            ));
        }
        if env_allow && !self.allow_fake_streaming {
            eprintln!(
                "tdw-runtime: TDW_ALLOW_FAKE_STREAMING=1 — run_streaming will \
                 fully materialise the fetch result before emitting progress \
                 markers; no real incremental streaming occurs."
            );
        }

        let object = self.run(fetcher, params).await?;
        Ok(Box::pin(ReadyProgressStream::new(vec![
            Ok(ProgressOrResult::Progress {
                stage: "fetch",
                fraction: 0.0,
                message: Some("started".to_string()),
            }),
            Ok(ProgressOrResult::Progress {
                stage: "fetch",
                fraction: 1.0,
                message: Some("completed".to_string()),
            }),
            Ok(ProgressOrResult::Done(object)),
        ])))
    }
}

struct ReadyProgressStream<T: DataModel> {
    items: VecDeque<Result<ProgressOrResult<T>>>,
}

impl<T: DataModel> ReadyProgressStream<T> {
    fn new(items: Vec<Result<ProgressOrResult<T>>>) -> Self {
        Self {
            items: VecDeque::from(items),
        }
    }
}

impl<T: DataModel> Unpin for ReadyProgressStream<T> {}

impl<T: DataModel> Stream for ReadyProgressStream<T> {
    type Item = Result<ProgressOrResult<T>>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Wake, Waker};

    use async_trait::async_trait;
    use bytes::Bytes;

    #[derive(
        Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    struct Query {
        symbol: String,
    }

    #[derive(
        Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    struct Row {
        symbol: String,
    }

    struct MockFetcher;

    #[async_trait]
    impl Fetcher<Query, Row> for MockFetcher {
        const PROVIDER: &'static str = "mock";
        const ENDPOINT: &'static str = "equity_historical";

        fn transform_query(params: Value) -> Result<Query> {
            let symbol = params
                .get("symbol")
                .and_then(Value::as_str)
                .ok_or_else(|| tdw_core::Error::InvalidQuery("missing symbol".to_string()))?;
            Ok(Query {
                symbol: symbol.to_string(),
            })
        }

        async fn extract_data(&self, query: &Query, _creds: &Credentials) -> Result<Bytes> {
            Ok(Bytes::from(query.symbol.clone()))
        }

        fn transform_data(&self, _query: &Query, raw: Bytes) -> Result<Vec<Row>> {
            let symbol = String::from_utf8(raw.to_vec())
                .map_err(|error| tdw_core::Error::Provider(error.to_string()))?;
            Ok(vec![Row { symbol }])
        }
    }

    #[test]
    fn runner_exposes_explicit_provider_registration() {
        let mut runner = CommandRunner::default();
        assert!(
            runner
                .register_provider(RegistryEntry::fetcher("fileset", "equity_historical"))
                .is_ok()
        );

        assert_eq!(runner.registered_providers().len(), 1);
    }

    // G008/`RT1b`: default runner refuses run_streaming (no opt-in, no env var).
    // Only verifiable when TDW_ALLOW_FAKE_STREAMING is absent from the environment;
    // skip if a developer has set the env var locally. The authoritative env-manipulation
    // tests live in tests/`RT1b`_gate.rs (integration test, allows unsafe env ops).
    #[test]
    fn run_streaming_refuses_without_builder_opt_in() {
        if std::env::var("TDW_ALLOW_FAKE_STREAMING").as_deref() == Ok("1") {
            return; // developer override — skip
        }
        let runner = CommandRunner::default(); // allow_fake_streaming = false
        let fetcher = MockFetcher;
        let result =
            block_on(runner.run_streaming(&fetcher, serde_json::json!({"symbol": "AAPL"})));
        match result {
            Err(tdw_core::Error::Provider(msg)) => {
                assert!(
                    msg.contains("TDW_ALLOW_FAKE_STREAMING"),
                    "error must name the bypass flag, got: {msg}"
                );
            }
            Err(other) => panic!("expected Provider error, got a different error: {other}"),
            Ok(_) => panic!("expected Provider error, got Ok"),
        }
    }

    // G008/`RT1b`: with explicit builder opt-in, run_streaming must succeed.
    #[test]
    fn runner_streaming_wraps_terminal_fetch_result() {
        let runner = CommandRunner::default().allow_fake_streaming();
        let fetcher = MockFetcher;
        let mut stream = block_on(runner.run_streaming(
            &fetcher,
            serde_json::json!({
                "symbol": "AAPL"
            }),
        ))
        .unwrap_or_else(|error| panic!("streaming run failed: {error}"));

        assert!(matches!(
            poll_next(&mut stream),
            Some(Ok(ProgressOrResult::Progress { stage: "fetch", .. }))
        ));
        assert!(matches!(
            poll_next(&mut stream),
            Some(Ok(ProgressOrResult::Progress {
                stage: "fetch",
                fraction: 1.0,
                ..
            }))
        ));
        let done = poll_next(&mut stream);

        match done {
            Some(Ok(ProgressOrResult::Done(object))) => {
                assert_eq!(object.provider, "mock");
                assert_eq!(object.rows[0].symbol, "AAPL");
            }
            other => panic!("unexpected terminal event: {other:?}"),
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);

        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    fn poll_next<T: DataModel>(
        stream: &mut ProgressStream<T>,
    ) -> Option<Result<ProgressOrResult<T>>> {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        match stream.as_mut().poll_next(&mut context) {
            Poll::Ready(item) => item,
            Poll::Pending => None,
        }
    }
}
