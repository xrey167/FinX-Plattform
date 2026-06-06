//! Tests for the real Yahoo Finance HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Two layers:
//!
//! 1. **Cassette test** (`cassette_replay_*`) — always runs under the
//!    feature; parses a recorded Yahoo v8 chart response shape and
//!    asserts row decoding. Verifies the deserialiser without
//!    actually hitting Yahoo.
//!
//! 2. **Live test** (`live_yahoo_*`) — additionally gated by
//!    `TDW_YAHOO_LIVE=1`; performs a real HTTP request against Yahoo
//!    Finance. Skipped by default to keep CI quiet and avoid Yahoo
//!    rate-limit / region restrictions.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};
use tdw_provider_yahoo::YahooHttpEquityHistoricalFetcher;

fn sample_query() -> tdw_provider_fileset::EquityHistoricalQuery {
    FilesetEquityHistoricalFetcher::transform_query(json!({ "symbol": "AAPL" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

/// A small slice of the real Yahoo v8 chart envelope shape, with three
/// daily bars including one bar with all-null fields (Yahoo emits this
/// when the requested range overlaps an open session).
fn cassette_bytes() -> Bytes {
    cassette_bytes!({
        "chart": {
            "result": [{
                "meta": { "symbol": "AAPL", "currency": "USD" },
                "timestamp": [1_700_006_400, 1_700_092_800, 1_700_179_200],
                "indicators": {
                    "quote": [{
                        "open":   [180.0, 182.5, null],
                        "high":   [183.2, 184.0, null],
                        "low":    [179.5, 181.8, null],
                        "close":  [182.4, 183.6, null],
                        "volume": [50_123_400i64, 45_678_900i64, null]
                    }]
                }
            }],
            "error": null
        }
    })
}

#[test]
fn cassette_replay_decodes_yahoo_chart_envelope_and_skips_null_bars() {
    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    // The third (all-null) bar must be dropped.
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2023-11-15");
    assert_eq!(rows[0].open, 180.0);
    assert_eq!(rows[0].close, 182.4);
    assert_eq!(rows[0].volume, 50_123_400);
    assert_eq!(rows[1].date, "2023-11-16");
    assert_eq!(rows[1].close, 183.6);
}

#[test]
fn cassette_replay_surfaces_yahoo_error_envelope() {
    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let envelope = cassette_bytes!({ "chart": { "result": [], "error": { "code": "Not Found" } } });
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");
    assert!(err.to_string().contains("yahoo chart error"));
}

#[tokio::test]
async fn live_yahoo_returns_recent_bars_when_env_var_set() {
    if std::env::var("TDW_YAHOO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_YAHOO_LIVE != 1; skipping live yahoo integration test");
        return;
    }

    let fetcher = YahooHttpEquityHistoricalFetcher::default();
    let query = sample_query();
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}
