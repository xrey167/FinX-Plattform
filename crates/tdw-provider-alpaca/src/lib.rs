#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::AlpacaHttpStockBarsFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "alpaca";
pub const BASE_URL: &str = "https://data.alpaca.markets";
pub const API_KEY_HEADER: &str = "APCA-API-KEY-ID";
pub const API_SECRET_HEADER: &str = "APCA-API-SECRET-KEY";

pub type Result<T> = std::result::Result<T, AlpacaProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub credential_header: &'static str,
    pub secret_header: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_header: &'static str,
    pub secret_header: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlpacaStockBarsQuery {
    pub symbol: String,
    pub start: String,
    pub end: String,
    pub timeframe: String,
    pub limit: u32,
    pub feed: Option<String>,
}

impl AlpacaStockBarsQuery {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(symbol: &str, start: &str, end: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            start: normalize_date(start)?,
            end: normalize_date(end)?,
            timeframe: "1Day".to_string(),
            limit: 1_000,
            feed: None,
        })
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn with_timeframe(mut self, timeframe: &str) -> Result<Self> {
        self.timeframe = normalize_token(timeframe)?;
        Ok(self)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn with_limit(mut self, limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(AlpacaProviderError::InvalidLimit);
        }
        self.limit = limit;
        Ok(self)
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn with_feed(mut self, feed: Option<&str>) -> Result<Self> {
        self.feed = feed.map(normalize_token).transpose()?;
        Ok(self)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AlpacaProviderError {
    #[error("alpaca symbol must not be empty")]
    EmptySymbol,
    #[error("alpaca symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("alpaca stock-bars date must be YYYY-MM-DD")]
    InvalidDate,
    #[error("alpaca query token contains unsupported characters")]
    InvalidToken,
    #[error("alpaca stock-bars limit must be greater than zero")]
    InvalidLimit,
    #[error("alpaca api key must be supplied by the caller")]
    MissingApiKey,
    #[error("alpaca api secret must be supplied by the caller")]
    MissingApiSecret,
}

#[must_use]
pub const fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "stock_bars",
        base_url: BASE_URL,
        credential_header: API_KEY_HEADER,
        secret_header: API_SECRET_HEADER,
    }];
    ENDPOINTS
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn stock_bars_request(symbol: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(AlpacaProviderError::MissingApiKey);
    }
    let symbol = normalize_symbol(symbol)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "stock_bars",
        path: format!("/v2/stocks/bars?symbols={symbol}"),
        credential_header: API_KEY_HEADER,
        secret_header: API_SECRET_HEADER,
    })
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(AlpacaProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_'))
    {
        return Err(AlpacaProviderError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
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
        return Err(AlpacaProviderError::InvalidDate);
    }
    Ok(date.to_string())
}

fn normalize_token(token: &str) -> Result<String> {
    let token = token.trim();
    if token.is_empty()
        || !token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(AlpacaProviderError::InvalidToken);
    }
    Ok(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_offline_request_contract_without_secret_material() {
        let request = stock_bars_request("aapl", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(endpoints()[0].provider, "alpaca");
        assert_eq!(request.path, "/v2/stocks/bars?symbols=AAPL");
        assert_eq!(request.credential_header, API_KEY_HEADER);
        assert_eq!(request.secret_header, API_SECRET_HEADER);
        assert!(stock_bars_request("AAPL", false).is_err());
        assert!(stock_bars_request("", true).is_err());
        assert!(stock_bars_request("AAPL/../../secret", true).is_err());
    }

    #[test]
    fn builds_stock_bars_query_model() {
        let query = AlpacaStockBarsQuery::new("aapl", "2024-01-02", "2024-01-05")
            .and_then(|query| query.with_timeframe("1Day"))
            .and_then(|query| query.with_limit(100))
            .and_then(|query| query.with_feed(Some("iex")))
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.symbol, "AAPL");
        assert_eq!(query.start, "2024-01-02");
        assert_eq!(query.end, "2024-01-05");
        assert_eq!(query.timeframe, "1Day");
        assert_eq!(query.limit, 100);
        assert_eq!(query.feed.as_deref(), Some("iex"));
        assert!(
            AlpacaStockBarsQuery::new("AAPL/../../secret", "2024-01-02", "2024-01-05").is_err()
        );
        assert!(AlpacaStockBarsQuery::new("AAPL", "20240102", "2024-01-05").is_err());
        assert!(
            AlpacaStockBarsQuery::new("AAPL", "2024-01-02", "2024-01-05")
                .and_then(|query| query.with_timeframe("1Day&limit=1"))
                .is_err()
        );
        assert!(
            AlpacaStockBarsQuery::new("AAPL", "2024-01-02", "2024-01-05")
                .and_then(|query| query.with_limit(0))
                .is_err()
        );
    }
}
