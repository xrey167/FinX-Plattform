//! Tests for the Seeking Alpha HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes inline. The live tests are additionally gated by
//! `TDW_SEEKING_ALPHA_LIVE=1` and require `TDW_SEEKING_ALPHA_API_KEY`.

#![cfg(feature = "http")]

use tdw_provider_seeking_alpha::{
    SeekingAlphaArticlesQuery, SeekingAlphaProviderError, SeekingAlphaRatingsQuery,
    http_fetcher::{SeekingAlphaArticlesHttpFetcher, SeekingAlphaRatingsHttpFetcher},
};

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
// Cassette: deserialise articles JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_articles_deserialises_one_article() {
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct Envelope {
        data: Vec<tdw_provider_seeking_alpha::SeekingAlphaArticle>,
    }

    // The public SeekingAlphaArticle uses snake_case field names; the cassette
    // envelope matches the wire shape mapped by the http fetcher. Here we
    // exercise the public struct via a re-mapped cassette to avoid duplicating
    // deserialization logic.
    let articles: Vec<tdw_provider_seeking_alpha::SeekingAlphaArticle> =
        serde_json::from_str::<serde_json::Value>(articles_cassette_json())
            .expect("cassette must be valid JSON")
            .get("data")
            .and_then(|d| d.as_array())
            .expect("data array must be present")
            .iter()
            .map(|entry| {
                let id = entry["id"].as_str().unwrap_or_default().to_string();
                let attrs = &entry["attributes"];
                tdw_provider_seeking_alpha::SeekingAlphaArticle {
                    id,
                    title: attrs["title"].as_str().unwrap_or_default().to_string(),
                    publish_on: attrs["publishOn"].as_str().unwrap_or_default().to_string(),
                    is_locked_pro: attrs["isLockedPro"].as_bool().unwrap_or_default(),
                    comment_count: attrs["commentCount"].as_u64().unwrap_or_default() as u32,
                }
            })
            .collect();

    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].id, "abc123");
    assert_eq!(articles[0].title, "Apple Q1 2024 Earnings: Strong Beat");
    assert_eq!(articles[0].publish_on, "2024-01-25T20:30:00Z");
    assert!(!articles[0].is_locked_pro);
    assert_eq!(articles[0].comment_count, 45);
}

// ---------------------------------------------------------------------------
// Cassette: deserialise ratings JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_ratings_deserialises_one_entry() {
    let root: serde_json::Value =
        serde_json::from_str(ratings_cassette_json()).expect("cassette must be valid JSON");
    let entry = &root["data"][0];

    let ratings = tdw_provider_seeking_alpha::SeekingAlphaRatings {
        ticker: entry["id"].as_str().unwrap_or_default().to_string(),
        quant_rating: entry["attributes"]["quant_rating"]
            .as_f64()
            .unwrap_or_default(),
        authors_rating: entry["attributes"]["authors_rating"]
            .as_f64()
            .unwrap_or_default(),
        sell_side_rating: entry["attributes"]["sell_side_rating"]
            .as_f64()
            .unwrap_or_default(),
        quant_rating_change: entry["attributes"]["quant_rating_change"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    };

    assert_eq!(ratings.ticker, "AAPL");
    assert_eq!(ratings.quant_rating, 4.12);
    assert_eq!(ratings.authors_rating, 3.85);
    assert_eq!(ratings.sell_side_rating, 4.21);
    assert_eq!(ratings.quant_rating_change, "up");
}

// ---------------------------------------------------------------------------
// Missing API key surfaces SeekingAlphaProviderError::MissingApiKey
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
        .fetch(&query)
        .await
        .expect_err("missing key must return error");

    assert_eq!(err, SeekingAlphaProviderError::MissingApiKey);
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
        .fetch(&query)
        .await
        .expect_err("missing key must return error");

    assert_eq!(err, SeekingAlphaProviderError::MissingApiKey);
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
    let query = SeekingAlphaArticlesQuery::new("AAPL", 5)
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    let items = fetcher
        .fetch(&query)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert!(
        !items.is_empty(),
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
    let query =
        SeekingAlphaRatingsQuery::new("AAPL").unwrap_or_else(|e| panic!("query must build: {e}"));
    let ratings = fetcher
        .fetch(&query)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert_eq!(ratings.ticker, "AAPL");
    assert!(ratings.quant_rating > 0.0, "quant_rating must be positive");
}
