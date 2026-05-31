#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::NasdaqHttpDatasetFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "nasdaq";
pub const BASE_URL: &str = "https://data.nasdaq.com/api/v3";
pub const API_KEY_PARAM: &str = "api_key";

pub type Result<T> = std::result::Result<T, NasdaqProviderError>;

/// Maximum allowed length for database and dataset identifiers.
const MAX_IDENTIFIER_LEN: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_param: &'static str,
}

/// Query parameters for the NASDAQ Data Link dataset data endpoint.
///
/// Maps to `GET /datasets/{database}/{dataset}/data`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NasdaqDatasetQuery {
    pub database: String,
    pub dataset: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl NasdaqDatasetQuery {
    /// Build a validated query for the given database/dataset pair.
    ///
    /// # Errors
    ///
    /// Returns an error if `database` or `dataset` fail identifier
    /// validation (empty, too long, or contain non-ASCII-alphanumeric /
    /// underscore characters).
    pub fn new(database: &str, dataset: &str) -> Result<Self> {
        Ok(Self {
            database: normalize_identifier(database)?,
            dataset: normalize_identifier(dataset)?,
            start_date: None,
            end_date: None,
        })
    }

    /// Restrict results to observations on or after `YYYY-MM-DD`.
    ///
    /// # Errors
    ///
    /// Returns an error if the date string is not in `YYYY-MM-DD` format.
    pub fn with_start_date(mut self, date: &str) -> Result<Self> {
        self.start_date = Some(normalize_date(date)?);
        Ok(self)
    }

    /// Restrict results to observations on or before `YYYY-MM-DD`.
    ///
    /// # Errors
    ///
    /// Returns an error if the date string is not in `YYYY-MM-DD` format.
    pub fn with_end_date(mut self, date: &str) -> Result<Self> {
        self.end_date = Some(normalize_date(date)?);
        Ok(self)
    }
}

/// A single row returned by the NASDAQ Data Link dataset data endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NasdaqDataRow {
    pub database: String,
    pub dataset: String,
    pub column_names: Vec<String>,
    pub values: Vec<serde_json::Value>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NasdaqProviderError {
    #[error("nasdaq database/dataset identifier must not be empty")]
    EmptyIdentifier,
    #[error("nasdaq database/dataset identifier exceeds {MAX_IDENTIFIER_LEN} characters")]
    IdentifierTooLong,
    #[error("nasdaq database/dataset identifier contains unsupported characters")]
    InvalidIdentifier,
    #[error("nasdaq date must be YYYY-MM-DD")]
    InvalidDate,
    #[error("nasdaq api key must be supplied by the caller")]
    MissingApiKey,
}

/// Build and validate a provider request for the dataset data endpoint.
///
/// # Errors
///
/// Returns [`NasdaqProviderError::MissingApiKey`] when `api_key_present`
/// is `false`, or a validation error when the identifiers are invalid.
pub fn dataset_request(
    database: &str,
    dataset: &str,
    api_key_present: bool,
) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(NasdaqProviderError::MissingApiKey);
    }
    let database = normalize_identifier(database)?;
    let dataset = normalize_identifier(dataset)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "datasets",
        path: format!("/datasets/{database}/{dataset}/data"),
        credential_param: API_KEY_PARAM,
    })
}

fn normalize_identifier(identifier: &str) -> Result<String> {
    let identifier = identifier.trim();
    if identifier.is_empty() {
        return Err(NasdaqProviderError::EmptyIdentifier);
    }
    if identifier.len() > MAX_IDENTIFIER_LEN {
        return Err(NasdaqProviderError::IdentifierTooLong);
    }
    if !identifier
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(NasdaqProviderError::InvalidIdentifier);
    }
    Ok(identifier.to_ascii_uppercase())
}

fn normalize_date(date: &str) -> Result<String> {
    let date = date.trim();
    let bytes = date.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit());
    if !valid {
        return Err(NasdaqProviderError::InvalidDate);
    }
    Ok(date.to_string())
}

/// Offline stub fetcher that returns a single hard-coded row.
///
/// Used in unit tests and when the `http` feature is disabled. Not
/// suitable for production use.
pub struct NasdaqStubFetcher;

impl NasdaqStubFetcher {
    /// Return stub rows for the given query without performing any
    /// network I/O.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails validation.
    pub fn fetch(query: &NasdaqDatasetQuery) -> Result<Vec<NasdaqDataRow>> {
        Ok(vec![NasdaqDataRow {
            database: query.database.clone(),
            dataset: query.dataset.clone(),
            column_names: vec![
                "Date".to_string(),
                "Open".to_string(),
                "High".to_string(),
                "Low".to_string(),
                "Close".to_string(),
                "Volume".to_string(),
            ],
            values: vec![
                serde_json::Value::String("2024-01-02".to_string()),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(185.6)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(186.1)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(184.4)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(185.2)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
                serde_json::Value::Number(serde_json::Number::from(55_000_000_u64)),
            ],
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_dataset_request_contract() {
        let request = dataset_request("WIKI", "AAPL", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(request.provider, "nasdaq");
        assert!(request.path.contains("/datasets/WIKI/AAPL/data"));
        assert_eq!(request.credential_param, API_KEY_PARAM);
        assert!(dataset_request("WIKI", "AAPL", false).is_err());
        assert!(dataset_request("", "AAPL", true).is_err());
        assert!(dataset_request("WIKI", "", true).is_err());
        assert!(dataset_request("WIKI?inject=1", "AAPL", true).is_err());
    }

    #[test]
    fn builds_dataset_query_with_optional_dates() {
        let query = NasdaqDatasetQuery::new("WIKI", "AAPL")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.database, "WIKI");
        assert_eq!(query.dataset, "AAPL");
        assert!(query.start_date.is_none());
        assert!(query.end_date.is_none());

        let query_with_dates = NasdaqDatasetQuery::new("FRED", "GDP")
            .and_then(|q| q.with_start_date("2024-01-01"))
            .and_then(|q| q.with_end_date("2024-12-31"))
            .unwrap_or_else(|error| panic!("query with dates should build: {error}"));

        assert_eq!(query_with_dates.start_date.as_deref(), Some("2024-01-01"));
        assert_eq!(query_with_dates.end_date.as_deref(), Some("2024-12-31"));
    }

    #[test]
    fn rejects_invalid_identifiers() {
        assert!(matches!(
            NasdaqDatasetQuery::new("", "AAPL"),
            Err(NasdaqProviderError::EmptyIdentifier)
        ));
        assert!(matches!(
            NasdaqDatasetQuery::new("WIKI", ""),
            Err(NasdaqProviderError::EmptyIdentifier)
        ));
        assert!(matches!(
            NasdaqDatasetQuery::new("WIKI/../../secret", "AAPL"),
            Err(NasdaqProviderError::InvalidIdentifier)
        ));
        assert!(matches!(
            NasdaqDatasetQuery::new("WIKI", "AAPL?api_key=stolen"),
            Err(NasdaqProviderError::InvalidIdentifier)
        ));

        let long_id = "A".repeat(MAX_IDENTIFIER_LEN + 1);
        assert!(matches!(
            NasdaqDatasetQuery::new(&long_id, "AAPL"),
            Err(NasdaqProviderError::IdentifierTooLong)
        ));
    }

    #[test]
    fn rejects_invalid_dates() {
        let query = NasdaqDatasetQuery::new("WIKI", "AAPL")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert!(matches!(
            query.clone().with_start_date("20240101"),
            Err(NasdaqProviderError::InvalidDate)
        ));
        assert!(matches!(
            query.with_end_date("2024/01/01"),
            Err(NasdaqProviderError::InvalidDate)
        ));
    }

    #[test]
    fn normalizes_identifiers_to_uppercase() {
        let query = NasdaqDatasetQuery::new("wiki", "aapl")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.database, "WIKI");
        assert_eq!(query.dataset, "AAPL");
    }

    #[test]
    fn stub_fetcher_returns_single_row() {
        let query = NasdaqDatasetQuery::new("WIKI", "AAPL")
            .unwrap_or_else(|error| panic!("query should build: {error}"));
        let rows = NasdaqStubFetcher::fetch(&query)
            .unwrap_or_else(|error| panic!("stub fetch should succeed: {error}"));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].database, "WIKI");
        assert_eq!(rows[0].dataset, "AAPL");
        assert_eq!(rows[0].column_names[0], "Date");
        assert_eq!(
            rows[0].values[0],
            serde_json::Value::String("2024-01-02".to_string())
        );
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            NasdaqProviderError::EmptyIdentifier
                .to_string()
                .contains("empty")
        );
        assert!(
            NasdaqProviderError::IdentifierTooLong
                .to_string()
                .contains("exceeds")
        );
        assert!(
            NasdaqProviderError::InvalidIdentifier
                .to_string()
                .contains("unsupported")
        );
        assert!(
            NasdaqProviderError::InvalidDate
                .to_string()
                .contains("YYYY-MM-DD")
        );
        assert!(
            NasdaqProviderError::MissingApiKey
                .to_string()
                .contains("api key")
        );
    }
}
