#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{FinnhubHttpProfileFetcher, FinnhubHttpQuoteSnapshotFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "finnhub";
pub const BASE_URL: &str = "https://finnhub.io/api/v1";
pub const API_KEY_ENV: &str = "TDW_FINNHUB_API_KEY";

pub type Result<T> = std::result::Result<T, FinnhubError>;

/// Query for company profile data from the Finnhub `/stock/profile2` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubProfileQuery {
    pub symbol: String,
}

impl FinnhubProfileQuery {
    /// Construct a validated profile query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptySymbol`] if `symbol` is blank, or
    /// [`FinnhubError::InvalidSymbol`] if it contains characters that are not
    /// ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query for a last-price quote snapshot from the Finnhub `/quote` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubQuoteQuery {
    pub symbol: String,
}

impl FinnhubQuoteQuery {
    /// Construct a validated quote query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptySymbol`] if `symbol` is blank, or
    /// [`FinnhubError::InvalidSymbol`] if it contains characters that are not
    /// ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FinnhubError {
    #[error("finnhub symbol must not be empty")]
    EmptySymbol,
    #[error("finnhub symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("finnhub provider error: {0}")]
    Provider(String),
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(FinnhubError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(FinnhubError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Mock fetchers (offline, for unit tests and feature-flag-off builds)
// ---------------------------------------------------------------------------

/// Offline stub for the company-profile endpoint.
pub struct FinnhubMockProfileFetcher;

impl FinnhubMockProfileFetcher {
    /// Return one hardcoded [`tdw_domain::CompanyProfile`]-compatible row for the
    /// queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FinnhubProfileQuery) -> Result<Vec<FinnhubProfileRow>> {
        Ok(vec![FinnhubProfileRow {
            ticker: query.symbol.clone(),
            name: "Apple Inc".to_string(),
            currency: "USD".to_string(),
            exchange: "NASDAQ NMS - GLOBAL MARKET".to_string(),
            logo: "https://static2.finnhub.io/file/publicdatany/finnhubimage/stock_logo/AAPL.png"
                .to_string(),
            market_capitalization: 3_000_000.0,
        }])
    }
}

/// Offline stub for the quote-snapshot endpoint.
pub struct FinnhubMockQuoteSnapshotFetcher;

impl FinnhubMockQuoteSnapshotFetcher {
    /// Return one hardcoded quote row for the queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FinnhubQuoteQuery) -> Result<Vec<FinnhubQuoteRow>> {
        Ok(vec![FinnhubQuoteRow {
            symbol: query.symbol.clone(),
            c: 189.30,
            d: 1.20,
            dp: 0.638,
            pc: 188.10,
            t: 1_717_200_000,
        }])
    }
}

/// Intermediate row shape for a company-profile response, used to build
/// [`tdw_domain::CompanyProfile`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubProfileRow {
    /// Exchange ticker symbol.
    pub ticker: String,
    /// Company name.
    pub name: String,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Exchange name.
    pub exchange: String,
    /// Logo URL.
    pub logo: String,
    /// Market capitalisation in millions.
    pub market_capitalization: f64,
}

/// Intermediate row shape for a quote response, used to build
/// [`tdw_domain::QuoteSnapshot`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubQuoteRow {
    /// Ticker symbol (supplied by the query, not the response).
    pub symbol: String,
    /// Current price.
    pub c: f64,
    /// Change (absolute).
    pub d: f64,
    /// Change percent.
    pub dp: f64,
    /// Previous close.
    pub pc: f64,
    /// Unix epoch timestamp in seconds.
    pub t: i64,
}

// ---------------------------------------------------------------------------
// Unit tests (no feature gate required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_query_normalises_symbol_to_uppercase() {
        let q =
            FinnhubProfileQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn profile_query_rejects_empty_symbol() {
        assert_eq!(FinnhubProfileQuery::new(""), Err(FinnhubError::EmptySymbol));
        assert_eq!(
            FinnhubProfileQuery::new("   "),
            Err(FinnhubError::EmptySymbol)
        );
    }

    #[test]
    fn profile_query_rejects_invalid_characters() {
        assert_eq!(
            FinnhubProfileQuery::new("AAPL/../../secret"),
            Err(FinnhubError::InvalidSymbol)
        );
        assert_eq!(
            FinnhubProfileQuery::new("AAPL?token=x"),
            Err(FinnhubError::InvalidSymbol)
        );
    }

    #[test]
    fn profile_query_accepts_dots_dashes_underscores() {
        assert!(FinnhubProfileQuery::new("BRK.B").is_ok());
        assert!(FinnhubProfileQuery::new("BRK-B").is_ok());
        assert!(FinnhubProfileQuery::new("SPY_ETF").is_ok());
    }

    #[test]
    fn quote_query_normalises_symbol() {
        let q =
            FinnhubQuoteQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn quote_query_rejects_empty_symbol() {
        assert_eq!(FinnhubQuoteQuery::new(""), Err(FinnhubError::EmptySymbol));
    }

    #[test]
    fn mock_profile_fetcher_returns_stub_row() {
        let q = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
        let rows =
            FinnhubMockProfileFetcher::fetch_stub(&q).unwrap_or_else(|e| panic!("stub: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ticker, "AAPL");
        assert_eq!(rows[0].currency, "USD");
        assert_eq!(rows[0].market_capitalization, 3_000_000.0);
    }

    #[test]
    fn mock_quote_snapshot_fetcher_returns_stub_row() {
        let q = FinnhubQuoteQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
        let rows =
            FinnhubMockQuoteSnapshotFetcher::fetch_stub(&q).unwrap_or_else(|e| panic!("stub: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].c, 189.30);
        assert_eq!(rows[0].t, 1_717_200_000);
    }

    #[test]
    fn finnhub_error_messages_are_descriptive() {
        assert!(FinnhubError::EmptySymbol.to_string().contains("empty"));
        assert!(
            FinnhubError::InvalidSymbol
                .to_string()
                .contains("unsupported")
        );
        assert!(
            FinnhubError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
