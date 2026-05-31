#![forbid(unsafe_code)]
//! Tiingo data provider — offline query types, validation, and mock fetcher.
//!
//! The real HTTP fetcher is behind the `http` feature and lives in
//! [`http_fetcher`]. All types here compile without any network dependency.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{TiingoHttpHistoricalFetcher, TiingoHttpNewsFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "tiingo";
pub const BASE_URL: &str = "https://api.tiingo.com/tiingo";
pub const API_KEY_ENV: &str = "TDW_TIINGO_API_KEY";

pub type Result<T> = std::result::Result<T, TiingoProviderError>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TiingoProviderError {
    #[error("tiingo ticker must not be empty")]
    EmptyTicker,
    #[error("tiingo ticker contains unsupported characters")]
    InvalidTicker,
    #[error("tiingo news query must contain at least one ticker")]
    EmptyTickerList,
    #[error("tiingo provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query for Tiingo daily historical price data.
///
/// Maps to `GET /daily/{ticker}/prices`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TiingoHistoricalQuery {
    pub symbol: String,
}

impl TiingoHistoricalQuery {
    /// Build a validated historical query for the given symbol.
    ///
    /// # Errors
    ///
    /// Returns [`TiingoProviderError::EmptyTicker`] or
    /// [`TiingoProviderError::InvalidTicker`] when the symbol fails
    /// validation.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_ticker(symbol)?,
        })
    }
}

/// Query for Tiingo news feed filtered by tickers.
///
/// Maps to `GET /news?tickers={ticker}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TiingoNewsQuery {
    pub tickers: Vec<String>,
}

impl TiingoNewsQuery {
    /// Build a validated news query for the given list of tickers.
    ///
    /// # Errors
    ///
    /// Returns [`TiingoProviderError::EmptyTickerList`] when no tickers are
    /// supplied, or a ticker-level error when any entry fails validation.
    pub fn new(tickers: &[&str]) -> Result<Self> {
        if tickers.is_empty() {
            return Err(TiingoProviderError::EmptyTickerList);
        }
        let normalized = tickers
            .iter()
            .map(|t| normalize_ticker(t))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tickers: normalized,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(TiingoProviderError::EmptyTicker);
    }
    if !ticker
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(TiingoProviderError::InvalidTicker);
    }
    Ok(ticker.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Unit tests (offline — no feature gate required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_query_normalises_symbol() {
        let q = TiingoHistoricalQuery::new("aapl").unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn historical_query_rejects_empty_symbol() {
        assert_eq!(
            TiingoHistoricalQuery::new(""),
            Err(TiingoProviderError::EmptyTicker)
        );
        assert_eq!(
            TiingoHistoricalQuery::new("   "),
            Err(TiingoProviderError::EmptyTicker)
        );
    }

    #[test]
    fn historical_query_rejects_special_characters() {
        assert_eq!(
            TiingoHistoricalQuery::new("AAPL/../../secret"),
            Err(TiingoProviderError::InvalidTicker)
        );
        assert_eq!(
            TiingoHistoricalQuery::new("AAPL?token=x"),
            Err(TiingoProviderError::InvalidTicker)
        );
    }

    #[test]
    fn news_query_normalises_tickers() {
        let q =
            TiingoNewsQuery::new(&["aapl", "msft"]).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(q.tickers, vec!["AAPL", "MSFT"]);
    }

    #[test]
    fn news_query_rejects_empty_list() {
        assert_eq!(
            TiingoNewsQuery::new(&[]),
            Err(TiingoProviderError::EmptyTickerList)
        );
    }

    #[test]
    fn news_query_rejects_invalid_ticker_in_list() {
        assert_eq!(
            TiingoNewsQuery::new(&["AAPL", "BAD TICKER"]),
            Err(TiingoProviderError::InvalidTicker)
        );
    }

    #[test]
    fn error_messages_are_human_readable() {
        assert!(
            TiingoProviderError::EmptyTicker
                .to_string()
                .contains("empty")
        );
        assert!(
            TiingoProviderError::InvalidTicker
                .to_string()
                .contains("unsupported")
        );
        assert!(
            TiingoProviderError::EmptyTickerList
                .to_string()
                .contains("at least one")
        );
        let provider_err = TiingoProviderError::Provider("boom".to_string());
        assert!(provider_err.to_string().contains("boom"));
    }
}
