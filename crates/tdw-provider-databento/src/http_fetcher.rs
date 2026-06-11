//! Real Databento HTTP backends for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Databento's historical API
//! via `reqwest`. All calls use HTTP Basic auth where the username is
//! `TDW_DATABENTO_API_KEY` and the password is empty.
//!
//! Live integration tests are additionally gated by `TDW_DATABENTO_LIVE=1`
//! so unattended CI stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_domain::{MarketDataBar, Ohlcv, TimeGranularity};

use crate::{API_KEY_ENV, BASE_URL, DatabentoTimeseriesQuery};

const USER_AGENT: &str = "tdw-provider-databento/0.1";

// ---------------------------------------------------------------------------
// Timeseries fetcher  (`/timeseries.get_range`)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Databento timeseries OHLCV fetcher.
    pub DatabentoHttpTimeseriesFetcher,
    BASE_URL
);

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
    #[serde(flatten)]
    ohlcv: Ohlcv,
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
            .ok_or_else(|| Error::InvalidQuery("databento dataset must be a string".to_string()))?;
        let symbols: Vec<String> = params
            .get("symbols")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidQuery("databento symbols must be an array".to_string()))?
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
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "databento")?;

        let url = format!(
            "{}/timeseries.get_range",
            self.base_url().trim_end_matches('/')
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
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
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
                source: "databento".to_string(),
                ..record.ohlcv.into_bar_template()
            });
        }
        Ok(bars)
    }
}

// ---------------------------------------------------------------------------
// Metadata fetcher  (`GET /metadata.list_datasets`)
// ---------------------------------------------------------------------------

/// Query type for the metadata endpoint.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct DatabentoMetadataQuery {
    /// Opaque marker; the datasets endpoint takes no real parameters.
    pub _placeholder: Option<String>,
}

/// Response row: a single dataset identifier string.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct DatabentoDataset {
    pub id: String,
}

tdw_core::provider_fetcher_struct!(
    /// Production fetcher for `GET /metadata.list_datasets`.
    pub DatabentoMetadataFetcher,
    BASE_URL
);

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
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "databento")?;

        let url = format!(
            "{}/metadata.list_datasets",
            self.base_url().trim_end_matches('/')
        );

        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
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
    tdw_core::date::unix_seconds_to_iso_timestamp(seconds)
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
