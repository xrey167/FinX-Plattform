//! Real Databento HTTP backends for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Databento's historical API
//! via `reqwest`. All calls use HTTP Basic auth where the username is
//! `TDW_DATABENTO_API_KEY` and the password is empty.
//!
//! Live integration tests are additionally gated by `TDW_DATABENTO_LIVE=1`
//! so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{API_KEY_ENV, BASE_URL, DatabentoTimeseriesQuery};

const USER_AGENT: &str = "tdw-provider-databento/0.1";

// ---------------------------------------------------------------------------
// Timeseries fetcher  (`/timeseries.get_range`)
// ---------------------------------------------------------------------------

/// Production Databento timeseries OHLCV fetcher.
#[derive(Clone, Debug)]
pub struct DatabentoHttpTimeseriesFetcher {
    base_url: String,
}

impl Default for DatabentoHttpTimeseriesFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl DatabentoHttpTimeseriesFetcher {
    /// Override the Databento base URL (useful for testing).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `databento` / `timeseries` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

// --- Databento API response types ------------------------------------------

#[derive(Deserialize)]
struct TimeseriesResponse {
    #[serde(default)]
    records: Vec<OhlcvRecord>,
}

#[derive(Deserialize)]
struct OhlcvRecord {
    /// Nanosecond Unix timestamp.
    ts_event: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

// ---------------------------------------------------------------------------

#[async_trait]
impl Fetcher<DatabentoTimeseriesQuery, MarketDataBar> for DatabentoHttpTimeseriesFetcher {
    const PROVIDER: &'static str = "databento";
    const ENDPOINT: &'static str = "timeseries";

    fn transform_query(params: Value) -> Result<DatabentoTimeseriesQuery> {
        let dataset = params
            .get("dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("databento dataset must be a string".to_string())
            })?;
        let symbols: Vec<String> = params
            .get("symbols")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::InvalidQuery("databento symbols must be an array".to_string())
            })?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| Error::InvalidQuery("each symbol must be a string".to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        let start = params
            .get("start")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("databento start must be YYYY-MM-DD".to_string()))?;
        let end = params
            .get("end")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("databento end must be YYYY-MM-DD".to_string()))?;

        DatabentoTimeseriesQuery::new(dataset, symbols, start, end)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &DatabentoTimeseriesQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = std::env::var(API_KEY_ENV)
            .map_err(|_| Error::Provider(format!("{API_KEY_ENV} not set")))?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(Error::Provider(format!("{API_KEY_ENV} must not be empty")));
        }

        let url = format!(
            "{}/timeseries.get_range",
            self.base_url.trim_end_matches('/')
        );
        let body = serde_json::json!({
            "dataset": query.dataset,
            "symbols": query.symbols,
            "schema": "ohlcv-1d",
            "start": query.start,
            "end": query.end,
            "encoding": "json",
        });

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("databento client build: {e}")))?;

        let response = client
            .post(&url)
            // Databento Basic auth: username = api_key, password = ""
            .basic_auth(&api_key, Some(""))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("databento timeseries request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable body>"));
            return Err(Error::Provider(format!(
                "databento timeseries returned {status}: {text}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("databento read timeseries body: {e}")))
    }

    fn transform_data(
        &self,
        query: &DatabentoTimeseriesQuery,
        raw: Bytes,
    ) -> Result<Vec<MarketDataBar>> {
        let envelope: TimeseriesResponse = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("databento parse timeseries json: {e}")))?;

        // Use the first symbol as the label when there is exactly one.
        let symbol_label = if query.symbols.len() == 1 {
            query.symbols[0].clone()
        } else {
            query.symbols.join(",")
        };

        let mut bars = Vec::with_capacity(envelope.records.len());
        for record in envelope.records {
            bars.push(MarketDataBar {
                symbol: symbol_label.clone(),
                venue: query.dataset.clone(),
                granularity: TimeGranularity::Day,
                ts: unix_nanos_to_iso_timestamp(record.ts_event),
                open: record.open,
                high: record.high,
                low: record.low,
                close: record.close,
                volume: record.volume,
                source: "databento".to_string(),
            });
        }
        Ok(bars)
    }
}

// ---------------------------------------------------------------------------
// Metadata fetcher  (`GET /metadata.list_datasets`)
// ---------------------------------------------------------------------------

/// Query type for the metadata endpoint.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct DatabentoMetadataQuery {
    /// Opaque marker; the datasets endpoint takes no real parameters.
    pub _placeholder: Option<String>,
}

impl Default for DatabentoMetadataQuery {
    fn default() -> Self {
        Self { _placeholder: None }
    }
}

/// Response row: a single dataset identifier string.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct DatabentoDataset {
    pub id: String,
}

/// Production fetcher for `GET /metadata.list_datasets`.
#[derive(Clone, Debug)]
pub struct DatabentoMetadataFetcher {
    base_url: String,
}

impl Default for DatabentoMetadataFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl DatabentoMetadataFetcher {
    /// Override the base URL (useful for testing).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `databento` / `metadata` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[derive(Deserialize)]
struct MetadataResponse {
    #[serde(default)]
    result: Vec<String>,
}

#[async_trait]
impl Fetcher<DatabentoMetadataQuery, DatabentoDataset> for DatabentoMetadataFetcher {
    const PROVIDER: &'static str = "databento";
    const ENDPOINT: &'static str = "metadata";

    fn transform_query(_params: Value) -> Result<DatabentoMetadataQuery> {
        Ok(DatabentoMetadataQuery::default())
    }

    async fn extract_data(
        &self,
        _query: &DatabentoMetadataQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = std::env::var(API_KEY_ENV)
            .map_err(|_| Error::Provider(format!("{API_KEY_ENV} not set")))?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(Error::Provider(format!("{API_KEY_ENV} must not be empty")));
        }

        let url = format!(
            "{}/metadata.list_datasets",
            self.base_url.trim_end_matches('/')
        );

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("databento metadata client build: {e}")))?;

        let response = client
            .get(&url)
            .basic_auth(&api_key, Some(""))
            .send()
            .await
            .map_err(|e| Error::Provider(format!("databento metadata request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable body>"));
            return Err(Error::Provider(format!(
                "databento metadata returned {status}: {text}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("databento read metadata body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &DatabentoMetadataQuery,
        raw: Bytes,
    ) -> Result<Vec<DatabentoDataset>> {
        let envelope: MetadataResponse = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("databento parse metadata json: {e}")))?;
        Ok(envelope
            .result
            .into_iter()
            .map(|id| DatabentoDataset { id })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Timestamp conversion
// ---------------------------------------------------------------------------

/// Convert a nanosecond Unix timestamp to an ISO-8601 UTC string.
fn unix_nanos_to_iso_timestamp(nanos: i64) -> String {
    let seconds = nanos.div_euclid(1_000_000_000);
    let days_since_epoch = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_nanos_to_iso_timestamp_epoch() {
        assert_eq!(unix_nanos_to_iso_timestamp(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn unix_nanos_to_iso_timestamp_known_date() {
        // 2024-01-02 00:00:00 UTC = 1_704_153_600 seconds
        let nanos = 1_704_153_600i64 * 1_000_000_000;
        assert_eq!(unix_nanos_to_iso_timestamp(nanos), "2024-01-02T00:00:00Z");
    }
}
