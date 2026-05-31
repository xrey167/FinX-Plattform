//! Tests for the real Tiingo HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! response shape. Live tests are additionally gated by `TDW_TIINGO_LIVE=1`
//! and require `TDW_TIINGO_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tiingo::{
    TiingoHistoricalQuery, TiingoHttpHistoricalFetcher, TiingoHttpNewsFetcher, TiingoNewsQuery,
};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn historical_query() -> TiingoHistoricalQuery {
    TiingoHttpHistoricalFetcher::transform_query(json!({ "symbol": "aapl" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn news_query() -> TiingoNewsQuery {
    TiingoHttpNewsFetcher::transform_query(json!({ "tickers": ["aapl"] }))
        .unwrap_or_else(|e| panic!("news query should transform: {e}"))
}

fn historical_cassette_bytes() -> Bytes {
    Bytes::from(
        json!([
            {
                "date": "2024-01-02T00:00:00+00:00",
                "open": 185.6,
                "high": 186.1,
                "low": 184.4,
                "close": 185.2,
                "volume": 55000000.0,
                "adjClose": 185.2,
                "adjHigh": 186.1,
                "adjLow": 184.4,
                "adjOpen": 185.6,
                "adjVolume": 55000000.0,
                "divCash": 0.0,
                "splitFactor": 1.0
            },
            {
                "date": "2024-01-03T00:00:00+00:00",
                "open": 184.2,
                "high": 185.9,
                "low": 183.1,
                "close": 184.7,
                "volume": 48000000.0,
                "adjClose": 184.7,
                "adjHigh": 185.9,
                "adjLow": 183.1,
                "adjOpen": 184.2,
                "adjVolume": 48000000.0,
                "divCash": 0.0,
                "splitFactor": 1.0
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

fn news_cassette_bytes() -> Bytes {
    Bytes::from(
        json!([
            {
                "id": 123,
                "title": "Apple reports record quarter",
                "publishedDate": "2024-01-02T09:00:00+00:00",
                "url": "https://example.com/apple-record-quarter",
                "source": "Reuters",
                "tickers": ["AAPL"],
                "tags": ["earnings"]
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Cassette tests (always run when feature is enabled — no network)
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_historical_response() {
    let fetcher = TiingoHttpHistoricalFetcher::default();
    let query = historical_query();
    let rows = fetcher
        .transform_data(&query, historical_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].venue, "tiingo");
    assert_eq!(rows[0].ts, "2024-01-02T00:00:00+00:00");
    assert_eq!(rows[0].open, 185.6);
    assert_eq!(rows[0].close, 185.2);
    assert_eq!(rows[0].volume, 55_000_000.0);
    assert_eq!(rows[1].ts, "2024-01-03T00:00:00+00:00");
    assert_eq!(rows[1].close, 184.7);
}

#[test]
fn cassette_parse_news_response() {
    let fetcher = TiingoHttpNewsFetcher::default();
    let query = news_query();
    let articles = fetcher
        .transform_data(&query, news_cassette_bytes())
        .unwrap_or_else(|e| panic!("news transform_data must succeed: {e}"));

    assert_eq!(articles.len(), 1, "articles={articles:#?}");
    assert_eq!(articles[0].id, 123);
    assert_eq!(articles[0].title, "Apple reports record quarter");
    assert_eq!(articles[0].published_date, "2024-01-02T09:00:00+00:00");
    assert_eq!(articles[0].url, "https://example.com/apple-record-quarter");
    assert_eq!(articles[0].source, "Reuters");
}

#[test]
fn transform_query_historical_normalises_symbol_and_rejects_injection() {
    let q = TiingoHttpHistoricalFetcher::transform_query(json!({ "symbol": "msft" }))
        .unwrap_or_else(|e| panic!("should transform: {e}"));
    assert_eq!(q.symbol, "MSFT");

    // Also accepts "ticker" alias
    let q2 = TiingoHttpHistoricalFetcher::transform_query(json!({ "ticker": "nvda" }))
        .unwrap_or_else(|e| panic!("ticker alias should work: {e}"));
    assert_eq!(q2.symbol, "NVDA");

    assert!(
        TiingoHttpHistoricalFetcher::transform_query(json!({ "symbol": "AAPL/../../etc" }))
            .is_err(),
        "path injection must be rejected"
    );
    assert!(
        TiingoHttpHistoricalFetcher::transform_query(json!({})).is_err(),
        "missing symbol must be rejected"
    );
}

#[test]
fn transform_query_news_accepts_string_and_array() {
    let q_str = TiingoHttpNewsFetcher::transform_query(json!({ "tickers": "aapl" }))
        .unwrap_or_else(|e| panic!("string tickers should work: {e}"));
    assert_eq!(q_str.tickers, vec!["AAPL"]);

    let q_arr = TiingoHttpNewsFetcher::transform_query(json!({ "tickers": ["aapl", "msft"] }))
        .unwrap_or_else(|e| panic!("array tickers should work: {e}"));
    assert_eq!(q_arr.tickers, vec!["AAPL", "MSFT"]);

    assert!(
        TiingoHttpNewsFetcher::transform_query(json!({ "tickers": [] })).is_err(),
        "empty tickers must be rejected"
    );
    assert!(
        TiingoHttpNewsFetcher::transform_query(json!({})).is_err(),
        "missing tickers field must be rejected"
    );
}

#[test]
fn cassette_malformed_historical_json_returns_provider_error() {
    let fetcher = TiingoHttpHistoricalFetcher::default();
    let query = historical_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must fail");
    assert!(
        err.to_string().contains("tiingo historical parse_json"),
        "unexpected error: {err}"
    );
}

#[test]
fn cassette_malformed_news_json_returns_provider_error() {
    let fetcher = TiingoHttpNewsFetcher::default();
    let query = news_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must fail");
    assert!(
        err.to_string().contains("tiingo news parse_json"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// Live integration tests (skipped unless TDW_TIINGO_LIVE=1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_historical_returns_data_when_env_var_set() {
    if std::env::var("TDW_TIINGO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TIINGO_LIVE != 1; skipping live Tiingo historical integration test");
        return;
    }

    let fetcher = TiingoHttpHistoricalFetcher::default();
    let query = historical_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_news_returns_data_when_env_var_set() {
    if std::env::var("TDW_TIINGO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TIINGO_LIVE != 1; skipping live Tiingo news integration test");
        return;
    }

    let fetcher = TiingoHttpNewsFetcher::default();
    let query = news_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live news extract_data must succeed: {e}"));
    let articles = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live news transform_data must succeed: {e}"));

    assert!(
        !articles.is_empty(),
        "live news response must include at least one article"
    );
}
