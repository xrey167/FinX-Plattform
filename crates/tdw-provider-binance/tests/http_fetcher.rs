//! Tests for the real Binance ticker-price HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! ticker-price response shape. The live test is additionally gated by
//! `TDW_BINANCE_LIVE=1`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_binance::{BinanceHttpTickerPriceFetcher, BinanceTickerPriceQuery};

fn sample_query() -> BinanceTickerPriceQuery {
    BinanceHttpTickerPriceFetcher::transform_query(json!({ "symbol": "btcusdt" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!({
            "symbol": "BTCUSDT",
            "price": "67432.12000000"
        })
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_binance_ticker_price() {
    let fetcher = BinanceHttpTickerPriceFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "BTCUSDT");
    assert_eq!(rows[0].price, 67432.12);
}

#[test]
fn cassette_replay_surfaces_binance_error_envelope() {
    let fetcher = BinanceHttpTickerPriceFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "code": -1121,
            "msg": "Invalid symbol."
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("binance api error -1121"));
}

#[test]
fn transform_query_normalizes_symbol_and_rejects_query_injection() {
    let query = BinanceHttpTickerPriceFetcher::transform_query(json!({ "symbol": "ethusdt" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.symbol, "ETHUSDT");
    assert!(
        BinanceHttpTickerPriceFetcher::transform_query(json!({ "symbol": "ETHUSDT&recvWindow=1" }))
            .is_err()
    );
}

#[tokio::test]
async fn live_binance_returns_ticker_price_when_env_var_set() {
    if std::env::var("TDW_BINANCE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BINANCE_LIVE != 1; skipping live Binance integration test");
        return;
    }

    let fetcher = BinanceHttpTickerPriceFetcher::default();
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(!rows.is_empty(), "live response must include one price row");
    assert_eq!(rows[0].symbol, "BTCUSDT");
}
