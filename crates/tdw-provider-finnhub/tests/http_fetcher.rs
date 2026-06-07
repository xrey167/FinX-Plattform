//! Tests for the real Finnhub HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes without any network access. The live test is additionally gated by
//! `TDW_FINNHUB_LIVE=1` and requires `TDW_FINNHUB_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_finnhub::{
    FinnhubHttpProfileFetcher, FinnhubHttpQuoteSnapshotFetcher, FinnhubProfileQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn profile_cassette() -> Bytes {
    cassette_bytes!({
        "ticker": "AAPL",
        "name": "Apple Inc",
        "currency": "USD",
        "exchange": "NASDAQ NMS - GLOBAL MARKET",
        "logo": "https://static2.finnhub.io/file/publicdatany/finnhubimage/stock_logo/AAPL.png",
        "marketCapitalization": 3_000_000.0_f64
    })
}

fn quote_cassette() -> Bytes {
    cassette_bytes!({
        "c": 189.30_f64,
        "d": 1.20_f64,
        "dp": 0.638_f64,
        "h": 190.0_f64,
        "l": 188.0_f64,
        "o": 188.5_f64,
        "pc": 188.10_f64,
        "t": 1_717_200_000_i64
    })
}

// ---------------------------------------------------------------------------
// Profile cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_profile_response() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, profile_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].ticker, "AAPL");
    assert_eq!(rows[0].name, "Apple Inc");
    assert_eq!(rows[0].currency, "USD");
    assert_eq!(rows[0].exchange, "NASDAQ NMS - GLOBAL MARKET");
    assert_eq!(rows[0].market_cap_millions, 3_000_000.0);
}

#[test]
fn profile_transform_query_normalises_symbol_and_rejects_invalid() {
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "aapl"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "AAPL");

    assert!(
        FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL/../../secret"})).is_err()
    );
    assert!(FinnhubHttpProfileFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn profile_empty_response_produces_empty_vec() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    // Finnhub returns an empty object `{}` when the symbol is not found.
    let raw = cassette_bytes!({});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert!(rows.is_empty());
}

#[test]
fn profile_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub profile parse_json"));
}

// ---------------------------------------------------------------------------
// Quote cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_quote_snapshot_response() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, quote_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].current_price, 189.30);
    assert_eq!(rows[0].change, 1.20);
    assert_eq!(rows[0].change_percent, 0.638);
    assert_eq!(rows[0].prev_close, 188.10);
    // Finnhub timestamp is seconds; fetcher multiplies by 1000 for ts_ms.
    assert_eq!(rows[0].ts_ms, 1_717_200_000_000);
}

#[test]
fn quote_snapshot_transform_query_normalises_symbol_and_rejects_invalid() {
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");

    assert!(
        FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "MSFT/../../secret"}))
            .is_err()
    );
    assert!(FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn quote_snapshot_missing_numerics_fall_back_to_zero() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    // Only timestamp provided — all other numeric fields should default to 0.0.
    let raw = cassette_bytes!({"t": 0_i64});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].current_price, 0.0);
    assert_eq!(rows[0].ts_ms, 0);
}

#[test]
fn quote_snapshot_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub quote parse_json"));
}

// ---------------------------------------------------------------------------
// Live tests (gated by TDW_FINNHUB_LIVE=1 and TDW_FINNHUB_API_KEY)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_finnhub_profile_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub profile integration test");
        return;
    }

    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(!rows.is_empty(), "live profile response must include data");
    assert_eq!(rows[0].ticker, "AAPL");
}

#[tokio::test]
async fn live_finnhub_quote_snapshot_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub quote-snapshot integration test");
        return;
    }

    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live quote-snapshot response must include at least one entry"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_finnhub_quote_uses_ticker_param_alias() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub ticker-alias test");
        return;
    }

    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"ticker": "MSFT"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    assert_eq!(query.symbol, "MSFT");

    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let rows = live_fetch_nonempty!(fetcher, query);
    assert_eq!(rows[0].symbol, "MSFT");
}
