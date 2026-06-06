#![forbid(unsafe_code)]

//! Benzinga provider — offline validation, query structs, and stub fetcher.
//!
//! The real HTTP implementation is behind the `http` feature
//! (`src/http_fetcher.rs`). The crate compiles and tests correctly
//! without network access by default.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{BenzingaEarningsHttpFetcher, BenzingaNewsHttpFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "benzinga";
pub const BASE_URL: &str = "https://api.benzinga.com/api/v2";
pub const API_KEY_ENV: &str = "TDW_BENZINGA_API_KEY";

pub type Result<T> = std::result::Result<T, BenzingaProviderError>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BenzingaProviderError {
    #[error("benzinga symbol must not be empty")]
    EmptySymbol,
    #[error("benzinga symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("benzinga date must be YYYY-MM-DD, got: {0}")]
    InvalidDate(String),
    #[error("benzinga page_size must be between 1 and 100")]
    InvalidPageSize,
    #[error("benzinga api key env TDW_BENZINGA_API_KEY must be set")]
    MissingApiKey,
    #[error("benzinga provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query parameters for the Benzinga `/news` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BenzingaNewsQuery {
    pub symbol: String,
    pub page_size: u32,
}

impl BenzingaNewsQuery {
    /// Construct and validate a news query.
    ///
    /// # Errors
    ///
    /// Returns [`BenzingaProviderError`] when `symbol` is empty, contains
    /// unsupported characters, or `page_size` is outside `1..=100`.
    pub fn new(symbol: &str, page_size: u32) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            page_size: validate_page_size(page_size)?,
        })
    }
}

/// Query parameters for the Benzinga `/calendar/earnings` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BenzingaEarningsQuery {
    pub symbol: String,
    pub date_from: String,
    pub date_to: String,
}

impl BenzingaEarningsQuery {
    /// Construct and validate an earnings query.
    ///
    /// # Errors
    ///
    /// Returns [`BenzingaProviderError`] when any argument fails validation.
    pub fn new(symbol: &str, date_from: &str, date_to: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            date_from: normalize_date(date_from)?,
            date_to: normalize_date(date_to)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Stub / offline fetcher (always compiled in)
// ---------------------------------------------------------------------------

/// Returns hardcoded stub news items without any network call.
///
/// Useful for unit tests and offline development.
///
/// # Errors
///
/// Returns an error only when `query` validation fails (validated on
/// construction, so this is infallible in practice).
pub fn stub_fetch_news(query: &BenzingaNewsQuery) -> Result<Vec<BenzingaNewsItem>> {
    Ok(vec![
        BenzingaNewsItem {
            id: "stub-001".to_string(),
            title: format!("{} Stub Headline One", query.symbol),
            teaser: "Stub teaser text.".to_string(),
            url: "https://example.com/stub-001".to_string(),
            published_date: "2024-01-25T09:30:00Z".to_string(),
            source: "Benzinga".to_string(),
            stocks: vec![query.symbol.clone()],
        },
        BenzingaNewsItem {
            id: "stub-002".to_string(),
            title: format!("{} Stub Headline Two", query.symbol),
            teaser: "Another stub teaser.".to_string(),
            url: "https://example.com/stub-002".to_string(),
            published_date: "2024-01-24T14:00:00Z".to_string(),
            source: "Benzinga".to_string(),
            stocks: vec![query.symbol.clone()],
        },
    ])
}

/// Returns a hardcoded stub earnings record without any network call.
///
/// # Errors
///
/// Returns an error only when `query` validation fails.
pub fn stub_fetch_earnings(query: &BenzingaEarningsQuery) -> Result<Vec<BenzingaEarningsItem>> {
    Ok(vec![BenzingaEarningsItem {
        id: "stub-e1".to_string(),
        date: query.date_from.clone(),
        ticker: query.symbol.clone(),
        eps: "2.18".to_string(),
        eps_estimate: "2.10".to_string(),
        revenue: "119575000000".to_string(),
    }])
}

// ---------------------------------------------------------------------------
// Shared domain types returned by both stub and HTTP fetchers
// ---------------------------------------------------------------------------

/// A single news article from the Benzinga `/news` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BenzingaNewsItem {
    pub id: String,
    pub title: String,
    pub teaser: String,
    pub url: String,
    #[serde(rename = "publishedDate", alias = "published_date")]
    pub published_date: String,
    pub source: String,
    /// Ticker symbols mentioned in the article.
    pub stocks: Vec<String>,
}

/// A single row from the Benzinga `/calendar/earnings` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BenzingaEarningsItem {
    pub id: String,
    pub date: String,
    pub ticker: String,
    pub eps: String,
    #[serde(rename = "epsEstimate", alias = "eps_estimate")]
    pub eps_estimate: String,
    pub revenue: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(BenzingaProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(BenzingaProviderError::InvalidSymbol);
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
        return Err(BenzingaProviderError::InvalidDate(date.to_string()));
    }
    Ok(date.to_string())
}

fn validate_page_size(page_size: u32) -> Result<u32> {
    if page_size == 0 || page_size > 100 {
        return Err(BenzingaProviderError::InvalidPageSize);
    }
    Ok(page_size)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- BenzingaNewsQuery validation ---

    #[test]
    fn news_query_normalises_symbol_to_uppercase() {
        let query =
            BenzingaNewsQuery::new("aapl", 20).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.symbol, "AAPL");
        assert_eq!(query.page_size, 20);
    }

    #[test]
    fn news_query_rejects_empty_symbol() {
        assert_eq!(
            BenzingaNewsQuery::new("", 20).expect_err("empty symbol should fail"),
            BenzingaProviderError::EmptySymbol
        );
        assert_eq!(
            BenzingaNewsQuery::new("   ", 20).expect_err("blank symbol should fail"),
            BenzingaProviderError::EmptySymbol
        );
    }

    #[test]
    fn news_query_rejects_injection_characters() {
        assert_eq!(
            BenzingaNewsQuery::new("AAPL/../../secret", 20)
                .expect_err("path injection should fail"),
            BenzingaProviderError::InvalidSymbol
        );
        assert_eq!(
            BenzingaNewsQuery::new("AAPL?foo=bar", 20).expect_err("query injection should fail"),
            BenzingaProviderError::InvalidSymbol
        );
    }

    #[test]
    fn news_query_rejects_zero_page_size() {
        assert_eq!(
            BenzingaNewsQuery::new("AAPL", 0).expect_err("zero page size should fail"),
            BenzingaProviderError::InvalidPageSize
        );
    }

    #[test]
    fn news_query_rejects_page_size_above_100() {
        assert_eq!(
            BenzingaNewsQuery::new("AAPL", 101).expect_err("page size above 100 should fail"),
            BenzingaProviderError::InvalidPageSize
        );
    }

    // --- BenzingaEarningsQuery validation ---

    #[test]
    fn earnings_query_builds_with_valid_inputs() {
        let query = BenzingaEarningsQuery::new("aapl", "2024-01-01", "2024-12-31")
            .unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.symbol, "AAPL");
        assert_eq!(query.date_from, "2024-01-01");
        assert_eq!(query.date_to, "2024-12-31");
    }

    #[test]
    fn earnings_query_rejects_malformed_date() {
        assert!(BenzingaEarningsQuery::new("AAPL", "20240101", "2024-12-31").is_err());
        assert!(BenzingaEarningsQuery::new("AAPL", "2024-01-01", "2024/12/31").is_err());
        assert!(BenzingaEarningsQuery::new("AAPL", "2024-01-01", "not-a-date").is_err());
    }

    #[test]
    fn earnings_query_rejects_empty_symbol() {
        assert!(BenzingaEarningsQuery::new("", "2024-01-01", "2024-12-31").is_err());
    }

    // --- Stub fetchers ---

    #[test]
    fn stub_news_returns_two_items_for_valid_query() {
        let query =
            BenzingaNewsQuery::new("TSLA", 5).unwrap_or_else(|e| panic!("should build: {e}"));
        let items = stub_fetch_news(&query).unwrap_or_else(|e| panic!("stub must succeed: {e}"));
        assert_eq!(items.len(), 2);
        assert!(items[0].title.contains("TSLA"));
        assert_eq!(items[0].source, "Benzinga");
    }

    #[test]
    fn stub_earnings_returns_one_item_for_valid_query() {
        let query = BenzingaEarningsQuery::new("MSFT", "2024-01-01", "2024-12-31")
            .unwrap_or_else(|e| panic!("should build: {e}"));
        let items =
            stub_fetch_earnings(&query).unwrap_or_else(|e| panic!("stub must succeed: {e}"));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ticker, "MSFT");
        assert_eq!(items[0].eps, "2.18");
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            BenzingaProviderError::EmptySymbol
                .to_string()
                .contains("empty")
        );
        assert!(
            BenzingaProviderError::InvalidDate("bad".to_string())
                .to_string()
                .contains("bad")
        );
        assert!(
            BenzingaProviderError::MissingApiKey
                .to_string()
                .contains("TDW_BENZINGA_API_KEY")
        );
    }
}
