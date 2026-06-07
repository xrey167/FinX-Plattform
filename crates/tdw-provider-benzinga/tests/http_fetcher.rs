//! Tests for the Benzinga HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes inline. The live tests are additionally gated by
//! `TDW_BENZINGA_LIVE=1` and require `TDW_BENZINGA_API_KEY`.

#![cfg(feature = "http")]

use tdw_core::{Credentials, Fetcher};
use tdw_provider_benzinga::{
    BenzingaEarningsItem, BenzingaEarningsQuery, BenzingaNewsItem, BenzingaNewsQuery,
    http_fetcher::{BenzingaEarningsHttpFetcher, BenzingaNewsHttpFetcher},
};
use tdw_provider_testkit::live_fetch_obbject_nonempty;

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn news_cassette_json() -> &'static str {
    // `stocks` is Vec<String> on BenzingaNewsItem (already mapped from the
    // wire WireStock objects by the HTTP fetcher). The public struct field
    // is plain strings, so cassette JSON uses a string array.
    r#"[
        {
            "id": "abc123",
            "title": "Apple Q1 Beats Estimates",
            "teaser": "Apple reported strong Q1 results.",
            "url": "https://example.benzinga.com/apple-q1",
            "publishedDate": "2024-01-25T09:30:00Z",
            "source": "Benzinga",
            "stocks": ["AAPL"]
        },
        {
            "id": "abc124",
            "title": "Apple Guidance Raised",
            "teaser": "Management raised full-year guidance.",
            "url": "https://example.benzinga.com/apple-guidance",
            "publishedDate": "2024-01-26T10:00:00Z",
            "source": "Benzinga",
            "stocks": ["AAPL"]
        }
    ]"#
}

fn earnings_cassette_json() -> &'static str {
    r#"{
        "earnings": [
            {
                "id": "e1",
                "date": "2024-01-25",
                "ticker": "AAPL",
                "eps": "2.18",
                "epsEstimate": "2.10",
                "revenue": "119575000000"
            }
        ]
    }"#
}

// ---------------------------------------------------------------------------
// Query validation (re-tested here to confirm the public API from the test
// binary's perspective)
// ---------------------------------------------------------------------------

#[test]
fn news_query_rejects_empty_symbol() {
    assert!(BenzingaNewsQuery::new("", 20).is_err());
}

#[test]
fn news_query_rejects_path_injection() {
    assert!(BenzingaNewsQuery::new("AAPL/../secret", 20).is_err());
}

#[test]
fn news_query_rejects_zero_page_size() {
    assert!(BenzingaNewsQuery::new("AAPL", 0).is_err());
}

#[test]
fn earnings_query_rejects_malformed_date() {
    assert!(BenzingaEarningsQuery::new("AAPL", "20240101", "2024-12-31").is_err());
}

// ---------------------------------------------------------------------------
// Cassette: deserialise news JSON
// ---------------------------------------------------------------------------

#[test]
fn cassette_news_deserialises_two_articles() {
    let items: Vec<BenzingaNewsItem> =
        serde_json::from_str(news_cassette_json()).expect("cassette news must parse");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "abc123");
    assert_eq!(items[0].title, "Apple Q1 Beats Estimates");
    assert_eq!(items[0].source, "Benzinga");
    assert_eq!(items[0].published_date, "2024-01-25T09:30:00Z");
    assert_eq!(items[1].id, "abc124");
}

// ---------------------------------------------------------------------------
// Cassette: deserialise earnings JSON
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct EarningsEnvelope {
    earnings: Vec<BenzingaEarningsItem>,
}

#[test]
fn cassette_earnings_deserialises_one_row() {
    let envelope: EarningsEnvelope =
        serde_json::from_str(earnings_cassette_json()).expect("cassette earnings must parse");

    assert_eq!(envelope.earnings.len(), 1);
    let row = &envelope.earnings[0];
    assert_eq!(row.id, "e1");
    assert_eq!(row.ticker, "AAPL");
    assert_eq!(row.eps, "2.18");
    assert_eq!(row.eps_estimate, "2.10");
    assert_eq!(row.revenue, "119575000000");
}

// ---------------------------------------------------------------------------
// Missing API key surfaces BenzingaProviderError::MissingApiKey
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_api_key_returns_error_without_network_call() {
    // This test only makes sense when TDW_BENZINGA_API_KEY is absent.
    // If the key is already set in the environment (e.g. a developer machine
    // running live tests), skip rather than manipulate env (unsafe not
    // allowed by workspace lints).
    if std::env::var("TDW_BENZINGA_API_KEY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .is_some()
    {
        eprintln!("TDW_BENZINGA_API_KEY is set; skipping missing-key test");
        return;
    }

    let fetcher = BenzingaNewsHttpFetcher::default();
    let query =
        BenzingaNewsQuery::new("AAPL", 5).unwrap_or_else(|e| panic!("query must build: {e}"));
    let err = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .expect_err("missing key must return error");

    assert!(
        err.to_string().contains("TDW_BENZINGA_API_KEY"),
        "missing-key error should mention the env var, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Live integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_benzinga_news_returns_data_when_env_var_set() {
    if std::env::var("TDW_BENZINGA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BENZINGA_LIVE != 1; skipping live Benzinga news integration test");
        return;
    }

    let fetcher = BenzingaNewsHttpFetcher::default();
    let query =
        BenzingaNewsQuery::new("AAPL", 5).unwrap_or_else(|e| panic!("query must build: {e}"));
    live_fetch_obbject_nonempty!(
        fetcher,
        serde_json::to_value(&query).unwrap_or_else(|e| panic!("query serialises: {e}")),
        "live response must include at least one article"
    );
}

#[tokio::test]
async fn live_benzinga_earnings_returns_data_when_env_var_set() {
    if std::env::var("TDW_BENZINGA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BENZINGA_LIVE != 1; skipping live Benzinga earnings integration test");
        return;
    }

    let fetcher = BenzingaEarningsHttpFetcher::default();
    let query = BenzingaEarningsQuery::new("AAPL", "2024-01-01", "2024-12-31")
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    live_fetch_obbject_nonempty!(
        fetcher,
        serde_json::to_value(&query).unwrap_or_else(|e| panic!("query serialises: {e}")),
        "live response must include at least one earnings row"
    );
}
