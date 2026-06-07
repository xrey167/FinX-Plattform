//! Integration tests for the real Velodata HTTP fetchers.
//!
//! Cassette tests always run when the `http` feature is enabled and parse
//! recorded response shapes. The live tests are additionally gated by
//! `TDW_VELODATA_LIVE=1` and require `TDW_VELODATA_API_KEY` to be set.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};
use tdw_provider_velodata::{
    AggregatedLiquidation, AggregatedOi, FundingRate, VelodataFundingQuery,
    VelodataHttpFundingFetcher, VelodataHttpLiquidationsFetcher, VelodataHttpOiFetcher,
    VelodataLiquidationsQuery, VelodataOiQuery,
};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn funding_cassette_bytes() -> Bytes {
    cassette_bytes!([
        {
            "timestamp": 1704153600000i64,
            "exchange": "binance",
            "symbol": "BTCUSDT",
            "fundingRate": 0.0001,
            "fundingRateAnnualized": 0.1095
        },
        {
            "timestamp": 1704182400000i64,
            "exchange": "binance",
            "symbol": "BTCUSDT",
            "fundingRate": 0.00012,
            "fundingRateAnnualized": 0.1314
        }
    ])
}

fn liquidations_cassette_bytes() -> Bytes {
    cassette_bytes!([
        {
            "timestamp": 1704153600000i64,
            "symbol": "BTC",
            "longLiquidations": 1250000.0,
            "shortLiquidations": 850000.0,
            "totalLiquidations": 2100000.0
        },
        {
            "timestamp": 1704157200000i64,
            "symbol": "BTC",
            "longLiquidations": 430000.0,
            "shortLiquidations": 210000.0,
            "totalLiquidations": 640000.0
        }
    ])
}

fn oi_cassette_bytes() -> Bytes {
    cassette_bytes!([
        {
            "timestamp": 1704153600000i64,
            "symbol": "BTC",
            "openInterest": 15200000000.0,
            "openInterestChange": 250000000.0
        },
        {
            "timestamp": 1704157200000i64,
            "symbol": "BTC",
            "openInterest": 15450000000.0,
            "openInterestChange": 250000000.0
        }
    ])
}

// ---------------------------------------------------------------------------
// transform_query tests
// ---------------------------------------------------------------------------

#[test]
fn transform_query_funding_normalizes_and_validates() {
    let query = VelodataHttpFundingFetcher::transform_query(json!({
        "exchange": "Binance",
        "symbol": "btcusdt",
        "limit": 100
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    assert_eq!(query.exchange, "binance");
    assert_eq!(query.symbol, "BTCUSDT");
    assert_eq!(query.limit, 100);
}

#[test]
fn transform_query_funding_rejects_path_injection() {
    assert!(
        VelodataHttpFundingFetcher::transform_query(json!({
            "exchange": "bin/ance",
            "symbol": "BTCUSDT",
            "limit": 10
        }))
        .is_err()
    );
    assert!(
        VelodataHttpFundingFetcher::transform_query(json!({
            "exchange": "binance",
            "symbol": "BTC?flag=1",
            "limit": 10
        }))
        .is_err()
    );
    assert!(
        VelodataHttpFundingFetcher::transform_query(json!({
            "symbol": "BTCUSDT",
            "limit": 10
        }))
        .is_err()
    );
}

#[test]
fn transform_query_liquidations_normalizes_symbol() {
    let query = VelodataHttpLiquidationsFetcher::transform_query(json!({
        "symbol": "btc",
        "limit": 24
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    assert_eq!(query.symbol, "BTC");
    assert_eq!(query.limit, 24);
}

#[test]
fn transform_query_liquidations_rejects_invalid_symbol() {
    assert!(
        VelodataHttpLiquidationsFetcher::transform_query(json!({
            "symbol": "BTC/USD",
            "limit": 10
        }))
        .is_err()
    );
    assert!(VelodataHttpLiquidationsFetcher::transform_query(json!({ "limit": 10 })).is_err());
}

#[test]
fn transform_query_oi_normalizes_symbol() {
    let query = VelodataHttpOiFetcher::transform_query(json!({
        "symbol": "eth",
        "limit": 48
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    assert_eq!(query.symbol, "ETH");
    assert_eq!(query.limit, 48);
}

// ---------------------------------------------------------------------------
// Cassette replay — transform_data
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_funding_rates_into_domain_rows() {
    let fetcher = VelodataHttpFundingFetcher::default();
    let query = VelodataFundingQuery::new("binance", "BTCUSDT", 100)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows: Vec<FundingRate> = fetcher
        .transform_data(&query, funding_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].exchange, "binance");
    assert_eq!(rows[0].symbol, "BTCUSDT");
    assert_eq!(rows[0].timestamp, 1_704_153_600_000);
    assert_eq!(rows[0].funding_rate, 0.0001);
    assert_eq!(rows[0].funding_rate_annualized, 0.1095);
    assert_eq!(rows[1].funding_rate, 0.00012);
    assert!(rows[1].timestamp > rows[0].timestamp);
}

#[test]
fn cassette_replay_decodes_liquidations_into_domain_rows() {
    let fetcher = VelodataHttpLiquidationsFetcher::default();
    let query = VelodataLiquidationsQuery::new("BTC", 24)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows: Vec<AggregatedLiquidation> = fetcher
        .transform_data(&query, liquidations_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "BTC");
    assert_eq!(rows[0].long_liquidations, 1_250_000.0);
    assert_eq!(rows[0].short_liquidations, 850_000.0);
    assert_eq!(rows[0].total_liquidations, 2_100_000.0);
    assert!(rows[1].timestamp > rows[0].timestamp);
}

#[test]
fn cassette_replay_decodes_oi_into_domain_rows() {
    let fetcher = VelodataHttpOiFetcher::default();
    let query =
        VelodataOiQuery::new("BTC", 24).unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows: Vec<AggregatedOi> = fetcher
        .transform_data(&query, oi_cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "BTC");
    assert_eq!(rows[0].open_interest, 15_200_000_000.0);
    assert_eq!(rows[0].open_interest_change, 250_000_000.0);
    assert!(rows[1].timestamp > rows[0].timestamp);
}

#[test]
fn transform_data_rejects_malformed_json_for_all_fetchers() {
    let funding_fetcher = VelodataHttpFundingFetcher::default();
    let liq_fetcher = VelodataHttpLiquidationsFetcher::default();
    let oi_fetcher = VelodataHttpOiFetcher::default();

    let funding_query = VelodataFundingQuery::new("binance", "BTCUSDT", 1)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let liq_query = VelodataLiquidationsQuery::new("BTC", 1)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let oi_query =
        VelodataOiQuery::new("BTC", 1).unwrap_or_else(|e| panic!("query should build: {e}"));

    assert!(
        funding_fetcher
            .transform_data(&funding_query, Bytes::from_static(b"not json"))
            .is_err()
    );
    assert!(
        liq_fetcher
            .transform_data(&liq_query, Bytes::from_static(b"{}"))
            .is_err()
    );
    assert!(
        oi_fetcher
            .transform_data(&oi_query, Bytes::from_static(b"null"))
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// Live integration tests (gated by TDW_VELODATA_LIVE=1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_velodata_funding_rates_returns_rows_when_env_vars_set() {
    if std::env::var("TDW_VELODATA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_VELODATA_LIVE != 1; skipping live Velodata funding integration test");
        return;
    }

    let fetcher = VelodataHttpFundingFetcher::default();
    let query = VelodataFundingQuery::new("binance", "BTCUSDT", 10)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one row"
    );
    assert_eq!(rows[0].exchange, "binance");
    assert_eq!(rows[0].symbol, "BTCUSDT");
}

#[tokio::test]
async fn live_velodata_liquidations_returns_rows_when_env_vars_set() {
    if std::env::var("TDW_VELODATA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_VELODATA_LIVE != 1; skipping live Velodata liquidations integration test");
        return;
    }

    let fetcher = VelodataHttpLiquidationsFetcher::default();
    let query = VelodataLiquidationsQuery::new("BTC", 10)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one row"
    );
    assert_eq!(rows[0].symbol, "BTC");
}

#[tokio::test]
async fn live_velodata_oi_returns_rows_when_env_vars_set() {
    if std::env::var("TDW_VELODATA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_VELODATA_LIVE != 1; skipping live Velodata OI integration test");
        return;
    }

    let fetcher = VelodataHttpOiFetcher::default();
    let query =
        VelodataOiQuery::new("BTC", 10).unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one row"
    );
    assert_eq!(rows[0].symbol, "BTC");
}
