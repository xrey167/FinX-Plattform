//! Tests for the real Alpaca stock-bars HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! stock-bars response shape. The live test is additionally gated by
//! `TDW_ALPACA_LIVE=1` and requires `APCA_API_KEY_ID` plus
//! `APCA_API_SECRET_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_alpaca::{AlpacaHttpStockBarsFetcher, AlpacaStockBarsQuery};

fn sample_query() -> AlpacaStockBarsQuery {
    AlpacaHttpStockBarsFetcher::transform_query(json!({
        "symbol": "aapl",
        "start": "2024-01-02",
        "end": "2024-01-05",
        "timeframe": "1Day",
        "feed": "iex",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!({
            "bars": {
                "AAPL": [
                    {
                        "t": "2024-01-02T05:00:00Z",
                        "o": 187.15,
                        "h": 188.44,
                        "l": 183.89,
                        "c": 185.64,
                        "v": 82488700.0,
                        "n": 927445,
                        "vw": 185.96
                    },
                    {
                        "t": "2024-01-03T05:00:00Z",
                        "o": 184.22,
                        "h": 185.88,
                        "l": 183.43,
                        "c": 184.25,
                        "v": 58414500.0,
                        "n": 701238,
                        "vw": 184.42
                    }
                ]
            },
            "next_page_token": null
        })
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_alpaca_bars_into_market_bars() {
    let fetcher = AlpacaHttpStockBarsFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].venue, "alpaca");
    assert_eq!(rows[0].ts, "2024-01-02T05:00:00Z");
    assert_eq!(rows[0].open, 187.15);
    assert_eq!(rows[0].close, 185.64);
    assert_eq!(rows[0].volume, 82488700.0);
    assert_eq!(rows[1].ts, "2024-01-03T05:00:00Z");
}

#[test]
fn cassette_replay_surfaces_alpaca_error_envelope() {
    let fetcher = AlpacaHttpStockBarsFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "code": 40110000u64,
            "message": "invalid API key ID"
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("alpaca api error"));
}

#[test]
fn transform_query_normalizes_symbol_and_rejects_path_injection() {
    let query = AlpacaHttpStockBarsFetcher::transform_query(json!({
        "symbol": "msft",
        "start": "2024-01-02",
        "end": "2024-01-05"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.symbol, "MSFT");
    assert!(
        AlpacaHttpStockBarsFetcher::transform_query(json!({
            "symbol": "MSFT/../../secret",
            "start": "2024-01-02",
            "end": "2024-01-05"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_alpaca_returns_recent_bars_when_env_vars_set() {
    if std::env::var("TDW_ALPACA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ALPACA_LIVE != 1; skipping live Alpaca integration test");
        return;
    }

    let fetcher = AlpacaHttpStockBarsFetcher::default();
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}
