//! Integration tests for the real TMX HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Two layers:
//!
//! 1. **Cassette tests** (`cassette_*`) — always run under the feature;
//!    parse a recorded TMX `getquote` / `getsquote` response shape and
//!    assert row decoding. Verify the deserialiser without hitting TMX.
//!
//! 2. **Live tests** (`live_tmx_*`) — additionally gated by
//!    `TDW_TMX_LIVE=1`; perform a real HTTP request against TMX Money.
//!    Skipped by default to keep CI quiet.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tmx::http_fetcher::{
    TmxHttpBatchQuoteFetcher, TmxHttpQuoteFetcher, cassette_bytes,
};

// ── helpers ────────────────────────────────────────────────────────────────

fn single_query() -> tdw_provider_tmx::TmxQuoteQuery {
    TmxHttpQuoteFetcher::transform_query(json!({ "symbol": "TD" }))
        .unwrap_or_else(|error| panic!("query must transform: {error}"))
}

fn batch_query() -> tdw_provider_tmx::TmxBatchQuoteQuery {
    TmxHttpBatchQuoteFetcher::transform_query(json!({ "symbols": ["TD", "RY"] }))
        .unwrap_or_else(|error| panic!("batch query must transform: {error}"))
}

fn single_cassette() -> Bytes {
    cassette_bytes(json!([{
        "symbol": "TD",
        "exchange": "TSX",
        "lastPrice": 79.50,
        "change": 0.25,
        "changePercent": 0.32,
        "volume": 3_500_000u64,
        "marketCap": 145_200_000_000.0f64,
        "open": 79.25,
        "high": 79.85,
        "low": 79.10,
        "close": 79.50
    }]))
}

fn batch_cassette() -> Bytes {
    cassette_bytes(json!([
        {
            "symbol": "TD", "exchange": "TSX",
            "lastPrice": 79.50, "change": 0.25, "changePercent": 0.32,
            "volume": 3_500_000u64, "marketCap": null,
            "open": 79.25, "high": 79.85, "low": 79.10, "close": 79.50
        },
        {
            "symbol": "RY", "exchange": "TSX",
            "lastPrice": 130.0, "change": -0.50, "changePercent": -0.38,
            "volume": 2_100_000u64, "marketCap": null,
            "open": 130.5, "high": 131.0, "low": 129.5, "close": 130.0
        }
    ]))
}

// ── cassette: single quote ─────────────────────────────────────────────────

#[test]
#[allow(clippy::float_cmp)]
fn cassette_single_decodes_tmx_quote_envelope() {
    let fetcher = TmxHttpQuoteFetcher::default();
    let query = single_query();
    let rows = fetcher
        .transform_data(&query, single_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "TD");
    assert_eq!(rows[0].close, 79.50);
    assert_eq!(rows[0].volume, 3_500_000);
    assert_eq!(rows[0].open, 79.25);
    assert_eq!(rows[0].high, 79.85);
    assert_eq!(rows[0].low, 79.10);
}

#[test]
fn cassette_single_rejects_malformed_json() {
    let fetcher = TmxHttpQuoteFetcher::default();
    let query = single_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must fail");
    assert!(err.to_string().contains("json parse error") || err.to_string().contains("provider"));
}

// ── cassette: batch quote ──────────────────────────────────────────────────

#[test]
#[allow(clippy::float_cmp)]
fn cassette_batch_decodes_multiple_quotes() {
    let fetcher = TmxHttpBatchQuoteFetcher::default();
    let query = batch_query();
    let rows = fetcher
        .transform_data(&query, batch_cassette())
        .unwrap_or_else(|error| panic!("batch transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].symbol, "TD");
    assert_eq!(rows[1].symbol, "RY");
    assert_eq!(rows[1].close, 130.0);
}

// ── live: single quote ─────────────────────────────────────────────────────

#[tokio::test]
async fn live_tmx_single_quote_returns_row_when_env_var_set() {
    if std::env::var("TDW_TMX_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TMX_LIVE != 1; skipping live TMX single-quote test");
        return;
    }

    let fetcher = TmxHttpQuoteFetcher::default();
    let query = single_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must contain at least one row"
    );
    assert_eq!(rows[0].symbol, "TD");
}

// ── live: batch quote ──────────────────────────────────────────────────────

#[tokio::test]
async fn live_tmx_batch_quote_returns_rows_when_env_var_set() {
    if std::env::var("TDW_TMX_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TMX_LIVE != 1; skipping live TMX batch-quote test");
        return;
    }

    let fetcher = TmxHttpBatchQuoteFetcher::default();
    let query = batch_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live batch extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live batch transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live batch response must contain at least one row"
    );
}
