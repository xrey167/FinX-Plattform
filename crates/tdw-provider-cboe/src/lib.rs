#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    CboeHttpIndexDirectoryFetcher, CboeHttpIndexFetcher, CboeHttpIndexSnapshotFetcher,
    CboeHttpOptionsChainFetcher, CboeHttpOptionsFetcher,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "cboe";
pub const BASE_URL: &str = "https://cdn.cboe.com/api/global";

pub type Result<T> = std::result::Result<T, CboeProviderError>;

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Query parameters for the CBOE delayed options chain endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CboeOptionsQuery {
    /// Underlying symbol (e.g. "AAPL"). Normalised to uppercase ASCII.
    pub symbol: String,
}

impl CboeOptionsQuery {
    /// Construct and validate a new options query.
    ///
    /// # Errors
    ///
    /// Returns [`CboeProviderError::EmptySymbol`] when `symbol` is blank, or
    /// [`CboeProviderError::InvalidSymbol`] when it contains non-ASCII-alphanumeric
    /// characters (dots and hyphens are also allowed).
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query parameters for the CBOE US-index quote endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CboeIndexQuery {
    /// Index ticker (e.g. "VIX", "SPX"). Must be uppercase ASCII letters only.
    pub index: String,
}

impl CboeIndexQuery {
    /// Construct and validate a new index query.
    ///
    /// # Errors
    ///
    /// Returns [`CboeProviderError::EmptyIndex`] when `index` is blank, or
    /// [`CboeProviderError::InvalidIndex`] when it is not purely uppercase ASCII letters.
    pub fn new(index: &str) -> Result<Self> {
        Ok(Self {
            index: normalize_index(index)?,
        })
    }
}

/// Query parameters for the CBOE index-directory endpoint.
///
/// Backs both `index/available` (full directory listing) and `index/search`
/// (case-insensitive substring filter over the symbol and name). An empty
/// `query` lists every index; a non-empty `query` keeps only matching rows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CboeIndexDirectoryQuery {
    /// Optional case-insensitive substring filter over symbol/name. Empty =
    /// list all indices.
    #[serde(default)]
    pub query: String,
}

impl CboeIndexDirectoryQuery {
    /// Construct a directory query from an optional free-text filter.
    #[must_use]
    pub fn new(query: &str) -> Self {
        Self {
            query: query.trim().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Response / domain types
// ---------------------------------------------------------------------------

/// A single option contract from the CBOE delayed options chain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CboeOptionContract {
    pub option: String,
    pub bid: f64,
    pub ask: f64,
    pub iv: f64,
    pub delta: f64,
    pub gamma: f64,
    pub theta: f64,
    pub open_interest: u64,
}

/// A US-index quote from CBOE.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CboeIndexQuote {
    pub symbol: String,
    pub price: f64,
    pub change: f64,
    pub volume: u64,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CboeProviderError {
    #[error("cboe options symbol must not be empty")]
    EmptySymbol,
    #[error("cboe options symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("cboe index must not be empty")]
    EmptyIndex,
    #[error("cboe index must be uppercase ASCII letters only")]
    InvalidIndex,
    #[error("cboe provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Request helpers
// ---------------------------------------------------------------------------

/// Build the path for a delayed options chain request without performing I/O.
///
/// # Errors
///
/// Propagates symbol validation errors.
pub fn options_request_path(symbol: &str) -> Result<String> {
    let symbol = normalize_symbol(symbol)?;
    Ok(format!("/delayed_quotes/options/{symbol}"))
}

/// Build the path for a US-index quote request without performing I/O.
///
/// # Errors
///
/// Propagates index validation errors.
pub fn index_request_path(index: &str) -> Result<String> {
    let index = normalize_index(index)?;
    Ok(format!("/us_indices/quotes/{index}"))
}

/// Build the path for the CBOE index-directory (definitions) request.
///
/// The directory is a static keyless CDN document listing every CBOE index
/// (name, symbol, description); the same document backs both `index/available`
/// and `index/search`, so the path takes no parameter.
#[must_use]
pub const fn index_directory_path() -> &'static str {
    "/us_indices/definitions/all_us_indices.json"
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(CboeProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        return Err(CboeProviderError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

fn normalize_index(index: &str) -> Result<String> {
    let index = index.trim();
    if index.is_empty() {
        return Err(CboeProviderError::EmptyIndex);
    }
    if !index.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(CboeProviderError::InvalidIndex);
    }
    Ok(index.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Mock offline fetcher (no I/O, for unit tests / offline CI)
// ---------------------------------------------------------------------------

/// Returns a hard-coded stub options chain for any valid symbol.
///
/// # Errors
///
/// Propagates symbol validation errors.
pub fn stub_options_chain(symbol: &str) -> Result<Vec<CboeOptionContract>> {
    let symbol = normalize_symbol(symbol)?;
    Ok(vec![CboeOptionContract {
        option: format!("{symbol}240119C00180000"),
        bid: 5.10,
        ask: 5.30,
        iv: 0.235,
        delta: 0.45,
        gamma: 0.02,
        theta: -0.05,
        open_interest: 12_500,
    }])
}

/// Returns a hard-coded stub index quote for any valid index ticker.
///
/// # Errors
///
/// Propagates index validation errors.
pub fn stub_index_quote(index: &str) -> Result<CboeIndexQuote> {
    let index = normalize_index(index)?;
    Ok(CboeIndexQuote {
        symbol: index,
        price: 13.45,
        change: -0.23,
        volume: 0,
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- CboeOptionsQuery validation ---

    #[test]
    fn options_query_normalises_symbol_to_uppercase() {
        let query =
            CboeOptionsQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.symbol, "AAPL");
    }

    #[test]
    fn options_query_rejects_empty_symbol() {
        assert_eq!(
            CboeOptionsQuery::new(""),
            Err(CboeProviderError::EmptySymbol)
        );
        assert_eq!(
            CboeOptionsQuery::new("   "),
            Err(CboeProviderError::EmptySymbol)
        );
    }

    #[test]
    fn options_query_rejects_special_characters() {
        assert_eq!(
            CboeOptionsQuery::new("AAPL/../../etc"),
            Err(CboeProviderError::InvalidSymbol)
        );
        assert_eq!(
            CboeOptionsQuery::new("AAPL?foo=bar"),
            Err(CboeProviderError::InvalidSymbol)
        );
    }

    #[test]
    fn options_query_allows_dot_and_hyphen() {
        let query =
            CboeOptionsQuery::new("BRK.B").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.symbol, "BRK.B");
    }

    // --- CboeIndexQuery validation ---

    #[test]
    fn index_query_normalises_to_uppercase() {
        let query =
            CboeIndexQuery::new("vix").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.index, "VIX");
    }

    #[test]
    fn index_query_rejects_empty_index() {
        assert_eq!(CboeIndexQuery::new(""), Err(CboeProviderError::EmptyIndex));
        assert_eq!(
            CboeIndexQuery::new("  "),
            Err(CboeProviderError::EmptyIndex)
        );
    }

    #[test]
    fn index_query_rejects_digits_and_special_chars() {
        assert_eq!(
            CboeIndexQuery::new("VIX2"),
            Err(CboeProviderError::InvalidIndex)
        );
        assert_eq!(
            CboeIndexQuery::new("SP/X"),
            Err(CboeProviderError::InvalidIndex)
        );
    }

    // --- Request path helpers ---

    #[test]
    fn options_request_path_contains_symbol() {
        let path =
            options_request_path("aapl").unwrap_or_else(|e| panic!("path should build: {e}"));
        assert_eq!(path, "/delayed_quotes/options/AAPL");
    }

    #[test]
    fn index_request_path_contains_index() {
        let path = index_request_path("vix").unwrap_or_else(|e| panic!("path should build: {e}"));
        assert_eq!(path, "/us_indices/quotes/VIX");
    }

    #[test]
    fn options_request_path_rejects_path_injection() {
        assert!(options_request_path("../secret").is_err());
    }

    // --- Stub fetchers ---

    #[test]
    fn stub_options_chain_returns_contract_for_valid_symbol() {
        let contracts =
            stub_options_chain("AAPL").unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert!(!contracts.is_empty());
        assert!(contracts[0].option.starts_with("AAPL"));
        assert_eq!(contracts[0].open_interest, 12_500);
    }

    #[test]
    fn stub_options_chain_propagates_validation_error() {
        assert!(stub_options_chain("").is_err());
    }

    #[test]
    fn stub_index_quote_returns_quote_for_valid_index() {
        let quote = stub_index_quote("VIX").unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(quote.symbol, "VIX");
        assert!(quote.price > 0.0);
    }

    #[test]
    fn stub_index_quote_propagates_validation_error() {
        assert!(stub_index_quote("").is_err());
        assert!(stub_index_quote("VIX2").is_err());
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_descriptive() {
        assert_eq!(
            CboeProviderError::EmptySymbol.to_string(),
            "cboe options symbol must not be empty"
        );
        assert_eq!(
            CboeProviderError::InvalidIndex.to_string(),
            "cboe index must be uppercase ASCII letters only"
        );
    }
}
