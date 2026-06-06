//! Tests for the `CCData` HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes offline. The live test is additionally gated by `TDW_CCDATA_LIVE=1`
//! and requires `TDW_CCDATA_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_ccdata::{
    CCDataHttpFetcher, CCDataOhlcvQuery, stub_asset_response, stub_ohlcv_response,
};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn ohlcv_query() -> CCDataOhlcvQuery {
    CCDataHttpFetcher::transform_query(json!({
        "market": "ccix",
        "instrument": "BTC-USD",
        "limit": 2
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn ohlcv_cassette() -> Bytes {
    Bytes::from(
        json!({
            "Data": {
                "TimeFrom": 1704067200,
                "TimeTo": 1704153600,
                "Instrument": "BTC-USD",
                "Entries": [
                    {
                        "TIMESTAMP": 1704153600,
                        "OPEN": 44000.0,
                        "HIGH": 46000.0,
                        "LOW": 43000.0,
                        "CLOSE": 45500.0,
                        "VOLUME": 920000.0,
                        "QUOTE_VOLUME": 41860000000.0
                    },
                    {
                        "TIMESTAMP": 1704067200,
                        "OPEN": 42000.0,
                        "HIGH": 45000.0,
                        "LOW": 41500.0,
                        "CLOSE": 44000.0,
                        "VOLUME": 850000.0,
                        "QUOTE_VOLUME": 37400000000.0
                    }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    )
}

fn asset_cassette() -> Bytes {
    Bytes::from(
        json!({
            "Data": {
                "ID": 1182,
                "SYMBOL": "BTC",
                "NAME": "Bitcoin",
                "ASSET_TYPE": "BLOCKCHAIN",
                "LAUNCH_DATE": 1230940800,
                "CIRCULATING_SUPPLY": 19500000.0,
                "MARKET_CAP_USD": 858000000000.0
            }
        })
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// OHLCV cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_ohlcv_into_market_bars() {
    let fetcher = CCDataHttpFetcher::default();
    let query = ohlcv_query();
    let rows = fetcher
        .transform_data(&query, ohlcv_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    // Rows are sorted ascending by timestamp string.
    assert_eq!(rows[0].symbol, "BTC-USD");
    assert_eq!(rows[0].venue, "ccdata");
    assert_eq!(rows[0].ts, "2024-01-01T00:00:00Z");
    assert_eq!(rows[0].open, 42000.0);
    assert_eq!(rows[0].close, 44000.0);
    assert_eq!(rows[0].volume, 850_000.0);
    assert_eq!(rows[1].ts, "2024-01-02T00:00:00Z");
    assert_eq!(rows[1].close, 45500.0);
}

#[test]
fn transform_query_normalizes_instrument_and_defaults_market() {
    let q = CCDataHttpFetcher::transform_query(json!({
        "instrument": "eth-eur",
        "limit": 7
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(q.instrument, "ETH-EUR");
    assert_eq!(q.market, "ccix");
    assert_eq!(q.limit, 7);
}

#[test]
fn transform_query_rejects_missing_instrument() {
    assert!(CCDataHttpFetcher::transform_query(json!({ "market": "ccix", "limit": 5 })).is_err());
}

#[test]
fn transform_query_rejects_invalid_instrument_format() {
    assert!(
        CCDataHttpFetcher::transform_query(json!({
            "instrument": "BTCUSD",
            "market": "ccix"
        }))
        .is_err()
    );
    assert!(
        CCDataHttpFetcher::transform_query(json!({
            "instrument": "BTC/USD",
            "market": "ccix"
        }))
        .is_err()
    );
}

#[test]
fn transform_data_uses_query_instrument_when_response_field_is_empty() {
    let fetcher = CCDataHttpFetcher::default();
    let query = ohlcv_query();
    // Construct a cassette without the Instrument field.
    let raw = Bytes::from(
        json!({
            "Data": {
                "TimeFrom": 1704067200,
                "TimeTo": 1704067200,
                "Entries": [{
                    "TIMESTAMP": 1704067200,
                    "OPEN": 42000.0,
                    "HIGH": 45000.0,
                    "LOW": 41500.0,
                    "CLOSE": 44000.0,
                    "VOLUME": 850000.0,
                    "QUOTE_VOLUME": 37400000000.0
                }]
            }
        })
        .to_string()
        .into_bytes(),
    );
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows[0].symbol, "BTC-USD");
}

// ---------------------------------------------------------------------------
// Asset metadata cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_asset_metadata() {
    use tdw_provider_ccdata::http_fetcher::CCDataAssetHttpFetcher;

    let fetcher = CCDataAssetHttpFetcher::default();
    let query = CCDataAssetHttpFetcher::transform_query(json!({ "symbol": "btc" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let rows = fetcher
        .transform_data(&query, asset_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "BTC");
    assert_eq!(rows[0].name, "Bitcoin");
    assert_eq!(rows[0].id, 1182);
    assert_eq!(rows[0].asset_type, "BLOCKCHAIN");
    assert_eq!(rows[0].circulating_supply, 19_500_000.0);
    assert!((rows[0].market_cap_usd - 858_000_000_000.0).abs() < 1.0);
}

#[test]
fn asset_transform_query_accepts_asset_symbol_key() {
    use tdw_provider_ccdata::http_fetcher::CCDataAssetHttpFetcher;

    let q = CCDataAssetHttpFetcher::transform_query(json!({ "asset_symbol": "eth" }))
        .unwrap_or_else(|e| panic!("asset_symbol key should work: {e}"));
    assert_eq!(q.symbol, "ETH");
}

#[test]
fn asset_transform_query_rejects_missing_symbol() {
    use tdw_provider_ccdata::http_fetcher::CCDataAssetHttpFetcher;

    assert!(CCDataAssetHttpFetcher::transform_query(json!({ "market": "ccix" })).is_err());
}

// ---------------------------------------------------------------------------
// Stub helpers coverage
// ---------------------------------------------------------------------------

#[test]
fn stub_helpers_parse_as_valid_json() {
    let ohlcv_json = stub_ohlcv_response("BTC-USD");
    let asset_json = stub_asset_response("BTC");

    serde_json::from_str::<serde_json::Value>(&ohlcv_json)
        .unwrap_or_else(|e| panic!("stub_ohlcv_response must be valid JSON: {e}"));
    serde_json::from_str::<serde_json::Value>(&asset_json)
        .unwrap_or_else(|e| panic!("stub_asset_response must be valid JSON: {e}"));
}

#[test]
fn stub_ohlcv_cassette_round_trips_through_transform_data() {
    let fetcher = CCDataHttpFetcher::default();
    let query = CCDataOhlcvQuery::new("ccix", "BTC-USD", 2)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let raw = Bytes::from(stub_ohlcv_response("BTC-USD").into_bytes());
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "stub response must decode to at least one row"
    );
    assert_eq!(rows[0].venue, "ccdata");
}

// ---------------------------------------------------------------------------
// Live integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_ccdata_ohlcv_returns_data_when_env_var_set() {
    if std::env::var("TDW_CCDATA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CCDATA_LIVE != 1; skipping live CCData integration test");
        return;
    }

    let fetcher = CCDataHttpFetcher::default();
    let query = CCDataOhlcvQuery::new("ccix", "BTC-USD", 5)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "BTC-USD");
    assert!(rows[0].close > 0.0, "close price must be positive");
}
