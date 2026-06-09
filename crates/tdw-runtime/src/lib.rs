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

#[derive(Clone, Debug, Default)]
pub struct CommandRunner {
    registry: ProviderRegistry,
    creds: Credentials,
}

impl CommandRunner {
    #[must_use]
    pub fn new(registry: ProviderRegistry) -> Self {
        Self {
            registry,
            creds: Credentials::default(),
        }
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

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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

    #[test]
    fn runner_streaming_wraps_terminal_fetch_result() {
        let runner = CommandRunner::default();
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
