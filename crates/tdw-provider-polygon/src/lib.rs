#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::PolygonHttpAggregatesFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "polygon";
pub const BASE_URL: &str = "https://api.polygon.io";
pub const API_KEY_PARAM: &str = "apiKey";

pub type Result<T> = std::result::Result<T, PolygonProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_param: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PolygonAggregatesQuery {
    pub ticker: String,
    pub from: String,
    pub to: String,
    pub adjusted: bool,
    pub limit: u32,
}

impl PolygonAggregatesQuery {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(ticker: &str, from: &str, to: &str) -> Result<Self> {
        Ok(Self {
            ticker: normalize_ticker(ticker)?,
            from: normalize_date(from)?,
            to: normalize_date(to)?,
            adjusted: true,
            limit: 5_000,
        })
    }

    #[must_use]
    pub const fn with_adjusted(mut self, adjusted: bool) -> Self {
        self.adjusted = adjusted;
        self
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn with_limit(mut self, limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(PolygonProviderError::InvalidLimit);
        }
        self.limit = limit;
        Ok(self)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolygonProviderError {
    #[error("polygon ticker must not be empty")]
    EmptyTicker,
    #[error("polygon ticker contains unsupported characters")]
    InvalidTicker,
    #[error("polygon aggregate date must be YYYY-MM-DD")]
    InvalidDate,
    #[error("polygon aggregate limit must be greater than zero")]
    InvalidLimit,
    #[error("polygon api key must be supplied by the caller")]
    MissingApiKey,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn aggregates_request(ticker: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(PolygonProviderError::MissingApiKey);
    }
    let ticker = normalize_ticker(ticker)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "aggregates",
        path: format!("/v2/aggs/ticker/{ticker}/range/1/day"),
        credential_param: API_KEY_PARAM,
    })
}

fn normalize_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(PolygonProviderError::EmptyTicker);
    }
    if !ticker
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(PolygonProviderError::InvalidTicker);
    }
    Ok(ticker.to_ascii_uppercase())
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
        return Err(PolygonProviderError::InvalidDate);
    }
    Ok(date.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_aggregates_request_contract() {
        let request = aggregates_request("msft", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(request.provider, "polygon");
        assert!(request.path.contains("/MSFT/range/1/day"));
        assert_eq!(request.credential_param, API_KEY_PARAM);
        assert!(aggregates_request("MSFT", false).is_err());
        assert!(aggregates_request("", true).is_err());
        assert!(aggregates_request("MSFT?adjusted=false", true).is_err());
    }

    #[test]
    fn builds_aggregates_query_model() {
        let query = PolygonAggregatesQuery::new("msft", "2024-01-02", "2024-01-05")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.ticker, "MSFT");
        assert_eq!(query.from, "2024-01-02");
        assert_eq!(query.to, "2024-01-05");
        assert!(query.adjusted);
        assert_eq!(query.limit, 5_000);
        assert!(
            PolygonAggregatesQuery::new("MSFT?adjusted=false", "2024-01-02", "2024-01-05").is_err()
        );
        assert!(PolygonAggregatesQuery::new("MSFT", "20240102", "2024-01-05").is_err());
        assert!(
            PolygonAggregatesQuery::new("MSFT", "2024-01-02", "2024-01-05")
                .and_then(|query| query.with_limit(0))
                .is_err()
        );
    }
}
