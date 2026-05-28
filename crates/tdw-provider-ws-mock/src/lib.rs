#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures_core::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::{Credentials, DataStream, Error, RegistryEntry, Result, Streamer};
use tdw_domain::{MarketDataBar, TimeGranularity};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EquityTickQuery {
    pub symbol: String,
}

#[derive(Clone, Debug, Default)]
pub struct MockEquityStreamer;

impl MockEquityStreamer {
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::streamer(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Streamer<EquityTickQuery, MarketDataBar> for MockEquityStreamer {
    const PROVIDER: &'static str = "mock-ws";
    const ENDPOINT: &'static str = "equity_ticks";

    async fn subscribe(
        &self,
        query: EquityTickQuery,
        _creds: &Credentials,
    ) -> Result<DataStream<MarketDataBar>> {
        let symbol = normalize_symbol(&query.symbol)?;
        Ok(Box::pin(VecStream::new(snapshot_rows(&symbol))))
    }

    async fn snapshot(
        &self,
        query: &EquityTickQuery,
        _creds: &Credentials,
    ) -> Result<Vec<MarketDataBar>> {
        let symbol = normalize_symbol(&query.symbol)?;
        Ok(snapshot_rows(&symbol))
    }
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.trim().is_empty() {
        return Err(Error::InvalidQuery("empty symbol".to_string()));
    }
    if !symbol
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(Error::InvalidQuery(
            "symbol contains unsupported characters".to_string(),
        ));
    }
    Ok(symbol.to_ascii_uppercase())
}

fn snapshot_rows(symbol: &str) -> Vec<MarketDataBar> {
    vec![MarketDataBar {
        symbol: symbol.to_string(),
        venue: "SIM".to_string(),
        granularity: TimeGranularity::Tick,
        ts: "2026-05-21T20:00:00Z".to_string(),
        open: 100.0,
        high: 100.0,
        low: 100.0,
        close: 100.0,
        volume: 100.0,
        source: "mock-ws".to_string(),
    }]
}

struct VecStream<T> {
    items: VecDeque<Result<T>>,
}

impl<T> VecStream<T> {
    fn new(items: Vec<T>) -> Self {
        Self {
            items: items.into_iter().map(Ok).collect(),
        }
    }
}

impl<T> Unpin for VecStream<T> {}

impl<T> Stream for VecStream<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.pop_front())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    use tdw_core::Credentials;

    #[test]
    fn exposes_mock_streamer_registration() {
        let entry = MockEquityStreamer::registry_entry();

        assert_eq!(entry.provider, "mock-ws");
        assert_eq!(entry.endpoint, "equity_ticks");
    }

    #[test]
    fn snapshot_and_stream_uppercase_symbol_rows() {
        let streamer = MockEquityStreamer;
        let query = EquityTickQuery {
            symbol: " aapl ".to_string(),
        };
        let rows = block_on_ready(streamer.snapshot(&query, &Credentials::default()))
            .unwrap_or_else(|error| panic!("snapshot should succeed: {error}"));
        let mut stream = block_on_ready(streamer.subscribe(query, &Credentials::default()))
            .unwrap_or_else(|error| panic!("subscribe should succeed: {error}"));
        let item = poll_next_ready(stream.as_mut())
            .unwrap_or_else(|| panic!("stream should yield one row"))
            .unwrap_or_else(|error| panic!("stream row should succeed: {error}"));

        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(item.symbol, "AAPL");
        assert!(poll_next_ready(stream.as_mut()).is_none());
        let empty = EquityTickQuery {
            symbol: String::new(),
        };
        assert!(block_on_ready(streamer.snapshot(&empty, &Credentials::default())).is_err());
        let invalid = EquityTickQuery {
            symbol: "AAPL/../../secret".to_string(),
        };
        assert!(block_on_ready(streamer.snapshot(&invalid, &Credentials::default())).is_err());
    }

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future should be ready without an executor"),
        }
    }

    fn poll_next_ready<T>(
        stream: Pin<&mut (dyn Stream<Item = Result<T>> + Send)>,
    ) -> Option<Result<T>> {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        match stream.poll_next(&mut context) {
            Poll::Ready(item) => item,
            Poll::Pending => panic!("test stream should be ready without an executor"),
        }
    }
}
