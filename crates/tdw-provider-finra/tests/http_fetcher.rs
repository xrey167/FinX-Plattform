#![cfg(feature = "http")]
//! Integration tests for the FINRA HTTP fetchers.
//!
//! Cassette tests always run under the `http` feature and parse recorded
//! FINRA response shapes without any network access.
//!
//! The live integration tests are additionally gated by `TDW_FINRA_LIVE=1`
//! and talk to the real `https://api.finra.org/data/group` endpoint.
//! No API key is required for the FINRA public API.

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_finra::{
    FinraOtcSummaryHttpFetcher, FinraOtcSummaryQuery, FinraShortInterestHttpFetcher,
    FinraShortInterestQuery,
};
use tdw_provider_testkit::live_fetch_rows_expect;

// ── Cassette helpers ──────────────────────────────────────────────────────────

fn cassette_short_interest() -> Bytes {
    Bytes::from(concat!(
        "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15\n",
        "TESLA INC|TSLA|G|55000000|54000000|1.85|22000000|2.5|2024-01-15\n",
        "MICROSOFT CORP|MSFT|G|42000000|41500000|1.20|30000000|1.4|2024-01-15\n",
    ))
}

fn cassette_otc_summary() -> Bytes {
    Bytes::from(concat!(
        "2024-01-15|MKTX|1234567|890\n",
        "2024-01-08|GSCO|9876543|1200\n",
    ))
}

// ── Cassette tests (always run with --features http) ─────────────────────────

#[test]
fn cassette_parse_short_interest_response() {
    let fetcher = FinraShortInterestHttpFetcher::default();
    let query = FinraShortInterestQuery::new(25, 0).expect("valid query");

    let rows = fetcher
        .transform_data(&query, cassette_short_interest())
        .expect("transform_data must succeed");

    assert_eq!(
        rows.len(),
        3,
        "expected 3 short-interest rows, got {rows:#?}"
    );

    assert_eq!(rows[0].issuer_name, "APPLE INC");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].market_class_code, "G");
    assert_eq!(rows[0].current_short_interest, 108_234_568);
    assert_eq!(rows[0].previous_short_interest, 107_982_345);
    assert!((rows[0].percent_change - 0.23).abs() < f64::EPSILON);
    assert_eq!(rows[0].avg_daily_share_volume, 56_789_012);
    assert!((rows[0].days_to_cover - 1.9).abs() < f64::EPSILON);
    assert_eq!(rows[0].settlement_date, "2024-01-15");

    assert_eq!(rows[1].symbol, "TSLA");
    assert_eq!(rows[2].symbol, "MSFT");
}

#[test]
fn cassette_parse_otc_summary_response() {
    let fetcher = FinraOtcSummaryHttpFetcher::default();
    let query = FinraOtcSummaryQuery::new(10).expect("valid query");

    let rows = fetcher
        .transform_data(&query, cassette_otc_summary())
        .expect("transform_data must succeed");

    assert_eq!(rows.len(), 2, "expected 2 OTC summary rows, got {rows:#?}");

    assert_eq!(rows[0].trade_report_date, "2024-01-15");
    assert_eq!(rows[0].market_participant_identifier, "MKTX");
    assert_eq!(rows[0].total_share_quantity, 1_234_567);
    assert_eq!(rows[0].total_trade_count, 890);

    assert_eq!(rows[1].trade_report_date, "2024-01-08");
    assert_eq!(rows[1].market_participant_identifier, "GSCO");
    assert_eq!(rows[1].total_share_quantity, 9_876_543);
    assert_eq!(rows[1].total_trade_count, 1200);
}

#[test]
fn cassette_transform_query_short_interest_roundtrip() {
    let q = FinraShortInterestHttpFetcher::transform_query(
        serde_json::json!({"limit": 50, "offset": 100}),
    )
    .expect("transform_query must succeed");
    assert_eq!(q.limit, 50);
    assert_eq!(q.offset, 100);
}

#[test]
fn cassette_transform_query_otc_summary_roundtrip() {
    let q = FinraOtcSummaryHttpFetcher::transform_query(serde_json::json!({"limit": 5}))
        .expect("transform_query must succeed");
    assert_eq!(q.limit, 5);
}

// ── Live integration tests (gated by TDW_FINRA_LIVE=1) ───────────────────────

#[tokio::test]
async fn live_finra_short_interest_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINRA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINRA_LIVE != 1; skipping live FINRA short-interest integration test");
        return;
    }

    let fetcher = FinraShortInterestHttpFetcher::default();
    let query = FinraShortInterestQuery::new(5, 0).expect("valid query");

    let rows = live_fetch_rows_expect!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live FINRA short-interest must return at least one record"
    );
    assert!(
        !rows[0].symbol.is_empty(),
        "first record symbol must not be empty"
    );
}

#[tokio::test]
async fn live_finra_otc_summary_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINRA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINRA_LIVE != 1; skipping live FINRA OTC summary integration test");
        return;
    }

    let fetcher = FinraOtcSummaryHttpFetcher::default();
    let query = FinraOtcSummaryQuery::new(5).expect("valid query");

    let rows = live_fetch_rows_expect!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live FINRA OTC summary must return at least one record"
    );
    assert!(
        !rows[0].trade_report_date.is_empty(),
        "first record trade_report_date must not be empty"
    );
}
