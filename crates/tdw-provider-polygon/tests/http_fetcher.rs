//! Tests for the real Polygon aggregates HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! aggregates response shape. The live test is additionally gated by
//! `TDW_POLYGON_LIVE=1` and requires `POLYGON_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_polygon::{PolygonAggregatesQuery, PolygonHttpAggregatesFetcher};

fn sample_query() -> PolygonAggregatesQuery {
    PolygonHttpAggregatesFetcher::transform_query(json!({
        "ticker": "msft",
        "from": "2024-01-02",
        "to": "2024-01-05",
        "adjusted": true,
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!({
            "ticker": "MSFT",
            "queryCount": 2,
            "resultsCount": 2,
            "adjusted": true,
            "status": "OK",
            "request_id": "test-cassette",
            "count": 2,
            "results": [
                {
                    "v": 25258600.0,
                    "vw": 370.12,
                    "o": 373.86,
                    "c": 370.87,
                    "h": 375.90,
                    "l": 366.77,
                    "t": 1704153600000i64,
                    "n": 567890
                },
                {
                    "v": 23083500.0,
                    "vw": 368.40,
                    "o": 369.01,
                    "c": 367.94,
                    "h": 373.26,
                    "l": 366.09,
                    "t": 1704240000000i64,
                    "n": 456789
                }
            ]
        })
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_polygon_aggregates_into_market_bars() {
    let fetcher = PolygonHttpAggregatesFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "MSFT");
    assert_eq!(rows[0].venue, "polygon");
    assert_eq!(rows[0].ts, "2024-01-02T00:00:00Z");
    assert_eq!(rows[0].open, 373.86);
    assert_eq!(rows[0].close, 370.87);
    assert_eq!(rows[0].volume, 25258600.0);
    assert_eq!(rows[1].ts, "2024-01-03T00:00:00Z");
}

#[test]
fn cassette_replay_surfaces_polygon_error_envelope() {
    let fetcher = PolygonHttpAggregatesFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "status": "ERROR",
            "error": "API key invalid"
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("polygon api error"));
}

#[test]
fn transform_query_normalizes_ticker_and_rejects_path_injection() {
    let query = PolygonHttpAggregatesFetcher::transform_query(json!({
        "symbol": "aapl",
        "from": "2024-01-02",
        "to": "2024-01-05"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.ticker, "AAPL");
    assert!(
        PolygonHttpAggregatesFetcher::transform_query(json!({
            "ticker": "AAPL/../../secret",
            "from": "2024-01-02",
            "to": "2024-01-05"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_polygon_returns_recent_bars_when_env_vars_set() {
    if std::env::var("TDW_POLYGON_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_POLYGON_LIVE != 1; skipping live Polygon integration test");
        return;
    }

    let fetcher = PolygonHttpAggregatesFetcher::default();
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "MSFT");
}
