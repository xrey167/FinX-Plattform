#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{TradierHttpOptionsFetcher, TradierHttpQuoteFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "tradier";
pub const BASE_URL: &str = "https://api.tradier.com/v1";
pub const API_KEY_ENV: &str = "TDW_TRADIER_API_KEY";

pub type Result<T> = std::result::Result<T, TradierProviderError>;

/// Query parameters for the Tradier real-time quote endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradierQuoteQuery {
    pub symbol: String,
}

impl TradierQuoteQuery {
    /// Construct a validated quote query.
    ///
    /// # Errors
    ///
    /// Returns [`TradierProviderError::EmptySymbol`] if `symbol` is blank,
    /// or [`TradierProviderError::InvalidSymbol`] if it contains characters
    /// that are not ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query parameters for the Tradier options chain endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradierOptionsQuery {
    pub symbol: String,
    /// Expiration date in `YYYY-MM-DD` format.
    pub expiration: String,
}

impl TradierOptionsQuery {
    /// Construct a validated options chain query.
    ///
    /// # Errors
    ///
    /// Returns [`TradierProviderError::EmptySymbol`] or
    /// [`TradierProviderError::InvalidSymbol`] for bad symbols, and
    /// [`TradierProviderError::InvalidExpiration`] if `expiration` is not
    /// exactly `YYYY-MM-DD`.
    pub fn new(symbol: &str, expiration: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            expiration: normalize_expiration(expiration)?,
        })
    }
}

/// Errors produced by the Tradier provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TradierProviderError {
    #[error("tradier symbol must not be empty")]
    EmptySymbol,
    #[error("tradier symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("tradier expiration date must be YYYY-MM-DD")]
    InvalidExpiration,
    #[error("tradier api key env {0} must be set")]
    MissingApiKey(String),
}

/// Normalise and validate a ticker symbol.
fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(TradierProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(TradierProviderError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

/// Validate an expiration string that must be exactly `YYYY-MM-DD`.
fn normalize_expiration(expiration: &str) -> Result<String> {
    let expiration = expiration.trim();
    let bytes = expiration.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit());
    if !valid {
        return Err(TradierProviderError::InvalidExpiration);
    }
    Ok(expiration.to_string())
}

// ---------------------------------------------------------------------------
// Mock fetcher — returns deterministic stub data without any HTTP I/O.
// ---------------------------------------------------------------------------

/// Stub quote record returned by the mock fetcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TradierQuote {
    pub symbol: String,
    pub last: f64,
    pub bid: f64,
    pub ask: f64,
    pub volume: u64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub change: f64,
}

/// Stub option record returned by the mock fetcher.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TradierOptionContract {
    pub symbol: String,
    pub option_type: String,
    pub strike: f64,
    pub bid: f64,
    pub ask: f64,
    pub open_interest: u64,
    pub volume: u64,
    pub expiration_date: String,
}

/// Return a deterministic stub quote for any symbol.
#[must_use]
pub fn mock_quote(query: &TradierQuoteQuery) -> TradierQuote {
    TradierQuote {
        symbol: query.symbol.clone(),
        last: 185.20,
        bid: 185.15,
        ask: 185.25,
        volume: 55_000_000,
        open: 184.50,
        high: 186.10,
        low: 184.20,
        close: 185.20,
        change: -0.50,
    }
}

/// Return a deterministic stub options chain for any query.
#[must_use]
pub fn mock_options_chain(query: &TradierOptionsQuery) -> Vec<TradierOptionContract> {
    vec![TradierOptionContract {
        symbol: format!(
            "{}{}C00180000",
            query.symbol,
            query.expiration.replace('-', "").get(2..).unwrap_or("")
        ),
        option_type: "call".to_string(),
        strike: 180.0,
        bid: 5.10,
        ask: 5.30,
        open_interest: 12_500,
        volume: 850,
        expiration_date: query.expiration.clone(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- symbol validation ---------------------------------------------------

    #[test]
    fn normalises_lowercase_symbol_to_uppercase() {
        let query =
            TradierQuoteQuery::new("aapl").unwrap_or_else(|e| panic!("should normalise: {e}"));
        assert_eq!(query.symbol, "AAPL");
    }

    #[test]
    fn trims_whitespace_from_symbol() {
        let query =
            TradierQuoteQuery::new("  msft  ").unwrap_or_else(|e| panic!("should trim: {e}"));
        assert_eq!(query.symbol, "MSFT");
    }

    #[test]
    fn rejects_empty_symbol() {
        assert_eq!(
            TradierQuoteQuery::new(""),
            Err(TradierProviderError::EmptySymbol)
        );
        assert_eq!(
            TradierQuoteQuery::new("   "),
            Err(TradierProviderError::EmptySymbol)
        );
    }

    #[test]
    fn rejects_symbol_with_path_injection_characters() {
        assert_eq!(
            TradierQuoteQuery::new("AAPL/../../etc"),
            Err(TradierProviderError::InvalidSymbol)
        );
        assert_eq!(
            TradierQuoteQuery::new("AAPL?greeks=true"),
            Err(TradierProviderError::InvalidSymbol)
        );
    }

    #[test]
    fn accepts_symbol_with_allowed_punctuation() {
        let q = TradierQuoteQuery::new("BRK.B")
            .unwrap_or_else(|e| panic!("dot should be allowed: {e}"));
        assert_eq!(q.symbol, "BRK.B");
    }

    // -- expiration validation -----------------------------------------------

    #[test]
    fn rejects_malformed_expiration_dates() {
        assert_eq!(
            TradierOptionsQuery::new("AAPL", "20240119"),
            Err(TradierProviderError::InvalidExpiration)
        );
        assert_eq!(
            TradierOptionsQuery::new("AAPL", "2024/01/19"),
            Err(TradierProviderError::InvalidExpiration)
        );
        assert_eq!(
            TradierOptionsQuery::new("AAPL", ""),
            Err(TradierProviderError::InvalidExpiration)
        );
    }

    #[test]
    fn accepts_valid_expiration_date() {
        let q = TradierOptionsQuery::new("AAPL", "2024-01-19")
            .unwrap_or_else(|e| panic!("valid expiration: {e}"));
        assert_eq!(q.expiration, "2024-01-19");
        assert_eq!(q.symbol, "AAPL");
    }

    // -- mock fetchers -------------------------------------------------------

    #[test]
    fn mock_quote_returns_stub_data_for_any_symbol() {
        let q = TradierQuoteQuery::new("AAPL").unwrap_or_else(|e| panic!("{e}"));
        let quote = mock_quote(&q);
        assert_eq!(quote.symbol, "AAPL");
        assert_eq!(quote.last, 185.20);
        assert_eq!(quote.volume, 55_000_000);
    }

    #[test]
    fn mock_options_chain_returns_one_call_contract() {
        let q = TradierOptionsQuery::new("AAPL", "2024-01-19").unwrap_or_else(|e| panic!("{e}"));
        let chain = mock_options_chain(&q);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].option_type, "call");
        assert_eq!(chain[0].strike, 180.0);
        assert_eq!(chain[0].expiration_date, "2024-01-19");
    }
}
