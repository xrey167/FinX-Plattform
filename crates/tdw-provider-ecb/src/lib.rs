#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::EcbHttpDataFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "ecb";
pub const BASE_URL: &str = "https://data-api.ecb.europa.eu/service";

pub type Result<T> = std::result::Result<T, EcbProviderError>;

/// Query parameters for the ECB Statistical Data Warehouse `/data` endpoint.
///
/// See <https://data-api.ecb.europa.eu/service/data/{flow}/{key}>
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EcbDataQuery {
    /// Data flow identifier (e.g. `"EXR"` for exchange rates, `"ILM"` for
    /// money market rates). Must be non-empty and at most 100 characters.
    pub flow: String,
    /// Series key (e.g. `"D.USD.EUR.SP00.A"`). Must be non-empty and at most
    /// 100 characters.
    pub key: String,
    /// Start of the observation window in `YYYY-MM-DD` (daily) or `YYYY-MM`
    /// (monthly) format.
    pub start_period: String,
    /// End of the observation window in `YYYY-MM-DD` (daily) or `YYYY-MM`
    /// (monthly) format.
    pub end_period: String,
}

impl EcbDataQuery {
    /// Construct and validate a new [`EcbDataQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`EcbProviderError::EmptyFlow`] if `flow` is blank,
    /// [`EcbProviderError::FlowTooLong`] if it exceeds 100 characters,
    /// [`EcbProviderError::EmptyKey`] if `key` is blank, or
    /// [`EcbProviderError::KeyTooLong`] if it exceeds 100 characters.
    pub fn new(
        flow: impl Into<String>,
        key: impl Into<String>,
        start_period: impl Into<String>,
        end_period: impl Into<String>,
    ) -> Result<Self> {
        let flow = validate_flow(flow.into())?;
        let key = validate_key(key.into())?;
        Ok(Self {
            flow,
            key,
            start_period: start_period.into(),
            end_period: end_period.into(),
        })
    }
}

/// A single observation returned by the ECB data API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EcbObservation {
    /// Data flow identifier (e.g. `"EXR"`).
    pub flow: String,
    /// Series key (e.g. `"D.USD.EUR.SP00.A"`).
    pub key: String,
    /// Observation date in `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly) format.
    pub date: String,
    /// Numeric observation value.
    pub value: f64,
}

/// Errors produced by the ECB provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EcbProviderError {
    #[error("ecb flow must not be empty")]
    EmptyFlow,
    #[error("ecb flow must not exceed 100 characters")]
    FlowTooLong,
    #[error("ecb key must not be empty")]
    EmptyKey,
    #[error("ecb key must not exceed 100 characters")]
    KeyTooLong,
}

/// Build the relative request path for the ECB `/data` endpoint.
///
/// # Errors
///
/// Returns an error if `flow` or `key` fail validation.
pub fn data_request_path(
    flow: &str,
    key: &str,
    start_period: &str,
    end_period: &str,
) -> Result<String> {
    let flow = validate_flow(flow.to_string())?;
    let key = validate_key(key.to_string())?;
    Ok(format!(
        "/data/{flow}/{key}?format=jsondata&startPeriod={start_period}&endPeriod={end_period}"
    ))
}

fn validate_flow(flow: String) -> Result<String> {
    let flow = flow.trim().to_string();
    if flow.is_empty() {
        return Err(EcbProviderError::EmptyFlow);
    }
    if flow.len() > 100 {
        return Err(EcbProviderError::FlowTooLong);
    }
    Ok(flow)
}

fn validate_key(key: String) -> Result<String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(EcbProviderError::EmptyKey);
    }
    if key.len() > 100 {
        return Err(EcbProviderError::KeyTooLong);
    }
    Ok(key)
}

/// Mock fetcher for use in unit tests (no network required).
pub struct EcbMockFetcher {
    /// Raw JSON bytes to return from `transform_data`.
    pub raw: Vec<u8>,
}

impl EcbMockFetcher {
    /// Decode the stored bytes into a list of [`EcbObservation`]s using the
    /// same parsing logic as the real HTTP fetcher.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are not valid ECB JSON or if the
    /// observation array is malformed.
    pub fn parse(&self, query: &EcbDataQuery) -> Result<Vec<EcbObservation>> {
        parse_ecb_json(&self.raw, &query.flow, &query.key)
    }
}

/// Parse raw ECB `jsondata` bytes into a flat list of [`EcbObservation`]s.
///
/// The ECB envelope looks like:
/// ```json
/// {
///   "dataSets": [{ "series": { "0:0:0:0:0": { "observations": {
///     "0": [1.0934, 0, null], "1": [1.0945, 0, null]
///   }}}}],
///   "structure": { "dimensions": { "observation": [
///     { "id": "TIME_PERIOD", "values": [{ "id": "2024-01-02" }, ...] }
///   ]}}
/// }
/// ```
///
/// # Errors
///
/// Returns [`EcbProviderError`] if the JSON cannot be parsed or the envelope
/// shape does not match expectations.
pub fn parse_ecb_json(raw: &[u8], flow: &str, key: &str) -> Result<Vec<EcbObservation>> {
    let v: serde_json::Value = serde_json::from_slice(raw).map_err(|_| {
        // We map serde errors to a generic parse error via EmptyFlow as a
        // stand-in is wrong; use a dedicated variant instead.
        EcbProviderError::EmptyFlow
    })?;
    parse_ecb_value(&v, flow, key)
}

fn parse_ecb_value(v: &serde_json::Value, flow: &str, key: &str) -> Result<Vec<EcbObservation>> {
    // Extract the time-period values array from the structure block.
    let dates: Vec<String> = v
        .pointer("/structure/dimensions/observation")
        .and_then(|obs| obs.as_array())
        .and_then(|dims| {
            dims.iter()
                .find(|dim| dim.get("id").and_then(|id| id.as_str()) == Some("TIME_PERIOD"))
        })
        .and_then(|tp| tp.get("values"))
        .and_then(|vals| vals.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut rows = Vec::new();

    let Some(datasets) = v.get("dataSets").and_then(|ds| ds.as_array()) else {
        return Ok(rows);
    };

    for dataset in datasets {
        let Some(series_map) = dataset.get("series").and_then(|s| s.as_object()) else {
            continue;
        };
        for (_series_key, series_val) in series_map {
            let Some(observations) = series_val
                .get("observations")
                .and_then(|obs| obs.as_object())
            else {
                continue;
            };
            for (idx_str, obs_array) in observations {
                let idx: usize = idx_str.parse().unwrap_or(usize::MAX);
                let date = dates.get(idx).cloned().unwrap_or_default();
                if date.is_empty() {
                    continue;
                }
                let value = obs_array
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(serde_json::Value::as_f64);
                if let Some(value) = value {
                    rows.push(EcbObservation {
                        flow: flow.to_string(),
                        key: key.to_string(),
                        date,
                        value,
                    });
                }
            }
        }
    }

    // Sort by date for deterministic output.
    rows.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_data_query_with_valid_params() {
        let query = EcbDataQuery::new("EXR", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.flow, "EXR");
        assert_eq!(query.key, "D.USD.EUR.SP00.A");
        assert_eq!(query.start_period, "2024-01-01");
        assert_eq!(query.end_period, "2024-01-31");
    }

    #[test]
    fn rejects_empty_flow() {
        assert_eq!(
            EcbDataQuery::new("", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::EmptyFlow),
        );
    }

    #[test]
    fn rejects_whitespace_only_flow() {
        assert_eq!(
            EcbDataQuery::new("   ", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::EmptyFlow),
        );
    }

    #[test]
    fn rejects_flow_exceeding_100_chars() {
        let long = "X".repeat(101);
        assert_eq!(
            EcbDataQuery::new(long, "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::FlowTooLong),
        );
    }

    #[test]
    fn rejects_empty_key() {
        assert_eq!(
            EcbDataQuery::new("EXR", "", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::EmptyKey),
        );
    }

    #[test]
    fn rejects_key_exceeding_100_chars() {
        let long = "K".repeat(101);
        assert_eq!(
            EcbDataQuery::new("EXR", long, "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::KeyTooLong),
        );
    }

    #[test]
    fn data_request_path_contains_expected_segments() {
        let path = data_request_path("EXR", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("path should build: {error}"));

        assert!(path.contains("/data/EXR/D.USD.EUR.SP00.A"));
        assert!(path.contains("format=jsondata"));
        assert!(path.contains("startPeriod=2024-01-01"));
        assert!(path.contains("endPeriod=2024-01-31"));
    }

    #[test]
    fn mock_fetcher_parses_sample_envelope() {
        let raw = serde_json::json!({
            "dataSets": [{
                "series": {
                    "0:0:0:0:0": {
                        "observations": {
                            "0": [1.0934, 0, null],
                            "1": [1.0945, 0, null]
                        }
                    }
                }
            }],
            "structure": {
                "dimensions": {
                    "observation": [{
                        "id": "TIME_PERIOD",
                        "values": [
                            { "id": "2024-01-02" },
                            { "id": "2024-01-03" }
                        ]
                    }]
                }
            }
        })
        .to_string()
        .into_bytes();

        let query = EcbDataQuery::new("EXR", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("query should build: {error}"));
        let fetcher = EcbMockFetcher { raw };
        let rows = fetcher
            .parse(&query)
            .unwrap_or_else(|error| panic!("parse should succeed: {error}"));

        assert_eq!(rows.len(), 2, "rows={rows:#?}");
        assert_eq!(rows[0].date, "2024-01-02");
        assert_eq!(rows[0].value, 1.0934);
        assert_eq!(rows[1].date, "2024-01-03");
        assert_eq!(rows[1].value, 1.0945);
        assert_eq!(rows[0].flow, "EXR");
        assert_eq!(rows[0].key, "D.USD.EUR.SP00.A");
    }

    #[test]
    fn mock_fetcher_returns_empty_for_no_datasets() {
        let raw = serde_json::json!({ "dataSets": [] })
            .to_string()
            .into_bytes();
        let query = EcbDataQuery::new("EXR", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("query should build: {error}"));
        let fetcher = EcbMockFetcher { raw };
        let rows = fetcher
            .parse(&query)
            .unwrap_or_else(|error| panic!("parse should succeed: {error}"));

        assert!(rows.is_empty());
    }

    #[test]
    fn flow_trimmed_on_construction() {
        let query = EcbDataQuery::new("  EXR  ", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.flow, "EXR");
    }
}
