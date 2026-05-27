#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::YahooHttpEquityHistoricalFetcher;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;
use tdw_provider_fileset::EquityHistoricalQuery;

#[derive(Clone, Debug, Default)]
pub struct YahooEquityHistoricalFetcher;

impl YahooEquityHistoricalFetcher {
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<EquityHistoricalQuery, EquityHistoricalData> for YahooEquityHistoricalFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<EquityHistoricalQuery> {
        tdw_provider_fileset::FilesetEquityHistoricalFetcher::transform_query(params)
    }

    async fn extract_data(
        &self,
        query: &EquityHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let rows = vec![EquityHistoricalData {
            symbol: query.symbol.clone(),
            date: "2026-05-21".to_string(),
            open: 201.0,
            high: 203.0,
            low: 200.0,
            close: 202.0,
            volume: 21_000,
        }];
        serde_json::to_vec(&rows)
            .map(Bytes::from)
            .map_err(|error| Error::Provider(error.to_string()))
    }

    fn transform_data(
        &self,
        _query: &EquityHistoricalQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        serde_json::from_slice(&raw).map_err(|error| Error::Provider(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[test]
    fn exposes_yahoo_fetcher_registration() {
        let entry = YahooEquityHistoricalFetcher::registry_entry();

        assert_eq!(entry.provider, "yahoo");
        assert_eq!(entry.endpoint, "equity_historical");
    }

    #[test]
    fn fetcher_transforms_query_extracts_and_decodes_rows() {
        let fetcher = YahooEquityHistoricalFetcher;
        let query = YahooEquityHistoricalFetcher::transform_query(serde_json::json!({
            "symbol": "AAPL"
        }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
        let raw = block_on_ready(fetcher.extract_data(&query, &Credentials::default()))
            .unwrap_or_else(|error| panic!("extract should succeed: {error}"));
        let rows = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|error| panic!("rows should decode: {error}"));

        assert_eq!(query.symbol, "AAPL");
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].close, 202.0);
        assert!(
            YahooEquityHistoricalFetcher::transform_query(serde_json::json!({
                "symbol": "AAPL?range=1y"
            }))
            .is_err()
        );
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
}
