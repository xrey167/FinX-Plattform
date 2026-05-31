#![forbid(unsafe_code)]

//! Alpha Vantage market-data provider for the TDW platform.
//!
//! # Rate limits
//!
//! The free-tier Alpha Vantage API key is limited to **25 requests per day**.
//! Use the paid tier for higher throughput or batch workloads.
//!
//! # Authentication
//!
//! Set `TDW_ALPHA_VANTAGE_API_KEY` in the environment before making live
//! calls.  The key is appended as the `apikey` query parameter on every
//! request.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::AlphaVantageHttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider identifier used in registry entries and error messages.
pub const PROVIDER_ID: &str = "alpha_vantage";

/// Base URL for the Alpha Vantage REST API.
pub const BASE_URL: &str = "https://www.alphavantage.co/query";

/// Environment variable that holds the Alpha Vantage API key.
pub const API_KEY_ENV: &str = "TDW_ALPHA_VANTAGE_API_KEY";

/// Convenience type alias for fallible Alpha Vantage operations.
pub type Result<T> = std::result::Result<T, AlphaVantageError>;

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Which Alpha Vantage function to call.
///
/// # Rate limits
///
/// The free tier allows 25 requests per day across all functions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlphaVantageFunction {
    /// Daily OHLCV time series (`TIME_SERIES_DAILY`).
    TimeSeriesDaily,
    /// Latest quote snapshot (`GLOBAL_QUOTE`).
    GlobalQuote,
}

impl AlphaVantageFunction {
    /// Returns the `function=` query-parameter value expected by Alpha Vantage.
    #[must_use]
    pub fn as_api_str(&self) -> &'static str {
        match self {
            Self::TimeSeriesDaily => "TIME_SERIES_DAILY",
            Self::GlobalQuote => "GLOBAL_QUOTE",
        }
    }
}

/// Query parameters for an Alpha Vantage request.
///
/// # Rate limits
///
/// Alpha Vantage free-tier keys are limited to **25 requests per day**.
/// Repeated or automated calls should cache results locally.
///
/// # Example
///
/// ```rust
/// use tdw_provider_alpha_vantage::{AlphaVantageFunction, AlphaVantageQuery};
///
/// let q = AlphaVantageQuery::new("AAPL", AlphaVantageFunction::GlobalQuote)
///     .expect("valid query");
/// assert_eq!(q.symbol, "AAPL");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlphaVantageQuery {
    /// Ticker symbol (uppercase, ASCII alphanumeric with `.`, `-`, `_`).
    pub symbol: String,
    /// Which Alpha Vantage endpoint to hit.
    pub function: AlphaVantageFunction,
}

impl AlphaVantageQuery {
    /// Construct and validate a new query.
    ///
    /// # Errors
    ///
    /// Returns [`AlphaVantageError::EmptySymbol`] when `symbol` is blank.
    /// Returns [`AlphaVantageError::InvalidSymbol`] when `symbol` contains
    /// characters not allowed by Alpha Vantage (non-ASCII-alphanumeric except
    /// `.`, `-`, `_`).
    pub fn new(symbol: &str, function: AlphaVantageFunction) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            function,
        })
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the Alpha Vantage provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AlphaVantageError {
    /// The symbol string was empty or whitespace-only.
    #[error("alpha vantage symbol must not be empty")]
    EmptySymbol,

    /// The symbol contains characters not accepted by Alpha Vantage.
    #[error("alpha vantage symbol contains unsupported characters")]
    InvalidSymbol,

    /// The `TDW_ALPHA_VANTAGE_API_KEY` environment variable was not set or
    /// was empty.
    #[error("alpha vantage api key env TDW_ALPHA_VANTAGE_API_KEY must be set")]
    MissingApiKey,

    /// A provider-level error (HTTP failure, unexpected JSON shape, etc.).
    #[error("alpha vantage provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Offline / mock helpers
// ---------------------------------------------------------------------------

/// Returns a minimal stub `TIME_SERIES_DAILY` JSON response for offline
/// testing.
#[must_use]
pub fn stub_time_series_daily(symbol: &str) -> String {
    format!(
        r#"{{"Time Series (Daily)": {{"2024-01-02": {{"1. open": "185.6","2. high": "186.1","3. low": "184.4","4. close": "185.2","5. volume": "55000000"}},"2024-01-03": {{"1. open": "184.0","2. high": "185.0","3. low": "183.5","4. close": "184.8","5. volume": "48000000"}}}}, "Meta Data": {{"2. Symbol": "{symbol}"}}}}"#
    )
}

/// Returns a minimal stub `GLOBAL_QUOTE` JSON response for offline testing.
#[must_use]
pub fn stub_global_quote(symbol: &str) -> String {
    format!(
        r#"{{"Global Quote": {{"01. symbol": "{symbol}","05. price": "185.20","06. volume": "55000000"}}}}"#
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(AlphaVantageError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(AlphaVantageError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_new_normalizes_symbol_to_uppercase() {
        let q = AlphaVantageQuery::new("aapl", AlphaVantageFunction::GlobalQuote)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
        assert_eq!(q.function, AlphaVantageFunction::GlobalQuote);
    }

    #[test]
    fn query_new_rejects_empty_symbol() {
        assert_eq!(
            AlphaVantageQuery::new("", AlphaVantageFunction::GlobalQuote),
            Err(AlphaVantageError::EmptySymbol)
        );
        assert_eq!(
            AlphaVantageQuery::new("   ", AlphaVantageFunction::GlobalQuote),
            Err(AlphaVantageError::EmptySymbol)
        );
    }

    #[test]
    fn query_new_rejects_symbols_with_invalid_characters() {
        assert_eq!(
            AlphaVantageQuery::new("AAPL/../../etc", AlphaVantageFunction::GlobalQuote),
            Err(AlphaVantageError::InvalidSymbol)
        );
        assert_eq!(
            AlphaVantageQuery::new("AAPL?key=x", AlphaVantageFunction::GlobalQuote),
            Err(AlphaVantageError::InvalidSymbol)
        );
        assert_eq!(
            AlphaVantageQuery::new("AAPL&symbol=BRK", AlphaVantageFunction::GlobalQuote),
            Err(AlphaVantageError::InvalidSymbol)
        );
    }

    #[test]
    fn query_new_accepts_dots_dashes_underscores() {
        let q = AlphaVantageQuery::new("BRK.B", AlphaVantageFunction::TimeSeriesDaily)
            .unwrap_or_else(|e| panic!("BRK.B should be valid: {e}"));
        assert_eq!(q.symbol, "BRK.B");

        let q2 = AlphaVantageQuery::new("some_etf-x", AlphaVantageFunction::TimeSeriesDaily)
            .unwrap_or_else(|e| panic!("some_etf-x should be valid: {e}"));
        assert_eq!(q2.symbol, "SOME_ETF-X");
    }

    #[test]
    fn function_as_api_str_returns_correct_values() {
        assert_eq!(
            AlphaVantageFunction::TimeSeriesDaily.as_api_str(),
            "TIME_SERIES_DAILY"
        );
        assert_eq!(
            AlphaVantageFunction::GlobalQuote.as_api_str(),
            "GLOBAL_QUOTE"
        );
    }

    #[test]
    fn stub_time_series_daily_contains_symbol_and_dates() {
        let json = stub_time_series_daily("MSFT");
        assert!(json.contains("MSFT"));
        assert!(json.contains("2024-01-02"));
        assert!(json.contains("185.2"));
    }

    #[test]
    fn stub_global_quote_contains_symbol_and_price() {
        let json = stub_global_quote("TSLA");
        assert!(json.contains("TSLA"));
        assert!(json.contains("185.20"));
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert_eq!(
            AlphaVantageError::EmptySymbol.to_string(),
            "alpha vantage symbol must not be empty"
        );
        assert_eq!(
            AlphaVantageError::InvalidSymbol.to_string(),
            "alpha vantage symbol contains unsupported characters"
        );
        assert_eq!(
            AlphaVantageError::MissingApiKey.to_string(),
            "alpha vantage api key env TDW_ALPHA_VANTAGE_API_KEY must be set"
        );
        assert!(
            AlphaVantageError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
