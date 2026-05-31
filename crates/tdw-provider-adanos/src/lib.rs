#![forbid(unsafe_code)]

//! Adanos sentiment aggregation provider — offline validation, query structs,
//! mock fetcher, and domain types.
//!
//! The real HTTP implementation lives behind the `http` feature
//! (`src/http_fetcher.rs`). The crate compiles and tests correctly without
//! network access by default.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    AdanosPolymarketHttpFetcher, AdanosSentimentHttpFetcher, AdanosTrendingHttpFetcher,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "adanos";
pub const BASE_URL: &str = "https://api.adanos.io/v1";
pub const API_KEY_ENV: &str = "TDW_ADANOS_API_KEY";

pub type Result<T> = std::result::Result<T, AdanosProviderError>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdanosProviderError {
    #[error("adanos ticker must not be empty")]
    EmptyTicker,
    #[error("adanos ticker contains unsupported characters")]
    InvalidTicker,
    #[error("adanos limit must be between 1 and {0}")]
    InvalidLimit(u32),
    #[error("adanos api key env TDW_ADANOS_API_KEY must be set")]
    MissingApiKey,
    #[error("adanos provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query parameters for the Adanos `/sentiment/stocks/{ticker}` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdanosSentimentQuery {
    pub ticker: String,
}

impl AdanosSentimentQuery {
    /// Construct and validate a sentiment query.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] when `ticker` is empty or contains
    /// unsupported characters.
    pub fn new(ticker: &str) -> Result<Self> {
        Ok(Self {
            ticker: normalize_ticker(ticker)?,
        })
    }
}

/// Query parameters for the Adanos `/trending/stocks` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdanosTrendingQuery {
    pub limit: u32,
}

impl AdanosTrendingQuery {
    /// Construct and validate a trending query.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] when `limit` is outside `1..=100`.
    pub fn new(limit: u32) -> Result<Self> {
        Ok(Self {
            limit: validate_limit(limit, 100)?,
        })
    }
}

/// Query parameters for the Adanos `/polymarket/events` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdanosPolymarketQuery {
    pub limit: u32,
}

impl AdanosPolymarketQuery {
    /// Construct and validate a Polymarket events query.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] when `limit` is outside `1..=50`.
    pub fn new(limit: u32) -> Result<Self> {
        Ok(Self {
            limit: validate_limit(limit, 50)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Mock / offline fetcher (always compiled in)
// ---------------------------------------------------------------------------

/// Returns a hardcoded mock sentiment result without any network call.
///
/// Useful for unit tests and offline development.
///
/// # Errors
///
/// Returns an error only when `query` validation fails (validated on
/// construction, so this is infallible in practice).
pub fn mock_fetch_sentiment(query: &AdanosSentimentQuery) -> Result<AdanosSentimentResult> {
    Ok(AdanosSentimentResult {
        ticker: query.ticker.clone(),
        timestamp: 1_704_153_600,
        sentiment_score: 0.72,
        buzz_score: 85,
        sources: AdanosSentimentSources {
            reddit: 0.68,
            twitter: 0.75,
            news: 0.74,
        },
        mentions: AdanosMentions {
            reddit: 1250,
            twitter: 8500,
            news: 45,
        },
        trend: "bullish".to_string(),
    })
}

/// Returns hardcoded mock trending stocks without any network call.
///
/// # Errors
///
/// Returns an error only when `query` validation fails.
pub fn mock_fetch_trending(query: &AdanosTrendingQuery) -> Result<AdanosTrendingResult> {
    let all = vec![
        AdanosTrendingItem {
            ticker: "NVDA".to_string(),
            buzz_score: 98,
            sentiment_score: 0.85,
            mentions: 25000,
        },
        AdanosTrendingItem {
            ticker: "AAPL".to_string(),
            buzz_score: 85,
            sentiment_score: 0.72,
            mentions: 18000,
        },
    ];
    let items = all
        .into_iter()
        .take(query.limit as usize)
        .collect::<Vec<_>>();
    Ok(AdanosTrendingResult {
        timestamp: 1_704_153_600,
        trending: items,
    })
}

/// Returns hardcoded mock Polymarket events without any network call.
///
/// # Errors
///
/// Returns an error only when `query` validation fails.
pub fn mock_fetch_polymarket(query: &AdanosPolymarketQuery) -> Result<AdanosPolymarketResult> {
    let all = vec![AdanosPolymarketEvent {
        id: "abc123".to_string(),
        title: "Will BTC exceed $100k by Dec 2024?".to_string(),
        probability: 0.42,
        volume: 1_500_000.0,
        expires_at: "2024-12-31T23:59:59Z".to_string(),
    }];
    let events = all
        .into_iter()
        .take(query.limit as usize)
        .collect::<Vec<_>>();
    Ok(AdanosPolymarketResult { events })
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Source-level sentiment scores from the `/sentiment/stocks/{ticker}` response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosSentimentSources {
    pub reddit: f64,
    pub twitter: f64,
    pub news: f64,
}

/// Per-source mention counts from the `/sentiment/stocks/{ticker}` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdanosMentions {
    pub reddit: u64,
    pub twitter: u64,
    pub news: u64,
}

/// Full sentiment result for a single ticker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosSentimentResult {
    pub ticker: String,
    pub timestamp: i64,
    #[serde(rename = "sentimentScore")]
    pub sentiment_score: f64,
    #[serde(rename = "buzzScore")]
    pub buzz_score: u32,
    pub sources: AdanosSentimentSources,
    pub mentions: AdanosMentions,
    pub trend: String,
}

/// A single entry from the `/trending/stocks` response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosTrendingItem {
    pub ticker: String,
    #[serde(rename = "buzzScore")]
    pub buzz_score: u32,
    #[serde(rename = "sentimentScore")]
    pub sentiment_score: f64,
    pub mentions: u64,
}

/// Full trending result from `/trending/stocks`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosTrendingResult {
    pub timestamp: i64,
    pub trending: Vec<AdanosTrendingItem>,
}

/// A single Polymarket event from the `/polymarket/events` response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosPolymarketEvent {
    pub id: String,
    pub title: String,
    pub probability: f64,
    pub volume: f64,
    #[serde(rename = "expiresAt")]
    pub expires_at: String,
}

/// Full Polymarket result from `/polymarket/events`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdanosPolymarketResult {
    pub events: Vec<AdanosPolymarketEvent>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(AdanosProviderError::EmptyTicker);
    }
    if !ticker
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(AdanosProviderError::InvalidTicker);
    }
    Ok(ticker.to_ascii_uppercase())
}

fn validate_limit(limit: u32, max: u32) -> Result<u32> {
    if limit == 0 || limit > max {
        return Err(AdanosProviderError::InvalidLimit(max));
    }
    Ok(limit)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- AdanosSentimentQuery validation ---

    #[test]
    fn sentiment_query_normalises_ticker_to_uppercase() {
        let query =
            AdanosSentimentQuery::new("aapl").unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.ticker, "AAPL");
    }

    #[test]
    fn sentiment_query_rejects_empty_ticker() {
        assert_eq!(
            AdanosSentimentQuery::new("").expect_err("empty ticker should fail"),
            AdanosProviderError::EmptyTicker
        );
        assert_eq!(
            AdanosSentimentQuery::new("   ").expect_err("blank ticker should fail"),
            AdanosProviderError::EmptyTicker
        );
    }

    #[test]
    fn sentiment_query_rejects_injection_characters() {
        assert_eq!(
            AdanosSentimentQuery::new("AAPL/../../secret").expect_err("path injection should fail"),
            AdanosProviderError::InvalidTicker
        );
        assert_eq!(
            AdanosSentimentQuery::new("AAPL?foo=bar").expect_err("query injection should fail"),
            AdanosProviderError::InvalidTicker
        );
    }

    // --- AdanosTrendingQuery validation ---

    #[test]
    fn trending_query_builds_with_valid_limit() {
        let query = AdanosTrendingQuery::new(20).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.limit, 20);
    }

    #[test]
    fn trending_query_rejects_zero_limit() {
        assert_eq!(
            AdanosTrendingQuery::new(0).expect_err("zero limit should fail"),
            AdanosProviderError::InvalidLimit(100)
        );
    }

    #[test]
    fn trending_query_rejects_limit_above_100() {
        assert_eq!(
            AdanosTrendingQuery::new(101).expect_err("limit above 100 should fail"),
            AdanosProviderError::InvalidLimit(100)
        );
    }

    #[test]
    fn trending_query_accepts_limit_of_100() {
        let query = AdanosTrendingQuery::new(100).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.limit, 100);
    }

    // --- AdanosPolymarketQuery validation ---

    #[test]
    fn polymarket_query_builds_with_valid_limit() {
        let query = AdanosPolymarketQuery::new(10).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.limit, 10);
    }

    #[test]
    fn polymarket_query_rejects_zero_limit() {
        assert_eq!(
            AdanosPolymarketQuery::new(0).expect_err("zero limit should fail"),
            AdanosProviderError::InvalidLimit(50)
        );
    }

    #[test]
    fn polymarket_query_rejects_limit_above_50() {
        assert_eq!(
            AdanosPolymarketQuery::new(51).expect_err("limit above 50 should fail"),
            AdanosProviderError::InvalidLimit(50)
        );
    }

    #[test]
    fn polymarket_query_accepts_limit_of_50() {
        let query = AdanosPolymarketQuery::new(50).unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.limit, 50);
    }

    // --- Mock fetchers ---

    #[test]
    fn mock_sentiment_returns_result_for_valid_query() {
        let query =
            AdanosSentimentQuery::new("AAPL").unwrap_or_else(|e| panic!("should build: {e}"));
        let result =
            mock_fetch_sentiment(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(result.ticker, "AAPL");
        assert_eq!(result.trend, "bullish");
        assert_eq!(result.buzz_score, 85);
    }

    #[test]
    fn mock_trending_returns_items_up_to_limit() {
        let query = AdanosTrendingQuery::new(1).unwrap_or_else(|e| panic!("should build: {e}"));
        let result =
            mock_fetch_trending(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(result.trending.len(), 1);
        assert_eq!(result.trending[0].ticker, "NVDA");
    }

    #[test]
    fn mock_trending_returns_all_when_limit_covers_all() {
        let query = AdanosTrendingQuery::new(20).unwrap_or_else(|e| panic!("should build: {e}"));
        let result =
            mock_fetch_trending(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(result.trending.len(), 2);
    }

    #[test]
    fn mock_polymarket_returns_events_up_to_limit() {
        let query = AdanosPolymarketQuery::new(5).unwrap_or_else(|e| panic!("should build: {e}"));
        let result =
            mock_fetch_polymarket(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].id, "abc123");
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            AdanosProviderError::EmptyTicker
                .to_string()
                .contains("empty")
        );
        assert!(
            AdanosProviderError::InvalidLimit(100)
                .to_string()
                .contains("100")
        );
        assert!(
            AdanosProviderError::MissingApiKey
                .to_string()
                .contains("TDW_ADANOS_API_KEY")
        );
    }
}
