//! Tests for the real AkShare historical OHLCV HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! response shape. The live test is additionally gated by
//! `TDW_AKSHARE_LIVE=1`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_akshare::{AkShareHttpFetcher, AkShareMarket, AkShareQuery};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn sample_a_share_query() -> AkShareQuery {
    AkShareHttpFetcher::transform_query(json!({
        "symbol": "000001",
        "market": "AShares",
        "start_date": "20240101",
        "end_date": "20240131"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn sample_hk_query() -> AkShareQuery {
    AkShareHttpFetcher::transform_query(json!({
        "symbol": "00700",
        "market": "HongKong",
        "start_date": "20240101",
        "end_date": "20240131"
    }))
    .unwrap_or_else(|e| panic!("hk query should transform: {e}"))
}

fn a_share_cassette_bytes() -> Bytes {
    cassette_bytes!([
        {
            "日期": "2024-01-02",
            "开盘": 10.5,
            "收盘": 10.8,
            "最高": 10.9,
            "最低": 10.4,
            "成交量": 8500000.0
        },
        {
            "日期": "2024-01-03",
            "开盘": 10.8,
            "收盘": 11.0,
            "最高": 11.2,
            "最低": 10.7,
            "成交量": 9200000.0
        }
    ])
}

fn hk_cassette_bytes() -> Bytes {
    cassette_bytes!([
        {
            "日期": "2024-01-02",
            "开盘": 310.0,
            "收盘": 315.0,
            "最高": 318.0,
            "最低": 308.0,
            "成交量": 25000000.0
        }
    ])
}

// ---------------------------------------------------------------------------
// Cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_a_share_bars() {
    let fetcher = AkShareHttpFetcher::default();
    let query = sample_a_share_query();
    let rows = fetcher
        .transform_data(&query, a_share_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "expected 2 bars, got: {rows:#?}");
    assert_eq!(rows[0].symbol, "000001");
    assert_eq!(rows[0].venue, "akshare_a");
    assert_eq!(rows[0].ts, "2024-01-02T00:00:00Z");
    assert_eq!(rows[0].open, 10.5);
    assert_eq!(rows[0].close, 10.8);
    assert_eq!(rows[0].high, 10.9);
    assert_eq!(rows[0].low, 10.4);
    assert_eq!(rows[0].volume, 8_500_000.0);
    assert_eq!(rows[1].ts, "2024-01-03T00:00:00Z");
    assert_eq!(rows[1].open, 10.8);
}

#[test]
fn cassette_replay_decodes_hk_bars() {
    let fetcher = AkShareHttpFetcher::default();
    let query = sample_hk_query();
    let rows = fetcher
        .transform_data(&query, hk_cassette_bytes())
        .unwrap_or_else(|e| panic!("hk transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "expected 1 HK bar, got: {rows:#?}");
    assert_eq!(rows[0].symbol, "00700");
    assert_eq!(rows[0].venue, "akshare_hk");
    assert_eq!(rows[0].ts, "2024-01-02T00:00:00Z");
    assert_eq!(rows[0].open, 310.0);
    assert_eq!(rows[0].close, 315.0);
    assert_eq!(rows[0].volume, 25_000_000.0);
}

#[test]
fn cassette_replay_surfaces_parse_error_on_bad_json() {
    let fetcher = AkShareHttpFetcher::default();
    let query = sample_a_share_query();
    let bad = Bytes::from(b"not valid json".to_vec());
    let err = fetcher
        .transform_data(&query, bad)
        .expect_err("invalid JSON must propagate as error");
    assert!(
        err.to_string().contains("akshare parse_json"),
        "error must mention akshare parse_json, got: {err}"
    );
}

#[test]
fn transform_query_defaults_to_a_shares_when_market_omitted() {
    let query = AkShareHttpFetcher::transform_query(json!({
        "symbol": "000001",
        "start_date": "20240101",
        "end_date": "20240131"
    }))
    .unwrap_or_else(|e| panic!("query without explicit market must transform: {e}"));

    assert_eq!(query.market, AkShareMarket::AShares);
}

#[test]
fn transform_query_rejects_missing_symbol() {
    assert!(
        AkShareHttpFetcher::transform_query(json!({
            "start_date": "20240101",
            "end_date": "20240131"
        }))
        .is_err(),
        "missing symbol must be rejected"
    );
}

#[test]
fn transform_query_rejects_invalid_symbol() {
    assert!(
        AkShareHttpFetcher::transform_query(json!({
            "symbol": "00A001",
            "start_date": "20240101",
            "end_date": "20240131"
        }))
        .is_err(),
        "non-digit symbol chars must be rejected"
    );
}

#[test]
fn transform_query_accepts_hk_market_alias() {
    let query = AkShareHttpFetcher::transform_query(json!({
        "symbol": "00700",
        "market": "hk",
        "start_date": "20240101",
        "end_date": "20240131"
    }))
    .unwrap_or_else(|e| panic!("hk alias must be accepted: {e}"));
    assert_eq!(query.market, AkShareMarket::HongKong);
}

// ---------------------------------------------------------------------------
// Live test (gated by TDW_AKSHARE_LIVE=1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_akshare_returns_recent_a_share_bars_when_env_var_set() {
    if std::env::var("TDW_AKSHARE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_AKSHARE_LIVE != 1; skipping live AkShare integration test");
        return;
    }

    let fetcher = AkShareHttpFetcher::default();
    let query = sample_a_share_query();
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "000001");
    assert_eq!(rows[0].venue, "akshare_a");
}
