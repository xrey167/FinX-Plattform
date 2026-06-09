//! Tests for the real CoinGecko HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! The cassette test always runs under the feature and parses the recorded
//! OHLC array-of-arrays shape. The live test is additionally gated by
//! `TDW_COINGECKO_LIVE=1`; CoinGecko's free tier needs no API key (an
//! optional `COINGECKO_API_KEY` is forwarded when present).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_coingecko::{CoinGeckoHttpOhlcFetcher, CoinGeckoOhlcQuery};
use tdw_provider_testkit::live_fetch_nonempty;

fn sample_query() -> CoinGeckoOhlcQuery {
    CoinGeckoHttpOhlcFetcher::transform_query(json!({
        "coin_id": "bitcoin",
        "vs_currency": "usd",
        "days": 7
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

#[test]
fn cassette_ohlc_transforms_array_of_arrays() {
    let fetcher = CoinGeckoHttpOhlcFetcher::default();
    let query = sample_query();
    // CoinGecko OHLC wire shape: [[timestamp_ms, open, high, low, close], ...]
    let raw = Bytes::from(
        json!([
            [1717200000000_i64, 67000.0, 68000.0, 66500.0, 67800.0],
            [1717286400000_i64, 67800.0, 69000.0, 67500.0, 68900.0]
        ])
        .to_string()
        .into_bytes(),
    );
    let bars = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].symbol, "bitcoin");
    assert_eq!(bars[0].venue, "coingecko");
    assert_eq!(bars[0].open, 67000.0);
    assert_eq!(bars[1].close, 68900.0);
}

#[tokio::test]
async fn live_coingecko_ohlc_returns_bars_when_env_var_set() {
    if std::env::var("TDW_COINGECKO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_COINGECKO_LIVE != 1; skipping live CoinGecko integration test");
        return;
    }

    let fetcher = CoinGeckoHttpOhlcFetcher::default();
    let query = sample_query();
    let bars = live_fetch_nonempty!(fetcher, query);

    assert!(
        !bars.is_empty(),
        "live CoinGecko OHLC must return at least one bar"
    );
    assert_eq!(bars[0].symbol, "bitcoin");
    assert_eq!(bars[0].venue, "coingecko");
}
