//! Tests for the real CBOE HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes inline. The live tests are additionally gated by `TDW_CBOE_LIVE=1`
//! and make real network calls to cdn.cboe.com.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_cboe::{
    CboeHttpIndexFetcher, CboeHttpIndexSnapshotFetcher, CboeHttpOptionsChainFetcher,
    CboeHttpOptionsFetcher, CboeIndexQuery, CboeOptionsQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn options_cassette_bytes() -> Bytes {
    cassette_bytes!({
        "data": {
            "options": [
                {
                    "option": "AAPL240119C00180000",
                    "bid": 5.10,
                    "ask": 5.30,
                    "iv": 0.235,
                    "delta": 0.45,
                    "gamma": 0.02,
                    "theta": -0.05,
                    "open_interest": 12500
                },
                {
                    "option": "AAPL240119P00180000",
                    "bid": 2.40,
                    "ask": 2.60,
                    "iv": 0.210,
                    "delta": -0.55,
                    "gamma": 0.02,
                    "theta": -0.04,
                    "open_interest": 8750
                }
            ]
        }
    })
}

fn index_cassette_bytes() -> Bytes {
    cassette_bytes!({
        "data": {
            "symbol": "VIX",
            "price": 13.45,
            "change": -0.23,
            "volume": 0
        }
    })
}

fn sample_options_query() -> CboeOptionsQuery {
    CboeHttpOptionsFetcher::transform_query(json!({ "symbol": "aapl" }))
        .unwrap_or_else(|e| panic!("options query should transform: {e}"))
}

fn sample_index_query() -> CboeIndexQuery {
    CboeHttpIndexFetcher::transform_query(json!({ "index": "vix" }))
        .unwrap_or_else(|e| panic!("index query should transform: {e}"))
}

// ---------------------------------------------------------------------------
// Cassette tests — always run under the http feature
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_options_chain() {
    let fetcher = CboeHttpOptionsFetcher::default();
    let query = sample_options_query();
    let contracts = fetcher
        .transform_data(&query, options_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(contracts.len(), 2, "contracts={contracts:#?}");
    assert_eq!(contracts[0].option, "AAPL240119C00180000");
    assert_eq!(contracts[0].bid, 5.10);
    assert_eq!(contracts[0].ask, 5.30);
    assert_eq!(contracts[0].iv, 0.235);
    assert_eq!(contracts[0].delta, 0.45);
    assert_eq!(contracts[0].open_interest, 12_500);
    assert_eq!(contracts[1].option, "AAPL240119P00180000");
}

#[test]
fn cassette_replay_decodes_index_quote() {
    let fetcher = CboeHttpIndexFetcher::default();
    let query = sample_index_query();
    let quotes = fetcher
        .transform_data(&query, index_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(quotes.len(), 1, "quotes={quotes:#?}");
    assert_eq!(quotes[0].symbol, "VIX");
    assert_eq!(quotes[0].price, 13.45);
    assert_eq!(quotes[0].change, -0.23);
    assert_eq!(quotes[0].volume, 0);
}

#[test]
fn transform_query_options_normalises_symbol() {
    let query = CboeHttpOptionsFetcher::transform_query(json!({ "symbol": "msft" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");
}

#[test]
fn transform_query_options_accepts_ticker_alias() {
    let query = CboeHttpOptionsFetcher::transform_query(json!({ "ticker": "SPY" }))
        .unwrap_or_else(|e| panic!("query should transform via ticker alias: {e}"));
    assert_eq!(query.symbol, "SPY");
}

#[test]
fn transform_query_options_rejects_path_injection() {
    assert!(
        CboeHttpOptionsFetcher::transform_query(json!({ "symbol": "../secret" })).is_err(),
        "path injection must be rejected"
    );
}

#[test]
fn transform_query_index_normalises_index() {
    let query = CboeHttpIndexFetcher::transform_query(json!({ "index": "spx" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.index, "SPX");
}

#[test]
fn transform_query_index_accepts_symbol_alias() {
    let query = CboeHttpIndexFetcher::transform_query(json!({ "symbol": "VIX" }))
        .unwrap_or_else(|e| panic!("query should transform via symbol alias: {e}"));
    assert_eq!(query.index, "VIX");
}

#[test]
fn transform_query_index_rejects_digits() {
    assert!(
        CboeHttpIndexFetcher::transform_query(json!({ "index": "VIX2" })).is_err(),
        "index with digits must be rejected"
    );
}

#[test]
fn options_cassette_malformed_json_returns_error() {
    let fetcher = CboeHttpOptionsFetcher::default();
    let query = sample_options_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must return an error");
    assert!(
        err.to_string().contains("cboe options parse_json"),
        "error={err}"
    );
}

#[test]
fn index_cassette_malformed_json_returns_error() {
    let fetcher = CboeHttpIndexFetcher::default();
    let query = sample_index_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must return an error");
    assert!(
        err.to_string().contains("cboe index parse_json"),
        "error={err}"
    );
}

// ---------------------------------------------------------------------------
// Catalog-facing fetchers -> tdw_domain models
// ---------------------------------------------------------------------------

#[test]
fn index_snapshot_maps_to_quote_snapshot_with_derived_percent() {
    let fetcher = CboeHttpIndexSnapshotFetcher::default();
    let query = CboeHttpIndexSnapshotFetcher::transform_query(json!({ "index": "vix" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let snapshots = fetcher
        .transform_data(&query, index_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(snapshots.len(), 1, "snapshots={snapshots:#?}");
    assert_eq!(snapshots[0].symbol, "VIX");
    assert_eq!(snapshots[0].current_price, 13.45);
    assert_eq!(snapshots[0].change, -0.23);
    // prev_close = 13.45 - (-0.23) = 13.68; change_percent = -0.23/13.68*100.
    assert!((snapshots[0].prev_close - 13.68).abs() < 1e-9);
    assert!((snapshots[0].change_percent - (-0.23 / 13.68 * 100.0)).abs() < 1e-9);
}

#[test]
fn options_chain_maps_to_option_contract_and_decodes_occ_symbol() {
    let fetcher = CboeHttpOptionsChainFetcher::default();
    let query = CboeHttpOptionsChainFetcher::transform_query(json!({ "symbol": "aapl" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let contracts = fetcher
        .transform_data(&query, options_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(contracts.len(), 2, "contracts={contracts:#?}");
    // AAPL240119C00180000 -> call, 2024-01-19, strike 180.
    assert_eq!(contracts[0].underlying_symbol, "AAPL");
    assert_eq!(
        contracts[0].contract_symbol.as_deref(),
        Some("AAPL240119C00180000")
    );
    assert_eq!(contracts[0].expiration, "2024-01-19");
    assert_eq!(contracts[0].option_type, "call");
    assert!((contracts[0].strike - 180.0).abs() < 1e-9);
    assert_eq!(contracts[0].bid, Some(5.10));
    assert_eq!(contracts[0].open_interest, Some(12_500));
    // The put leg decodes its type.
    assert_eq!(contracts[1].option_type, "put");
}

// ---------------------------------------------------------------------------
// Live tests — gated by TDW_CBOE_LIVE=1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_cboe_options_returns_contracts_when_env_var_set() {
    if std::env::var("TDW_CBOE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CBOE_LIVE != 1; skipping live CBOE options integration test");
        return;
    }

    let fetcher = CboeHttpOptionsFetcher::default();
    let query = CboeOptionsQuery::new("AAPL").unwrap_or_else(|e| panic!("query should build: {e}"));
    let contracts = live_fetch_nonempty!(fetcher, query);

    assert!(
        !contracts.is_empty(),
        "live response must include at least one option contract"
    );
}

#[tokio::test]
async fn live_cboe_index_returns_quote_when_env_var_set() {
    if std::env::var("TDW_CBOE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CBOE_LIVE != 1; skipping live CBOE index integration test");
        return;
    }

    let fetcher = CboeHttpIndexFetcher::default();
    let query = CboeIndexQuery::new("VIX").unwrap_or_else(|e| panic!("query should build: {e}"));
    let quotes = live_fetch_nonempty!(fetcher, query);

    assert_eq!(
        quotes.len(),
        1,
        "live response must include one index quote"
    );
    assert_eq!(quotes[0].symbol, "VIX");
}
