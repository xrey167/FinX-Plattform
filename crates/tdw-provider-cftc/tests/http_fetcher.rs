//! Tests for the real CFTC Socrata HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded Socrata array
//! response shapes without network access. The live tests are additionally
//! gated by `TDW_CFTC_LIVE=1` (keyless API; an optional `X-App-Token` raises the
//! rate limit).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_cftc::{CftcHttpCotFetcher, CftcHttpCotSearchFetcher};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn cot_cassette() -> Bytes {
    // Socrata returns every value as a JSON string; the fetcher coerces them.
    cassette_bytes!([
        {
            "market_and_exchange_names": "WHEAT-SRW - CHICAGO BOARD OF TRADE",
            "report_date_as_yyyy_mm_dd": "2024-06-04T00:00:00.000",
            "open_interest_all": "420000",
            "noncomm_positions_long_all": "95000",
            "noncomm_positions_short_all": "120000",
            "comm_positions_long_all": "200000",
            "comm_positions_short_all": "180000",
            "tot_rept_positions_long_all": "360000",
            "tot_rept_positions_short_all": "340000",
            "nonrept_positions_long_all": "60000",
            "nonrept_positions_short_all": "80000"
        },
        {
            "market_and_exchange_names": "",
            "report_date_as_yyyy_mm_dd": "2024-06-04T00:00:00.000"
        }
    ])
}

#[test]
fn cot_cassette_decodes_and_coerces_string_numbers() {
    let fetcher = CftcHttpCotFetcher::default();
    let query = CftcHttpCotFetcher::transform_query(json!({
        "command": "regulators/cftc/cot",
        "market": "WHEAT-SRW - CHICAGO BOARD OF TRADE"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, cot_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "empty-market row skipped; rows={rows:#?}");
    assert_eq!(rows[0].market, "WHEAT-SRW - CHICAGO BOARD OF TRADE");
    // The ISO timestamp is normalized to the bare calendar date.
    assert_eq!(rows[0].report_date, "2024-06-04");
    assert_eq!(rows[0].open_interest, Some(420_000.0));
    assert_eq!(rows[0].noncommercial_long, Some(95_000.0));
    assert_eq!(rows[0].noncommercial_short, Some(120_000.0));
    assert_eq!(rows[0].commercial_long, Some(200_000.0));
    assert_eq!(rows[0].commercial_short, Some(180_000.0));
    assert_eq!(rows[0].total_reportable_long, Some(360_000.0));
    assert_eq!(rows[0].total_reportable_short, Some(340_000.0));
    assert_eq!(rows[0].nonreportable_long, Some(60_000.0));
    assert_eq!(rows[0].nonreportable_short, Some(80_000.0));
}

#[test]
fn cot_cassette_surfaces_api_error_object() {
    let fetcher = CftcHttpCotFetcher::default();
    let query = CftcHttpCotFetcher::transform_query(json!({
        "command": "regulators/cftc/cot",
        "market": "WHEAT-SRW - CHICAGO BOARD OF TRADE"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    // Socrata reports a malformed/failed SoQL query as a JSON object, not an array.
    let envelope = cassette_bytes!({ "error": true, "message": "Invalid SoQL query" });
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error object must be propagated");
    assert!(err.to_string().contains("api error"), "got: {err}");
}

#[test]
fn cot_transform_data_rejects_malformed_json() {
    let fetcher = CftcHttpCotFetcher::default();
    let query = CftcHttpCotFetcher::transform_query(json!({
        "command": "regulators/cftc/cot",
        "market": "WHEAT-SRW - CHICAGO BOARD OF TRADE"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let garbage = Bytes::from_static(b"{not valid json");
    let err = fetcher
        .transform_data(&query, garbage)
        .expect_err("malformed JSON must be propagated");
    assert!(err.to_string().contains("parse_json"), "got: {err}");
}

#[test]
fn cot_transform_query_rejects_unknown_command() {
    assert!(CftcHttpCotFetcher::transform_query(json!({ "command": "bogus" })).is_err());
    assert!(CftcHttpCotFetcher::transform_query(json!({})).is_err());
}

fn cot_search_cassette() -> Bytes {
    cassette_bytes!([
        { "market_and_exchange_names": "WHEAT-SRW - CHICAGO BOARD OF TRADE" },
        { "market_and_exchange_names": "GOLD - COMMODITY EXCHANGE INC." },
        // A duplicate the fetcher must collapse, plus a blank to skip.
        { "market_and_exchange_names": "GOLD - COMMODITY EXCHANGE INC." },
        { "market_and_exchange_names": "" }
    ])
}

#[test]
fn cot_search_cassette_returns_distinct_markets() {
    let fetcher = CftcHttpCotSearchFetcher::default();
    let query = CftcHttpCotSearchFetcher::transform_query(json!({
        "command": "regulators/cftc/cot_search"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, cot_search_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "duplicate/blank collapsed; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "WHEAT-SRW - CHICAGO BOARD OF TRADE");
    assert_eq!(rows[1].series_id, "GOLD - COMMODITY EXCHANGE INC.");
    assert_eq!(rows[0].frequency.as_deref(), Some("Weekly"));
}

#[tokio::test]
async fn live_cftc_cot_returns_rows_when_env_set() {
    if std::env::var("TDW_CFTC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CFTC_LIVE != 1; skipping live CFTC COT test");
        return;
    }
    let fetcher = CftcHttpCotFetcher::default();
    let query = CftcHttpCotFetcher::transform_query(json!({
        "command": "regulators/cftc/cot",
        "market": "GOLD - COMMODITY EXCHANGE INC.",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live COT must include rows");
}

#[tokio::test]
async fn live_cftc_cot_search_returns_markets_when_env_set() {
    if std::env::var("TDW_CFTC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CFTC_LIVE != 1; skipping live CFTC COT search test");
        return;
    }
    let fetcher = CftcHttpCotSearchFetcher::default();
    let query = CftcHttpCotSearchFetcher::transform_query(json!({
        "command": "regulators/cftc/cot_search",
        "limit": 25
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live COT search must include markets");
}
