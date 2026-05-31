#![cfg(feature = "http")]
//! Tests for the Tradier HTTP fetchers.
//!
//! Cassette tests always run under the `http` feature and parse inline JSON
//! matching the real Tradier API response shape. The live test is additionally
//! gated by `TDW_TRADIER_LIVE=1` and requires `TDW_TRADIER_API_KEY`.

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tradier::{
    TradierHttpOptionsFetcher, TradierHttpQuoteFetcher, TradierOptionsQuery, TradierQuoteQuery,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn quote_query() -> TradierQuoteQuery {
    TradierHttpQuoteFetcher::transform_query(json!({ "symbol": "aapl" }))
        .unwrap_or_else(|e| panic!("quote query should transform: {e}"))
}

fn options_query() -> TradierOptionsQuery {
    TradierHttpOptionsFetcher::transform_query(json!({
        "symbol": "aapl",
        "expiration": "2024-01-19"
    }))
    .unwrap_or_else(|e| panic!("options query should transform: {e}"))
}

fn quote_cassette() -> Bytes {
    Bytes::from(
        json!({
            "quotes": {
                "quote": {
                    "symbol": "AAPL",
                    "last": 185.20,
                    "bid": 185.15,
                    "ask": 185.25,
                    "volume": 55000000,
                    "open": 184.50,
                    "high": 186.10,
                    "low": 184.20,
                    "close": 185.20,
                    "change": -0.50
                }
            }
        })
        .to_string()
        .into_bytes(),
    )
}

fn options_cassette() -> Bytes {
    Bytes::from(
        json!({
            "options": {
                "option": [
                    {
                        "symbol": "AAPL240119C00180000",
                        "option_type": "call",
                        "strike": 180.0,
                        "bid": 5.10,
                        "ask": 5.30,
                        "open_interest": 12500,
                        "volume": 850,
                        "expiration_date": "2024-01-19"
                    }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Cassette — quote
// ---------------------------------------------------------------------------

#[test]
fn cassette_quote_decodes_symbol_and_bid_ask() {
    let fetcher = TradierHttpQuoteFetcher::default();
    let query = quote_query();
    let rows = fetcher
        .transform_data(&query, quote_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].bid, 185.15);
    assert_eq!(rows[0].ask, 185.25);
    assert_eq!(rows[0].venue, "tradier");
}

#[test]
fn cassette_quote_transform_query_normalises_symbol() {
    let query = TradierHttpQuoteFetcher::transform_query(json!({ "symbol": "msft" }))
        .unwrap_or_else(|e| panic!("should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");
}

#[test]
fn cassette_quote_transform_query_rejects_missing_symbol() {
    TradierHttpQuoteFetcher::transform_query(json!({}))
        .expect_err("missing symbol must be rejected");
}

// ---------------------------------------------------------------------------
// Cassette — options chain
// ---------------------------------------------------------------------------

#[test]
fn cassette_options_decodes_chain_into_rows() {
    let fetcher = TradierHttpOptionsFetcher::default();
    let query = options_query();
    let rows = fetcher
        .transform_data(&query, options_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL240119C00180000");
    assert_eq!(rows[0].date, "2024-01-19");
    assert_eq!(rows[0].low, 180.0);
}

#[test]
fn cassette_options_transform_query_rejects_bad_expiration() {
    TradierHttpOptionsFetcher::transform_query(json!({
        "symbol": "AAPL",
        "expiration": "20240119"
    }))
    .expect_err("bad expiration must be rejected");
}

#[test]
fn cassette_options_transform_query_rejects_missing_expiration() {
    TradierHttpOptionsFetcher::transform_query(json!({ "symbol": "AAPL" }))
        .expect_err("missing expiration must be rejected");
}

// ---------------------------------------------------------------------------
// Live — gated by TDW_TRADIER_LIVE=1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_tradier_quote_returns_data_when_env_vars_set() {
    if std::env::var("TDW_TRADIER_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TRADIER_LIVE != 1; skipping live Tradier quote test");
        return;
    }

    let fetcher = TradierHttpQuoteFetcher::default();
    let query = quote_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one quote"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_tradier_options_chain_returns_data_when_env_vars_set() {
    if std::env::var("TDW_TRADIER_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TRADIER_LIVE != 1; skipping live Tradier options test");
        return;
    }

    let fetcher = TradierHttpOptionsFetcher::default();
    let query = options_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one option"
    );
}
