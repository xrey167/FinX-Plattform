#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{EcbHttpDataFetcher, EcbHttpReferenceRatesFetcher, EcbReferenceRatesQuery};

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
        let flow: String = flow.into();
        let flow = validate_flow(&flow)?;
        let key: String = key.into();
        let key = validate_key(&key)?;
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
    #[error("ecb flow contains characters outside the SDMX grammar")]
    InvalidFlow,
    #[error("ecb key contains characters outside the SDMX grammar")]
    InvalidKey,
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
    let flow = validate_flow(flow)?;
    let key = validate_key(key)?;
    Ok(format!(
        "/data/{flow}/{key}?format=jsondata&startPeriod={start_period}&endPeriod={end_period}"
    ))
}

fn validate_flow(flow: &str) -> Result<String> {
    let flow = flow.trim().to_string();
    if flow.is_empty() {
        return Err(EcbProviderError::EmptyFlow);
    }
    if flow.len() > 100 {
        return Err(EcbProviderError::FlowTooLong);
    }
    if !flow.chars().all(is_sdmx_char) {
        return Err(EcbProviderError::InvalidFlow);
    }
    Ok(flow)
}

fn validate_key(key: &str) -> Result<String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(EcbProviderError::EmptyKey);
    }
    if key.len() > 100 {
        return Err(EcbProviderError::KeyTooLong);
    }
    if !key.chars().all(is_sdmx_char) {
        return Err(EcbProviderError::InvalidKey);
    }
    Ok(key)
}

/// SDMX flow/series-key grammar: ASCII alphanumerics plus the structural
/// characters SDMX itself uses — `.` (dimension separator), `+` (OR within a
/// dimension), and `-`. Crucially this excludes `/`, `?`, `&`, `#` and
/// whitespace, so a `flow`/`key` interpolated into the `/data/{flow}/{key}?…`
/// URL path can no longer inject extra path segments or query parameters.
fn is_sdmx_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')
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
    Ok(parse_ecb_value(&v, flow, key))
}

fn parse_ecb_value(v: &serde_json::Value, flow: &str, key: &str) -> Vec<EcbObservation> {
    // Extract the time-period values array from the structure block (shared with
    // the reference-rates parser).
    let dates = observation_dates(v);

    let mut rows = Vec::new();

    let Some(datasets) = v.get("dataSets").and_then(|ds| ds.as_array()) else {
        return rows;
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
    rows
}

/// A reference-rate observation tagged with its resolved currency code.
///
/// Distinct from [`EcbObservation`] (which carries the request's `flow`/`key`):
/// a single wildcard EXR request returns one series per currency, so each row
/// must carry the currency that the SDMX series-dimension resolves to rather
/// than the wildcard key.
#[derive(Clone, Debug, PartialEq)]
pub struct EcbReferenceRate {
    /// ISO 4217 currency code the rate quotes (e.g. `"USD"`), resolved from the
    /// SDMX `CURRENCY` series dimension. Falls back to the colon-joined series
    /// key when the dimension cannot be resolved.
    pub currency: String,
    /// Observation date in `YYYY-MM-DD` (daily) format.
    pub date: String,
    /// Euro reference rate (units of `currency` per euro).
    pub value: f64,
}

/// Parse raw ECB `jsondata` bytes into per-currency reference rates.
///
/// Unlike [`parse_ecb_json`], which tags every row with the request key, this
/// resolves each series' `CURRENCY` dimension value so a single wildcard EXR
/// request (`D..EUR.SP00.A`) yields one labelled row per published pair. The
/// observation-axis date decoding is shared with [`parse_ecb_value`]; only the
/// per-series currency attribution is added.
///
/// # Errors
///
/// Returns [`EcbProviderError::EmptyFlow`] if the JSON cannot be parsed.
pub fn parse_ecb_reference_rates(raw: &[u8]) -> Result<Vec<EcbReferenceRate>> {
    let v: serde_json::Value =
        serde_json::from_slice(raw).map_err(|_| EcbProviderError::EmptyFlow)?;
    Ok(parse_ecb_reference_rates_value(&v))
}

fn parse_ecb_reference_rates_value(v: &serde_json::Value) -> Vec<EcbReferenceRate> {
    let dates = observation_dates(v);
    // The CURRENCY series-dimension values, in declaration order, plus the
    // position of that dimension within the series key.
    let (currency_position, currency_values) = currency_dimension(v);

    let mut rows = Vec::new();
    let Some(datasets) = v.get("dataSets").and_then(|ds| ds.as_array()) else {
        return rows;
    };
    for dataset in datasets {
        let Some(series_map) = dataset.get("series").and_then(|s| s.as_object()) else {
            continue;
        };
        for (series_key, series_val) in series_map {
            let currency = resolve_currency(series_key, currency_position, &currency_values);
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
                    rows.push(EcbReferenceRate {
                        currency: currency.clone(),
                        date,
                        value,
                    });
                }
            }
        }
    }
    rows.sort_by(|a, b| (&a.currency, &a.date).cmp(&(&b.currency, &b.date)));
    rows
}

/// The observation-axis `TIME_PERIOD` values, in index order.
fn observation_dates(v: &serde_json::Value) -> Vec<String> {
    v.pointer("/structure/dimensions/observation")
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
        .unwrap_or_default()
}

/// Locate the `CURRENCY` series dimension: its position within the colon-joined
/// series key and its ordered `id` values.
fn currency_dimension(v: &serde_json::Value) -> (Option<usize>, Vec<String>) {
    let Some(dims) = v
        .pointer("/structure/dimensions/series")
        .and_then(|series| series.as_array())
    else {
        return (None, Vec::new());
    };
    for (position, dim) in dims.iter().enumerate() {
        if dim.get("id").and_then(|id| id.as_str()) == Some("CURRENCY") {
            let values = dim
                .get("values")
                .and_then(|vals| vals.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default();
            return (Some(position), values);
        }
    }
    (None, Vec::new())
}

/// Resolve a series key (e.g. `"0:1:0:0:0"`) to its currency code via the
/// `CURRENCY` dimension index, falling back to the raw key on any miss.
fn resolve_currency(series_key: &str, position: Option<usize>, values: &[String]) -> String {
    let resolved = position
        .and_then(|position| series_key.split(':').nth(position))
        .and_then(|index_str| index_str.parse::<usize>().ok())
        .and_then(|index| values.get(index));
    resolved.map_or_else(|| series_key.to_string(), Clone::clone)
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact-value parse assertions; deterministic parser, exact comparison intended
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
    fn rejects_flow_with_injection_characters() {
        // A `/` would open a new URL path segment; `?`/`&` would inject query
        // params. The SDMX grammar excludes all of them.
        assert_eq!(
            EcbDataQuery::new("EX/R", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::InvalidFlow),
        );
    }

    #[test]
    fn rejects_key_with_injection_characters() {
        // `EXR/../../something?evil=1`-style payloads inject path segments and
        // query parameters into the ECB request; charset validation blocks them.
        assert_eq!(
            EcbDataQuery::new("EXR", "EXR/../../x?evil=1", "2024-01-01", "2024-01-31"),
            Err(EcbProviderError::InvalidKey),
        );
        // A `+` (SDMX OR) and `-` remain valid so legitimate keys still parse.
        assert!(EcbDataQuery::new("EXR", "D.USD+GBP.EUR-SP.A", "2024-01-01", "2024-01-31").is_ok());
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
    fn reference_rates_resolve_currency_per_series() {
        let raw = serde_json::json!({
            "dataSets": [{
                "series": {
                    "0:0:0:0:0": { "observations": { "0": [1.0934, 0, null] } },
                    "0:1:0:0:0": { "observations": { "0": [0.8534, 0, null] } }
                }
            }],
            "structure": {
                "dimensions": {
                    "series": [
                        { "id": "FREQ", "values": [{ "id": "D" }] },
                        { "id": "CURRENCY", "values": [{ "id": "USD" }, { "id": "GBP" }] },
                        { "id": "CURRENCY_DENOM", "values": [{ "id": "EUR" }] },
                        { "id": "EXR_TYPE", "values": [{ "id": "SP00" }] },
                        { "id": "EXR_SUFFIX", "values": [{ "id": "A" }] }
                    ],
                    "observation": [{
                        "id": "TIME_PERIOD",
                        "values": [{ "id": "2024-01-02" }]
                    }]
                }
            }
        })
        .to_string()
        .into_bytes();

        let rows = parse_ecb_reference_rates(&raw)
            .unwrap_or_else(|error| panic!("parse should succeed: {error}"));
        assert_eq!(rows.len(), 2, "rows={rows:#?}");
        // Sorted by (currency, date): GBP then USD.
        assert_eq!(rows[0].currency, "GBP");
        assert_eq!(rows[0].value, 0.8534);
        assert_eq!(rows[1].currency, "USD");
        assert_eq!(rows[1].value, 1.0934);
    }

    #[test]
    fn reference_rates_falls_back_to_series_key_without_currency_dimension() {
        let raw = serde_json::json!({
            "dataSets": [{
                "series": { "0:0": { "observations": { "0": [1.1, 0, null] } } }
            }],
            "structure": {
                "dimensions": {
                    "observation": [{
                        "id": "TIME_PERIOD",
                        "values": [{ "id": "2024-01-02" }]
                    }]
                }
            }
        })
        .to_string()
        .into_bytes();

        let rows = parse_ecb_reference_rates(&raw)
            .unwrap_or_else(|error| panic!("parse should succeed: {error}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].currency, "0:0");
    }

    #[test]
    fn flow_trimmed_on_construction() {
        let query = EcbDataQuery::new("  EXR  ", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.flow, "EXR");
    }
}
