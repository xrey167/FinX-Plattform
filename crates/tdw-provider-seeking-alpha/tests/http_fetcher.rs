//! Tests for the Seeking Alpha HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes through the real `transform_data` path (offline, no network). The
//! live tests are additionally gated by `TDW_SEEKING_ALPHA_LIVE=1` and require
//! `TDW_SEEKING_ALPHA_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_seeking_alpha::{
    SeekingAlphaArticlesQuery, SeekingAlphaRatingsQuery,
    http_fetcher::{SeekingAlphaArticlesHttpFetcher, SeekingAlphaRatingsHttpFetcher},
};
use tdw_provider_testkit::live_fetch_obbject_nonempty;

// ---------------------------------------------------------------------------
// Cassette JSON helpers
// ---------------------------------------------------------------------------

fn articles_cassette_json() -> &'static str {
    r#"{
        "data": [
            {
                "id": "abc123",
                "type": "article",
                "attributes": {
                    "title": "Apple Q1 2024 Earnings: Strong Beat",
                    "publishOn": "2024-01-25T20:30:00Z",
                    "isLockedPro": false,
                    "commentCount": 45
                },
                "relationships": {
                    "sentiments": {
                        "data": [{"id": "1", "type": "sentiment"}]
                    }
                }
            }
        ],
        "meta": {
            "page": {"size": 5}
        }
    }"#
}

fn ratings_cassette_json() -> &'static str {
    r#"{
        "data": [
            {
                "id": "AAPL",
                "type": "ticker",
                "attributes": {
                    "quant_rating": 4.12,
                    "authors_rating": 3.85,
                    "sell_side_rating": 4.21,
                    "quant_rating_change": "up"
                }
            }
        ]
    }"#
}

// ---------------------------------------------------------------------------
// Query validation
// ---------------------------------------------------------------------------

#[test]
fn articles_query_rejects_empty_ticker() {
    assert!(SeekingAlphaArticlesQuery::new("", 5).is_err());
}

#[test]
fn articles_query_rejects_path_injection() {
    assert!(SeekingAlphaArticlesQuery::new("AAPL/../secret", 5).is_err());
}

#[test]
fn articles_query_rejects_zero_size() {
    assert!(SeekingAlphaArticlesQuery::new("AAPL", 0).is_err());
}

#[test]
fn articles_query_rejects_size_above_40() {
    assert!(SeekingAlphaArticlesQuery::new("AAPL", 41).is_err());
}

#[test]
fn ratings_query_rejects_empty_ticker() {
    assert!(SeekingAlphaRatingsQuery::new("").is_err());
}

#[test]
fn ratings_query_rejects_path_injection() {
    assert!(SeekingAlphaRatingsQuery::new("AAPL/../../secret").is_err());
}

// ---------------------------------------------------------------------------
// transform_query
// ---------------------------------------------------------------------------

#[test]
fn articles_transform_query_builds_validated_query() {
    let params = json!({ "ticker": "aapl", "size": 5 });
    let query = SeekingAlphaArticlesHttpFetcher::transform_query(params)
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    assert_eq!(query.ticker, "AAPL");
    assert_eq!(query.size, 5);
}

#[test]
fn articles_transform_query_rejects_bad_size() {
    let params = json!({ "ticker": "AAPL", "size": 0 });
    assert!(SeekingAlphaArticlesHttpFetcher::transform_query(params).is_err());
}

#[test]
fn ratings_transform_query_builds_validated_query() {
    let params = json!({ "ticker": "msft" });
    let query = SeekingAlphaRatingsHttpFetcher::transform_query(params)
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    assert_eq!(query.ticker, "MSFT");
}

// ---------------------------------------------------------------------------
// transform_data — offline, fixture-driven (no network)
// ---------------------------------------------------------------------------

#[test]
fn articles_transform_data_parses_one_article() {
    let fetcher = SeekingAlphaArticlesHttpFetcher::default();
    let query = SeekingAlphaArticlesQuery::new("AAPL", 5)
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    let rows = fetcher
        .transform_data(
            &query,
            Bytes::from_static(articles_cassette_json().as_bytes()),
        )
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "abc123");
    assert_eq!(rows[0].title, "Apple Q1 2024 Earnings: Strong Beat");
    assert_eq!(rows[0].publish_on, "2024-01-25T20:30:00Z");
    assert!(!rows[0].is_locked_pro);
    assert_eq!(rows[0].comment_count, 45);
}

#[test]
fn ratings_transform_data_parses_one_entry() {
    let fetcher = SeekingAlphaRatingsHttpFetcher::default();
    let query =
        SeekingAlphaRatingsQuery::new("AAPL").unwrap_or_else(|e| panic!("query must build: {e}"));
    let rows = fetcher
        .transform_data(
            &query,
            Bytes::from_static(ratings_cassette_json().as_bytes()),
        )
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].ticker, "AAPL");
    assert_eq!(rows[0].quant_rating, 4.12);
    assert_eq!(rows[0].authors_rating, 3.85);
    assert_eq!(rows[0].sell_side_rating, 4.21);
    assert_eq!(rows[0].quant_rating_change, "up");
}

#[test]
fn ratings_transform_data_errors_on_empty_data() {
    let fetcher = SeekingAlphaRatingsHttpFetcher::default();
    let query =
        SeekingAlphaRatingsQuery::new("AAPL").unwrap_or_else(|e| panic!("query must build: {e}"));
    let result = fetcher.transform_data(&query, Bytes::from_static(br#"{"data":[]}"#));
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Registry entries are unique per (provider, endpoint)
// ---------------------------------------------------------------------------

#[test]
fn registry_entries_are_distinct() {
    let articles = SeekingAlphaArticlesHttpFetcher::registry_entry();
    let ratings = SeekingAlphaRatingsHttpFetcher::registry_entry();
    assert_eq!(articles.endpoint, "articles");
    assert_eq!(ratings.endpoint, "ratings");
    assert_ne!(
        (articles.provider, articles.endpoint),
        (ratings.provider, ratings.endpoint)
    );
}

// ---------------------------------------------------------------------------
// Missing API key surfaces an error without a network call
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_api_key_returns_error_without_network_call() {
    // Skip when the key is already set so we don't interfere with live runs.
    if std::env::var("TDW_SEEKING_ALPHA_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        eprintln!("TDW_SEEKING_ALPHA_API_KEY is set; skipping missing-key test");
        return;
    }

    let fetcher = SeekingAlphaArticlesHttpFetcher::default();
    let query = SeekingAlphaArticlesQuery::new("AAPL", 5)
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    let err = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .expect_err("missing key must return error");

    assert!(err.to_string().contains("TDW_SEEKING_ALPHA_API_KEY"));
}

#[tokio::test]
async fn missing_api_key_returns_error_for_ratings_without_network_call() {
    if std::env::var("TDW_SEEKING_ALPHA_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        eprintln!("TDW_SEEKING_ALPHA_API_KEY is set; skipping missing-key test");
        return;
    }

    let fetcher = SeekingAlphaRatingsHttpFetcher::default();
    let query =
        SeekingAlphaRatingsQuery::new("AAPL").unwrap_or_else(|e| panic!("query must build: {e}"));
    let err = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .expect_err("missing key must return error");

    assert!(err.to_string().contains("TDW_SEEKING_ALPHA_API_KEY"));
}

// ---------------------------------------------------------------------------
// Live integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_seeking_alpha_articles_returns_data_when_env_var_set() {
    if std::env::var("TDW_SEEKING_ALPHA_LIVE").ok().as_deref() != Some("1") {
        eprintln!(
            "TDW_SEEKING_ALPHA_LIVE != 1; skipping live Seeking Alpha articles integration test"
        );
        return;
    }

    let fetcher = SeekingAlphaArticlesHttpFetcher::default();
    live_fetch_obbject_nonempty!(
        fetcher,
        json!({ "ticker": "AAPL", "size": 5 }),
        "live response must include at least one article"
    );
}

#[tokio::test]
async fn live_seeking_alpha_ratings_returns_data_when_env_var_set() {
    if std::env::var("TDW_SEEKING_ALPHA_LIVE").ok().as_deref() != Some("1") {
        eprintln!(
            "TDW_SEEKING_ALPHA_LIVE != 1; skipping live Seeking Alpha ratings integration test"
        );
        return;
    }

    let fetcher = SeekingAlphaRatingsHttpFetcher::default();
    let obbject = fetcher
        .fetch(json!({ "ticker": "AAPL" }), &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert_eq!(obbject.rows.len(), 1);
    assert_eq!(obbject.rows[0].ticker, "AAPL");
    assert!(
        obbject.rows[0].quant_rating > 0.0,
        "quant_rating must be positive"
    );
}
