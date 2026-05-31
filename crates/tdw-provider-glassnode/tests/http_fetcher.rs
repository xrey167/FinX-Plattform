#![cfg(feature = "http")]
//! Tests for the Glassnode HTTP fetcher.
//!
//! Cassette tests always run under `--features http` and parse inline JSON
//! that matches the real API response shape.  The live test is additionally
//! gated by `TDW_GLASSNODE_LIVE=1` and requires `TDW_GLASSNODE_API_KEY`.

use bytes::Bytes;
use serde_json::json;
use tdw_provider_glassnode::{GlassnodeHttpFetcher, GlassnodeMetric, GlassnodeMetricQuery};

fn btc_mvrv_query() -> GlassnodeMetricQuery {
    GlassnodeMetricQuery::new("BTC", GlassnodeMetric::MvrvZScore, "24h")
        .unwrap_or_else(|e| panic!("query must build: {e}"))
}

fn fetcher() -> GlassnodeHttpFetcher {
    GlassnodeHttpFetcher::new().unwrap_or_else(|e| panic!("fetcher must build: {e}"))
}

fn mvrv_cassette() -> Bytes {
    Bytes::from(
        json!([
            {"t": 1704067200, "v": 1.72},
            {"t": 1704153600, "v": 1.85},
            {"t": 1704240000, "v": 1.91}
        ])
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_decodes_mvrv_z_score_points() {
    let f = fetcher();
    let q = btc_mvrv_query();
    let rows = f
        .decode(&q, mvrv_cassette())
        .unwrap_or_else(|e| panic!("decode must succeed: {e}"));

    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert_eq!(rows[0].timestamp, 1_704_067_200);
    assert_eq!(rows[0].value, 1.72);
    assert_eq!(rows[0].asset, "BTC");
    assert_eq!(rows[1].timestamp, 1_704_153_600);
    assert_eq!(rows[1].value, 1.85);
    assert_eq!(rows[2].value, 1.91);
}

#[test]
fn cassette_decodes_lth_supply_points() {
    let f = fetcher();
    let q = GlassnodeMetricQuery::new("BTC", GlassnodeMetric::LthSupply, "24h")
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    let raw = Bytes::from(
        json!([
            {"t": 1704153600, "v": 14_250_000.5}
        ])
        .to_string()
        .into_bytes(),
    );
    let rows = f
        .decode(&q, raw)
        .unwrap_or_else(|e| panic!("decode must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert!((rows[0].value - 14_250_000.5).abs() < f64::EPSILON);
}

#[test]
fn cassette_decodes_nupl_points() {
    let f = fetcher();
    let q = GlassnodeMetricQuery::new("BTC", GlassnodeMetric::Nupl, "24h")
        .unwrap_or_else(|e| panic!("query must build: {e}"));
    let raw = Bytes::from(
        json!([
            {"t": 1704153600, "v": 0.42}
        ])
        .to_string()
        .into_bytes(),
    );
    let rows = f
        .decode(&q, raw)
        .unwrap_or_else(|e| panic!("decode must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert!((rows[0].value - 0.42).abs() < f64::EPSILON);
}

#[test]
fn cassette_decode_returns_error_on_malformed_json() {
    let f = fetcher();
    let q = btc_mvrv_query();
    let err = f
        .decode(&q, Bytes::from_static(b"not json"))
        .expect_err("malformed JSON must produce an error");

    assert!(err.to_string().contains("json parse"), "err={err}");
}

#[test]
fn cassette_decode_handles_empty_array() {
    let f = fetcher();
    let q = btc_mvrv_query();
    let rows = f
        .decode(&q, Bytes::from_static(b"[]"))
        .unwrap_or_else(|e| panic!("empty array must decode: {e}"));

    assert!(rows.is_empty());
}

#[tokio::test]
async fn live_glassnode_returns_data_points_when_env_vars_set() {
    if std::env::var("TDW_GLASSNODE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_GLASSNODE_LIVE != 1; skipping live Glassnode integration test");
        return;
    }

    let f = fetcher();
    let q = btc_mvrv_query();
    let rows = f
        .fetch(&q)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one data point"
    );
    assert_eq!(rows[0].asset, "BTC");
}
