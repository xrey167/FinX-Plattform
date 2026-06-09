//! Tests for the real CoinGecko HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! The cassette test always runs under the feature and parses a recorded
//! CoinGecko OHLC response shape. The live test is additionally gated by
//! `TDW_COINGECKO_LIVE=1`; the free Demo tier requires no API key (an
//! optional `COINGECKO_API_KEY` is forwarded when present).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_coingecko::CoinGeckoHttpOhlcFetcher;
use tdw_provider_testkit::live_fetch_nonempty;

fn sample_query() -> tdw_provider_coingecko::CoinGeckoOhlcQuery {
    CoinGeckoHttpOhlcFetcher::transform_query(json!({
        "coin_id": "bitcoin",
        "vs_currency": "usd",
        "days": 7
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

#[test]
fn cassette_replay_decodes_ohlc_rows() {
    let fetcher = CoinGeckoHttpOhlcFetcher::default();
    let query = sample_query();
    // CoinGecko OHLC format: [[ts_ms, open, high, low, close], ...]
    let raw = Bytes::from(
        r#"[[1704153600000, 42000.0, 43000.0, 41500.0, 42500.0],
            [1704240000000, 42500.0, 44000.0, 42000.0, 43800.0]]"#,
    );
    let bars = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data: {error}"));
    assert_eq!(bars.len(), 2);
    assert_eq!(bars[0].symbol, "bitcoin");
    assert_eq!(bars[0].venue, "coingecko");
}

#[tokio::test]
async fn live_coingecko_returns_ohlc_bars_when_env_var_set() {
    if std::env::var("TDW_COINGECKO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_COINGECKO_LIVE != 1; skipping live CoinGecko integration test");
        return;
    }

    let fetcher = CoinGeckoHttpOhlcFetcher::default();
    let query = sample_query();
    let bars = live_fetch_nonempty!(fetcher, query);

    assert!(
        !bars.is_empty(),
        "live response must include at least one OHLC bar"
    );
    assert_eq!(bars[0].symbol, "bitcoin");
    assert!(
        bars[0].close > 0.0,
        "live close price must be positive, got {}",
        bars[0].close
    );
}
