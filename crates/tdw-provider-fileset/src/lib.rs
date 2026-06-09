#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use async_trait::async_trait;
use bytes::Bytes;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::query_params::StandardParams;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EquityHistoricalQuery {
    pub symbol: String,
    /// Shared date/interval/period/limit normalization parsed from the same
    /// caller payload as `symbol`. Defaults apply when the caller omits them.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

#[derive(Clone, Debug, Default)]
pub struct FilesetEquityHistoricalFetcher;

impl FilesetEquityHistoricalFetcher {
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<EquityHistoricalQuery, EquityHistoricalData> for FilesetEquityHistoricalFetcher {
    const PROVIDER: &'static str = "fileset";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<EquityHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
        Ok(EquityHistoricalQuery {
            symbol: normalize_symbol(symbol)?,
            params: StandardParams::from_value(&params)?,
        })
    }

    async fn extract_data(
        &self,
        query: &EquityHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let rows = fixture_rows(&query.symbol);
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

#[must_use]
pub fn fixture_rows(symbol: &str) -> Vec<EquityHistoricalData> {
    vec![
        EquityHistoricalData {
            symbol: symbol.to_string(),
            date: "2026-05-20".to_string(),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 10_000,
        },
        EquityHistoricalData {
            symbol: symbol.to_string(),
            date: "2026-05-21".to_string(),
            open: 101.0,
            high: 103.0,
            low: 100.0,
            close: 102.0,
            volume: 12_000,
        },
    ]
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_fileset_fetcher_registration() {
        let entry = FilesetEquityHistoricalFetcher::registry_entry();

        assert_eq!(entry.provider, "fileset");
        assert_eq!(entry.endpoint, "equity_historical");
    }

    #[test]
    fn rejects_empty_symbol_queries() {
        assert!(
            FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({ "symbol": "" }))
                .is_err()
        );
        assert!(
            FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({
                "symbol": "AAPL/../../secret"
            }))
            .is_err()
        );
        let query = FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({
            "symbol": " aapl "
        }))
        .unwrap_or_else(|error| panic!("query should normalize: {error}"));
        assert_eq!(query.symbol, "AAPL");
        // Absent standard params fall back to the shared defaults.
        assert_eq!(query.params, tdw_core::StandardParams::default());
    }

    #[test]
    fn transform_query_parses_shared_standard_params() {
        let query = FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({
            "symbol": "msft",
            "start_date": "2026-01-01",
            "end_date": "2026-02-01",
            "interval": "1h",
            "limit": 50
        }))
        .unwrap_or_else(|error| panic!("query should normalize: {error}"));

        assert_eq!(query.symbol, "MSFT");
        assert_eq!(query.params.interval, tdw_core::Interval::OneHour);
        assert_eq!(
            query.params.start_date.map(|d| d.to_string()).as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(query.params.limit, Some(50));
        // Shared cross-field validation rejects an inverted date range.
        assert!(
            FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({
                "symbol": "MSFT",
                "start_date": "2026-02-01",
                "end_date": "2026-01-01"
            }))
            .is_err()
        );
    }
}
