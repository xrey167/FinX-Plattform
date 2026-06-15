#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    YahooHttpConsensusFetcher, YahooHttpDividendsFetcher, YahooHttpEquityHistoricalFetcher,
    YahooHttpEtfInfoFetcher, YahooHttpFuturesCurveFetcher, YahooHttpFuturesHistoricalFetcher,
    YahooHttpOptionsChainFetcher, YahooHttpPredefinedScreenerFetcher,
    YahooHttpPricePerformanceFetcher, YahooHttpProfileFetcher, YahooHttpQuoteFetcher,
    YahooHttpShareStatisticsFetcher,
};

use async_trait::async_trait;
use bytes::Bytes;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;
use tdw_provider_fileset::EquityHistoricalQuery;

/// Canonical Yahoo provider id, shared by every fetcher in the crate.
pub const PROVIDER_ID: &str = "yahoo";
/// Base host for Yahoo's query API (v7/v8/v10 JSON endpoints).
pub const BASE_URL: &str = "https://query1.finance.yahoo.com";

/// A symbol-only query shared by the single-symbol Yahoo expansion fetchers
/// (profile, quote, performance, dividends, share statistics, consensus,
/// futures, options).
///
/// Symbol normalization mirrors the historical fetcher's
/// (`tdw_provider_fileset`) rules — uppercase, ASCII-alphanumeric plus
/// `. - _` — and additionally permits the `=` and `^` characters Yahoo uses for
/// continuous futures (`ES=F`), index (`^GSPC`), and contract symbols.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct YahooSymbolQuery {
    /// Normalized ticker / contract symbol (e.g. `AAPL`, `ES=F`).
    pub symbol: String,
}

impl YahooSymbolQuery {
    /// Parse a `{ "symbol": "..." }` payload, normalizing the symbol.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when `symbol` is missing, not a string,
    /// empty, or contains characters outside the accepted set.
    pub fn from_value(params: &Value) -> Result<Self> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("yahoo symbol must be a string".to_string()))?;
        Ok(Self {
            symbol: normalize_yahoo_symbol(symbol)?,
        })
    }
}

/// Normalize a Yahoo symbol: trim, reject empty, accept ASCII-alphanumeric plus
/// `. - _ = ^`, and uppercase. The `=`/`^` extensions cover Yahoo's futures and
/// index symbology that the equity-only fileset normalizer rejects.
fn normalize_yahoo_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(Error::InvalidQuery("empty symbol".to_string()));
    }
    if !symbol.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '=' | '^')
    }) {
        return Err(Error::InvalidQuery(
            "symbol contains unsupported characters".to_string(),
        ));
    }
    Ok(symbol.to_ascii_uppercase())
}

/// Query for a Yahoo predefined (saved) discovery screen.
///
/// `scr_ids` is the Yahoo predefined-screener id (e.g. `aggressive_small_caps`),
/// injected per dispatch binding; `count` caps the returned row count. The id is
/// validated against the known character set so it cannot inject extra query
/// parameters into the screener URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct YahooScreenerQuery {
    /// Yahoo predefined-screener id.
    pub scr_ids: String,
    /// Maximum number of rows to request.
    pub count: u32,
}

impl YahooScreenerQuery {
    /// Default page size when the caller does not specify a `count`/`limit`.
    const DEFAULT_COUNT: u32 = 25;

    /// Parse a `{ "scr_ids": "...", "count": N }` payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when `scr_ids` is missing, not a string,
    /// empty, or contains characters outside `[A-Za-z0-9_]`.
    pub fn from_value(params: &Value) -> Result<Self> {
        let scr_ids = params
            .get("scr_ids")
            .or_else(|| params.get("scrIds"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("yahoo scr_ids must be a string".to_string()))?
            .trim();
        if scr_ids.is_empty() {
            return Err(Error::InvalidQuery(
                "yahoo scr_ids must not be empty".to_string(),
            ));
        }
        if !scr_ids
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(Error::InvalidQuery(
                "yahoo scr_ids contains unsupported characters".to_string(),
            ));
        }
        let count = params
            .get("count")
            .or_else(|| params.get("limit"))
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .unwrap_or(Self::DEFAULT_COUNT);
        Ok(Self {
            scr_ids: scr_ids.to_string(),
            count,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct YahooEquityHistoricalFetcher;

impl YahooEquityHistoricalFetcher {
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
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
    fn symbol_query_accepts_equities_and_futures_and_rejects_garbage() {
        let equity = YahooSymbolQuery::from_value(&serde_json::json!({ "symbol": "aapl" }))
            .unwrap_or_else(|error| panic!("equity symbol: {error}"));
        assert_eq!(equity.symbol, "AAPL");

        let future = YahooSymbolQuery::from_value(&serde_json::json!({ "symbol": "ES=F" }))
            .unwrap_or_else(|error| panic!("future symbol: {error}"));
        assert_eq!(future.symbol, "ES=F");

        let index = YahooSymbolQuery::from_value(&serde_json::json!({ "symbol": "^GSPC" }))
            .unwrap_or_else(|error| panic!("index symbol: {error}"));
        assert_eq!(index.symbol, "^GSPC");

        assert!(YahooSymbolQuery::from_value(&serde_json::json!({ "symbol": "A B" })).is_err());
        assert!(YahooSymbolQuery::from_value(&serde_json::json!({ "symbol": "" })).is_err());
        assert!(YahooSymbolQuery::from_value(&serde_json::json!({})).is_err());
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact representable value, not an approximate result: 202.0 is a fixed close from the static test fixture decoded verbatim
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
