#![forbid(unsafe_code)]

//! BLS (Bureau of Labor Statistics) provider for the TDW data platform.
//!
//! Exposes query structs and a mock fetcher for offline use. The real HTTP
//! fetcher is compiled only under the `http` feature.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::BlsHttpTimeSeriesFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROVIDER_ID: &str = "bls";
pub const BASE_URL: &str = "https://api.bls.gov/publicAPI/v2";
pub const API_KEY_ENV: &str = "TDW_BLS_API_KEY";

/// Maximum number of series IDs allowed in a single BLS request.
pub const MAX_SERIES_IDS: usize = 50;

/// Earliest year accepted by the BLS v2 API.
pub const MIN_YEAR: u16 = 1900;

/// Latest year we allow in queries (well beyond any realistic request).
pub const MAX_YEAR: u16 = 2100;

pub type Result<T> = std::result::Result<T, BlsProviderError>;

/// Parameters for a BLS `timeseries/data` request.
///
/// # Validation
///
/// - `series_ids` must be non-empty and contain at most [`MAX_SERIES_IDS`] IDs.
/// - Each series ID must be non-empty and contain only ASCII alphanumeric
///   characters, underscores, or hyphens.
/// - `start_year` and `end_year` must be in `[MIN_YEAR, MAX_YEAR]`.
/// - `start_year` must be ≤ `end_year`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BlsSeriesQuery {
    pub series_ids: Vec<String>,
    pub start_year: u16,
    pub end_year: u16,
}

impl BlsSeriesQuery {
    /// Construct and validate a [`BlsSeriesQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`BlsProviderError`] if any validation constraint is violated.
    pub fn new(series_ids: Vec<String>, start_year: u16, end_year: u16) -> Result<Self> {
        validate_series_ids(&series_ids)?;
        validate_years(start_year, end_year)?;
        Ok(Self {
            series_ids,
            start_year,
            end_year,
        })
    }
}

/// One data point returned by the BLS API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BlsDataPoint {
    pub series_id: String,
    pub year: String,
    pub period: String,
    pub period_name: String,
    pub value: f64,
}

/// Errors produced by the BLS provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlsProviderError {
    #[error("series_ids must not be empty")]
    EmptySeriesIds,
    #[error("series_ids must contain at most {MAX_SERIES_IDS} IDs, got {0}")]
    TooManySeriesIds(usize),
    #[error("series ID must not be empty")]
    EmptySeriesId,
    #[error("series ID contains unsupported characters: {0}")]
    InvalidSeriesId(String),
    #[error("year {0} is outside the valid range [{MIN_YEAR}, {MAX_YEAR}]")]
    YearOutOfRange(u16),
    #[error("start_year ({0}) must be <= end_year ({1})")]
    YearRangeInverted(u16, u16),
    #[error("BLS API returned status: {0}")]
    ApiBadStatus(String),
    #[error("provider error: {0}")]
    Provider(String),
}

fn validate_series_ids(ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Err(BlsProviderError::EmptySeriesIds);
    }
    if ids.len() > MAX_SERIES_IDS {
        return Err(BlsProviderError::TooManySeriesIds(ids.len()));
    }
    for id in ids {
        validate_single_series_id(id)?;
    }
    Ok(())
}

fn validate_single_series_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(BlsProviderError::EmptySeriesId);
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
    {
        return Err(BlsProviderError::InvalidSeriesId(id.to_string()));
    }
    Ok(())
}

fn validate_years(start_year: u16, end_year: u16) -> Result<()> {
    if !(MIN_YEAR..=MAX_YEAR).contains(&start_year) {
        return Err(BlsProviderError::YearOutOfRange(start_year));
    }
    if !(MIN_YEAR..=MAX_YEAR).contains(&end_year) {
        return Err(BlsProviderError::YearOutOfRange(end_year));
    }
    if start_year > end_year {
        return Err(BlsProviderError::YearRangeInverted(start_year, end_year));
    }
    Ok(())
}

/// Mock fetcher that returns deterministic stub data without any network I/O.
///
/// Useful in unit tests and offline CI environments.
pub struct BlsMockFetcher;

impl BlsMockFetcher {
    /// Return a minimal stub response for each requested series ID.
    ///
    /// # Errors
    ///
    /// Returns [`BlsProviderError`] if `query` is invalid.
    pub fn fetch_stub(query: &BlsSeriesQuery) -> Result<Vec<BlsDataPoint>> {
        validate_series_ids(&query.series_ids)?;
        validate_years(query.start_year, query.end_year)?;

        let rows = query
            .series_ids
            .iter()
            .map(|id| BlsDataPoint {
                series_id: id.clone(),
                year: query.start_year.to_string(),
                period: "M01".to_string(),
                period_name: "January".to_string(),
                value: 100.0,
            })
            .collect();
        Ok(rows)
    }

    /// Parse a raw BLS JSON response body (`serde_json::Value`) into
    /// [`BlsDataPoint`] rows.
    ///
    /// This is the same parsing logic used by the HTTP fetcher, extracted here
    /// so it can be exercised in cassette tests without the `http` feature.
    ///
    /// # Errors
    ///
    /// Returns [`BlsProviderError::Provider`] if the JSON structure is
    /// unexpected or a value cannot be parsed as `f64`.
    pub fn parse_response(body: &Value) -> Result<Vec<BlsDataPoint>> {
        parse_bls_response(body)
    }
}

/// Shared JSON parsing logic used by both the mock and the HTTP fetcher.
pub(crate) fn parse_bls_response(body: &Value) -> Result<Vec<BlsDataPoint>> {
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    if status != "REQUEST_SUCCEEDED" {
        return Err(BlsProviderError::ApiBadStatus(status.to_string()));
    }

    let series_array = body
        .pointer("/Results/series")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            BlsProviderError::Provider("BLS response missing Results.series array".to_string())
        })?;

    let mut rows = Vec::new();
    for series in series_array {
        let series_id = series
            .get("seriesID")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BlsProviderError::Provider("BLS series missing seriesID field".to_string())
            })?;

        let data_array = series
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                BlsProviderError::Provider(format!("BLS series {series_id} missing data array"))
            })?;

        for point in data_array {
            let year = point.get("year").and_then(Value::as_str).ok_or_else(|| {
                BlsProviderError::Provider(format!("BLS data point in {series_id} missing year"))
            })?;
            let period = point.get("period").and_then(Value::as_str).ok_or_else(|| {
                BlsProviderError::Provider(format!("BLS data point in {series_id} missing period"))
            })?;
            let period_name = point
                .get("periodName")
                .and_then(Value::as_str)
                .unwrap_or("");
            let raw_value = point.get("value").and_then(Value::as_str).ok_or_else(|| {
                BlsProviderError::Provider(format!("BLS data point in {series_id} missing value"))
            })?;

            let value = raw_value.trim().parse::<f64>().map_err(|e| {
                BlsProviderError::Provider(format!(
                    "BLS value parse failed for {series_id}/{year}/{period}: {e}"
                ))
            })?;

            rows.push(BlsDataPoint {
                series_id: series_id.to_string(),
                year: year.to_string(),
                period: period.to_string(),
                period_name: period_name.to_string(),
                value,
            });
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- BlsSeriesQuery validation ---

    #[test]
    fn query_new_accepts_valid_inputs() {
        let q = BlsSeriesQuery::new(
            vec!["CUUR0000SA0".to_string(), "LNS14000000".to_string()],
            2020,
            2024,
        )
        .unwrap_or_else(|e| panic!("valid query must succeed: {e}"));

        assert_eq!(q.series_ids.len(), 2);
        assert_eq!(q.start_year, 2020);
        assert_eq!(q.end_year, 2024);
    }

    #[test]
    fn query_rejects_empty_series_ids() {
        let err =
            BlsSeriesQuery::new(vec![], 2020, 2024).expect_err("empty series_ids must be rejected");

        assert_eq!(err, BlsProviderError::EmptySeriesIds);
    }

    #[test]
    fn query_rejects_too_many_series_ids() {
        let ids: Vec<String> = (0..=MAX_SERIES_IDS)
            .map(|i| format!("SERIES{i:04}"))
            .collect();
        let err =
            BlsSeriesQuery::new(ids, 2020, 2024).expect_err("too many series IDs must be rejected");

        assert!(matches!(err, BlsProviderError::TooManySeriesIds(_)));
    }

    #[test]
    fn query_rejects_blank_series_id() {
        let err = BlsSeriesQuery::new(vec![String::new()], 2020, 2024)
            .expect_err("blank series ID must be rejected");

        assert_eq!(err, BlsProviderError::EmptySeriesId);
    }

    #[test]
    fn query_rejects_series_id_with_special_chars() {
        let err = BlsSeriesQuery::new(vec!["CPI&foo=bar".to_string()], 2020, 2024)
            .expect_err("series ID with special chars must be rejected");

        assert!(matches!(err, BlsProviderError::InvalidSeriesId(_)));
    }

    #[test]
    fn query_rejects_year_out_of_range() {
        let err = BlsSeriesQuery::new(vec!["CUUR0000SA0".to_string()], 1800, 2024)
            .expect_err("year below MIN_YEAR must be rejected");

        assert!(matches!(err, BlsProviderError::YearOutOfRange(1800)));
    }

    #[test]
    fn query_rejects_inverted_year_range() {
        let err = BlsSeriesQuery::new(vec!["CUUR0000SA0".to_string()], 2024, 2020)
            .expect_err("inverted year range must be rejected");

        assert!(matches!(
            err,
            BlsProviderError::YearRangeInverted(2024, 2020)
        ));
    }

    // --- BlsMockFetcher ---

    #[test]
    fn mock_fetcher_returns_one_stub_row_per_series() {
        let query = BlsSeriesQuery::new(
            vec!["CUUR0000SA0".to_string(), "LNS14000000".to_string()],
            2023,
            2024,
        )
        .unwrap_or_else(|e| panic!("valid query: {e}"));

        let rows = BlsMockFetcher::fetch_stub(&query)
            .unwrap_or_else(|e| panic!("mock fetch must succeed: {e}"));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].series_id, "CUUR0000SA0");
        assert_eq!(rows[0].year, "2023");
        assert_eq!(rows[1].series_id, "LNS14000000");
    }

    // --- parse_bls_response ---

    #[test]
    fn parse_response_decodes_well_formed_json() {
        let body = serde_json::json!({
            "status": "REQUEST_SUCCEEDED",
            "responseTime": 123,
            "Results": {
                "series": [{
                    "seriesID": "CUUR0000SA0",
                    "data": [
                        {
                            "year": "2024",
                            "period": "M01",
                            "periodName": "January",
                            "value": "308.417",
                            "footnotes": [{}]
                        }
                    ]
                }]
            }
        });

        let rows = BlsMockFetcher::parse_response(&body)
            .unwrap_or_else(|e| panic!("parse must succeed: {e}"));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].series_id, "CUUR0000SA0");
        assert_eq!(rows[0].year, "2024");
        assert_eq!(rows[0].period, "M01");
        assert_eq!(rows[0].period_name, "January");
        assert!((rows[0].value - 308.417).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_response_rejects_failed_status() {
        let body = serde_json::json!({
            "status": "REQUEST_FAILED",
            "Results": {}
        });

        let err = BlsMockFetcher::parse_response(&body)
            .expect_err("failed status must propagate as error");

        assert!(matches!(err, BlsProviderError::ApiBadStatus(_)));
    }
}
