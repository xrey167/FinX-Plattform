#![forbid(unsafe_code)]

//! Seeking Alpha provider — offline validation, query structs, and stub fetcher.
//!
//! The real HTTP implementation is behind the `http` feature
//! (`src/http_fetcher.rs`). The crate compiles and tests correctly
//! without network access by default.
//!
//! Auth uses RapidAPI headers:
//! - `x-rapidapi-key: {TDW_SEEKING_ALPHA_API_KEY}`
//! - `x-rapidapi-host: seeking-alpha.p.rapidapi.com`

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{SeekingAlphaArticlesHttpFetcher, SeekingAlphaRatingsHttpFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "seeking-alpha";
pub const BASE_URL: &str = "https://seeking-alpha.p.rapidapi.com";
pub const RAPIDAPI_KEY_ENV: &str = "TDW_SEEKING_ALPHA_API_KEY";
pub const RAPIDAPI_HOST: &str = "seeking-alpha.p.rapidapi.com";

/// Maximum allowed article page size.
pub const MAX_ARTICLE_SIZE: u32 = 40;

pub type Result<T> = std::result::Result<T, SeekingAlphaProviderError>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the Seeking Alpha provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SeekingAlphaProviderError {
    #[error("seeking-alpha ticker must not be empty")]
    EmptyTicker,
    #[error("seeking-alpha ticker contains unsupported characters")]
    InvalidTicker,
    #[error("seeking-alpha article size must be between 1 and {MAX_ARTICLE_SIZE}")]
    InvalidSize,
    #[error("seeking-alpha api key env TDW_SEEKING_ALPHA_API_KEY must be set")]
    MissingApiKey,
    #[error("seeking-alpha provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query parameters for the Seeking Alpha `/analysis/v2/list` endpoint.
///
/// Returns analyst articles for the given ticker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeekingAlphaArticlesQuery {
    pub ticker: String,
    /// Number of articles to return (1–40).
    pub size: u32,
}

impl SeekingAlphaArticlesQuery {
    /// Construct and validate an articles query.
    ///
    /// # Errors
    ///
    /// Returns [`SeekingAlphaProviderError`] when `ticker` is empty, contains
    /// unsupported characters, or `size` is outside `1..=40`.
    pub fn new(ticker: &str, size: u32) -> Result<Self> {
        Ok(Self {
            ticker: normalize_ticker(ticker)?,
            size: validate_size(size)?,
        })
    }
}

/// Query parameters for the Seeking Alpha `/symbols/v1/summary` endpoint.
///
/// Returns quant, author, and sell-side ratings for the given ticker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeekingAlphaRatingsQuery {
    pub ticker: String,
}

impl SeekingAlphaRatingsQuery {
    /// Construct and validate a ratings query.
    ///
    /// # Errors
    ///
    /// Returns [`SeekingAlphaProviderError`] when `ticker` is empty or
    /// contains unsupported characters.
    pub fn new(ticker: &str) -> Result<Self> {
        Ok(Self {
            ticker: normalize_ticker(ticker)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A single analyst article returned by `/analysis/v2/list`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SeekingAlphaArticle {
    pub id: String,
    pub title: String,
    pub publish_on: String,
    pub is_locked_pro: bool,
    pub comment_count: u32,
}

/// Ratings snapshot returned by `/symbols/v1/summary`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SeekingAlphaRatings {
    pub ticker: String,
    pub quant_rating: f64,
    pub authors_rating: f64,
    pub sell_side_rating: f64,
    pub quant_rating_change: String,
}

// ---------------------------------------------------------------------------
// Stub / offline fetcher (always compiled in)
// ---------------------------------------------------------------------------

/// Returns hardcoded stub articles without any network call.
///
/// Useful for unit tests and offline development.
///
/// # Errors
///
/// Infallible in practice — validation already runs at query construction.
pub fn stub_fetch_articles(query: &SeekingAlphaArticlesQuery) -> Result<Vec<SeekingAlphaArticle>> {
    Ok(vec![
        SeekingAlphaArticle {
            id: "stub-001".to_string(),
            title: format!("{} Q1 2024 Earnings: Strong Beat", query.ticker),
            publish_on: "2024-01-25T20:30:00Z".to_string(),
            is_locked_pro: false,
            comment_count: 45,
        },
        SeekingAlphaArticle {
            id: "stub-002".to_string(),
            title: format!("{} Outlook 2024: Cautious Optimism", query.ticker),
            publish_on: "2024-01-20T14:00:00Z".to_string(),
            is_locked_pro: true,
            comment_count: 12,
        },
    ])
}

/// Returns a hardcoded stub ratings record without any network call.
///
/// # Errors
///
/// Infallible in practice — validation already runs at query construction.
pub fn stub_fetch_ratings(query: &SeekingAlphaRatingsQuery) -> Result<SeekingAlphaRatings> {
    Ok(SeekingAlphaRatings {
        ticker: query.ticker.clone(),
        quant_rating: 4.12,
        authors_rating: 3.85,
        sell_side_rating: 4.21,
        quant_rating_change: "up".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(SeekingAlphaProviderError::EmptyTicker);
    }
    if !ticker
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(SeekingAlphaProviderError::InvalidTicker);
    }
    Ok(ticker.to_ascii_uppercase())
}

fn validate_size(size: u32) -> Result<u32> {
    if size == 0 || size > MAX_ARTICLE_SIZE {
        return Err(SeekingAlphaProviderError::InvalidSize);
    }
    Ok(size)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- SeekingAlphaArticlesQuery validation ---

    #[test]
    fn articles_query_normalises_ticker_to_uppercase() {
        let query = SeekingAlphaArticlesQuery::new("aapl", 5)
            .unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.ticker, "AAPL");
        assert_eq!(query.size, 5);
    }

    #[test]
    fn articles_query_rejects_empty_ticker() {
        assert_eq!(
            SeekingAlphaArticlesQuery::new("", 5).expect_err("empty ticker should fail"),
            SeekingAlphaProviderError::EmptyTicker
        );
        assert_eq!(
            SeekingAlphaArticlesQuery::new("   ", 5).expect_err("blank ticker should fail"),
            SeekingAlphaProviderError::EmptyTicker
        );
    }

    #[test]
    fn articles_query_rejects_injection_characters() {
        assert_eq!(
            SeekingAlphaArticlesQuery::new("AAPL/../../secret", 5)
                .expect_err("path injection should fail"),
            SeekingAlphaProviderError::InvalidTicker
        );
        assert_eq!(
            SeekingAlphaArticlesQuery::new("AAPL?foo=bar", 5)
                .expect_err("query injection should fail"),
            SeekingAlphaProviderError::InvalidTicker
        );
    }

    #[test]
    fn articles_query_rejects_zero_size() {
        assert_eq!(
            SeekingAlphaArticlesQuery::new("AAPL", 0).expect_err("zero size should fail"),
            SeekingAlphaProviderError::InvalidSize
        );
    }

    #[test]
    fn articles_query_rejects_size_above_max() {
        assert_eq!(
            SeekingAlphaArticlesQuery::new("AAPL", MAX_ARTICLE_SIZE + 1)
                .expect_err("size above max should fail"),
            SeekingAlphaProviderError::InvalidSize
        );
    }

    #[test]
    fn articles_query_accepts_max_size() {
        let query = SeekingAlphaArticlesQuery::new("AAPL", MAX_ARTICLE_SIZE)
            .unwrap_or_else(|e| panic!("max size should be accepted: {e}"));
        assert_eq!(query.size, MAX_ARTICLE_SIZE);
    }

    // --- SeekingAlphaRatingsQuery validation ---

    #[test]
    fn ratings_query_normalises_ticker_to_uppercase() {
        let query =
            SeekingAlphaRatingsQuery::new("msft").unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.ticker, "MSFT");
    }

    #[test]
    fn ratings_query_rejects_injection_characters() {
        assert!(SeekingAlphaRatingsQuery::new("AAPL/../../secret").is_err());
        assert!(SeekingAlphaRatingsQuery::new("AAPL&key=val").is_err());
    }

    #[test]
    fn ratings_query_rejects_empty_ticker() {
        assert_eq!(
            SeekingAlphaRatingsQuery::new("").expect_err("empty ticker should fail"),
            SeekingAlphaProviderError::EmptyTicker
        );
    }

    // --- Stub fetchers ---

    #[test]
    fn stub_articles_returns_two_items_for_valid_query() {
        let query = SeekingAlphaArticlesQuery::new("TSLA", 5)
            .unwrap_or_else(|e| panic!("should build: {e}"));
        let items =
            stub_fetch_articles(&query).unwrap_or_else(|e| panic!("stub must succeed: {e}"));
        assert_eq!(items.len(), 2);
        assert!(items[0].title.contains("TSLA"));
        assert!(!items[0].is_locked_pro);
        assert!(items[1].is_locked_pro);
    }

    #[test]
    fn stub_ratings_returns_record_for_valid_query() {
        let query =
            SeekingAlphaRatingsQuery::new("AAPL").unwrap_or_else(|e| panic!("should build: {e}"));
        let ratings =
            stub_fetch_ratings(&query).unwrap_or_else(|e| panic!("stub must succeed: {e}"));
        assert_eq!(ratings.ticker, "AAPL");
        assert_eq!(ratings.quant_rating_change, "up");
        assert!(ratings.quant_rating > 0.0);
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            SeekingAlphaProviderError::EmptyTicker
                .to_string()
                .contains("empty")
        );
        assert!(
            SeekingAlphaProviderError::MissingApiKey
                .to_string()
                .contains("TDW_SEEKING_ALPHA_API_KEY")
        );
        assert!(
            SeekingAlphaProviderError::InvalidSize
                .to_string()
                .contains("40")
        );
    }
}
