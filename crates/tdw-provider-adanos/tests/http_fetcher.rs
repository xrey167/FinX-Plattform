//! Tests for the Adanos HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes inline. The live tests are additionally gated by
//! `TDW_ADANOS_LIVE=1` and require `TDW_ADANOS_API_KEY`.

#![cfg(feature = "http")]

use tdw_core::{Credentials, Fetcher};
use tdw_provider_adanos::{
    AdanosPolymarketQuery, AdanosPolymarketResult, AdanosSentimentQuery, AdanosSentimentResult,
    AdanosTrendingQuery, AdanosTrendingResult,
    http_fetcher::{
        AdanosPolymarketHttpFetcher, AdanosSentimentHttpFetcher, AdanosTrendingHttpFetcher,
    },
};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn sentiment_cassette_json() -> &'static str {
    r#"{
        "ticker": "AAPL",
        "timestamp": 1704153600,
        "sentimentScore": 0.72,
        "buzzScore": 85,
        "sources": {
            "reddit": 0.68,
            "twitter": 0.75,
            "news": 0.74
        },
        "mentions": {
            "reddit": 1250,
            "twitter": 8500,
            "news": 45
        },
        "trend": "bullish"
    }"#
}

fn trending_cassette_json() -> &'static str {
    r#"{
        "timestamp": 1704153600,
        "trending": [
            {
                "ticker": "NVDA",
                "buzzScore": 98,
                "sentimentScore": 0.85,
                "mentions": 25000
            },
            {
                "ticker": "AAPL",
                "buzzScore": 85,
                "sentimentScore": 0.72,
                "mentions": 18000
            }
        ]
    }"#
}

fn polymarket_cassette_json() -> &'static str {
    r#"{
        "events": [
            {
                "id": "abc123",
                "title": "Will BTC exceed $100k by Dec 2024?",
                "probability": 0.42,
                "volume": 1500000.0,
                "expiresAt": "2024-12-31T23:59:59Z"
            }
        ]
    }"#
}

// ---------------------------------------------------------------------------
// Query validation (re-tested from the integration binary's perspective)
// ---------------------------------------------------------------------------

#[test]
fn sentiment_query_rejects_empty_ticker() {
    assert!(AdanosSentimentQuery::new("").is_err());
}

#[test]
fn sentiment_query_rejects_path_injection() {
    assert!(AdanosSentimentQuery::new("AAPL/../secret").is_err());
}

#[test]
fn trending_query_rejects_zero_limit() {
    assert!(AdanosTrendingQuery::new(0).is_err());
}

#[test]
fn trending_query_rejects_limit_above_100() {
    assert!(AdanosTrendingQuery::new(101).is_err());
}

#[test]
fn polymarket_query_rejects_zero_limit() {
    assert!(AdanosPolymarketQuery::new(0).is_err());
}

#[test]
fn polymarket_query_rejects_limit_above_50() {
    assert!(AdanosPolymarketQuery::new(51).is_err());
}

// ---------------------------------------------------------------------------
// Cassette: deserialise sentiment JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_sentiment_deserialises_correctly() {
    let result: AdanosSentimentResult =
        serde_json::from_str(sentiment_cassette_json()).expect("cassette sentiment must parse");

    assert_eq!(result.ticker, "AAPL");
    assert_eq!(result.timestamp, 1_704_153_600);
    assert_eq!(result.buzz_score, 85);
    assert_eq!(result.trend, "bullish");
    assert!((result.sentiment_score - 0.72).abs() < f64::EPSILON);
    assert!((result.sources.reddit - 0.68).abs() < f64::EPSILON);
    assert_eq!(result.mentions.reddit, 1250);
    assert_eq!(result.mentions.twitter, 8500);
    assert_eq!(result.mentions.news, 45);
}

// ---------------------------------------------------------------------------
// Cassette: deserialise trending JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_trending_deserialises_two_items() {
    let result: AdanosTrendingResult =
        serde_json::from_str(trending_cassette_json()).expect("cassette trending must parse");

    assert_eq!(result.timestamp, 1_704_153_600);
    assert_eq!(result.trending.len(), 2);
    assert_eq!(result.trending[0].ticker, "NVDA");
    assert_eq!(result.trending[0].buzz_score, 98);
    assert_eq!(result.trending[0].mentions, 25000);
    assert_eq!(result.trending[1].ticker, "AAPL");
}

// ---------------------------------------------------------------------------
// Cassette: deserialise polymarket JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_polymarket_deserialises_one_event() {
    let result: AdanosPolymarketResult =
        serde_json::from_str(polymarket_cassette_json()).expect("cassette polymarket must parse");

    assert_eq!(result.events.len(), 1);
    let event = &result.events[0];
    assert_eq!(event.id, "abc123");
    assert_eq!(event.title, "Will BTC exceed $100k by Dec 2024?");
    assert!((event.probability - 0.42).abs() < f64::EPSILON);
    assert!((event.volume - 1_500_000.0).abs() < f64::EPSILON);
    assert_eq!(event.expires_at, "2024-12-31T23:59:59Z");
}

// ---------------------------------------------------------------------------
// Missing API key surfaces AdanosProviderError::MissingApiKey
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_api_key_returns_error_without_network_call() {
    // Skip this test when TDW_ADANOS_API_KEY is already set in the environment
    // (e.g. a developer machine running live tests). Manipulating env vars
    // requires unsafe, which is forbidden by workspace lints.
    if std::env::var("TDW_ADANOS_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        eprintln!("TDW_ADANOS_API_KEY is set; skipping missing-key test");
        return;
    }

    let fetcher = AdanosSentimentHttpFetcher::default();
    let creds = Credentials::default();
    let err = fetcher
        .fetch(serde_json::json!({ "ticker": "AAPL" }), &creds)
        .await
        .expect_err("missing key must return error");

    assert!(
        err.to_string().contains("TDW_ADANOS_API_KEY"),
        "missing-key error should mention the env var, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Live integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_adanos_sentiment_returns_data_when_env_var_set() {
    if std::env::var("TDW_ADANOS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ADANOS_LIVE != 1; skipping live Adanos sentiment integration test");
        return;
    }

    let fetcher = AdanosSentimentHttpFetcher::default();
    let creds = Credentials::default();
    let result = fetcher
        .fetch(serde_json::json!({ "ticker": "AAPL" }), &creds)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert_eq!(result.provider, "adanos");
    assert_eq!(result.endpoint, "sentiment");
    let row = result
        .rows
        .first()
        .expect("live response must include a row");
    assert_eq!(row.ticker, "AAPL");
    assert!(!row.trend.is_empty(), "trend must be non-empty");
}

#[tokio::test]
async fn live_adanos_trending_returns_data_when_env_var_set() {
    if std::env::var("TDW_ADANOS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ADANOS_LIVE != 1; skipping live Adanos trending integration test");
        return;
    }

    let fetcher = AdanosTrendingHttpFetcher::default();
    let creds = Credentials::default();
    let result = fetcher
        .fetch(serde_json::json!({ "limit": 10 }), &creds)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert_eq!(result.endpoint, "trending");
    assert!(
        !result.rows.is_empty(),
        "live response must include at least one trending stock"
    );
}

#[tokio::test]
async fn live_adanos_polymarket_returns_data_when_env_var_set() {
    if std::env::var("TDW_ADANOS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ADANOS_LIVE != 1; skipping live Adanos polymarket integration test");
        return;
    }

    let fetcher = AdanosPolymarketHttpFetcher::default();
    let creds = Credentials::default();
    let result = fetcher
        .fetch(serde_json::json!({ "limit": 5 }), &creds)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert_eq!(result.endpoint, "polymarket");
    assert!(
        !result.rows.is_empty(),
        "live response must include at least one polymarket event"
    );
}
