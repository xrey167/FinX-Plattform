#![cfg(feature = "http")]
//! Tests for the real EIA HTTP fetchers.
//!
//! Cassette tests always run under the `http` feature and parse recorded
//! response shapes inline via the canonical `transform_data` hook. The live
//! tests are additionally gated by `TDW_EIA_LIVE=1` and require
//! `TDW_EIA_API_KEY`.

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_eia::{
    EiaCommodity, EiaHttpNaturalGasFetcher, EiaHttpReportFetcher, EiaHttpSpotPriceFetcher,
    EiaNaturalGasQuery, EiaReport, EiaReportQuery, EiaSpotPriceQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Spot-price cassette
// ---------------------------------------------------------------------------

fn spot_price_cassette() -> Bytes {
    cassette_bytes!({
        "response": {
            "data": [
                {
                    "period": "2024-01-02",
                    "product-name": "Crude Oil WTI",
                    "value": "72.36",
                    "units": "Dollars per Barrel"
                },
                {
                    "period": "2024-01-01",
                    "product-name": "Crude Oil WTI",
                    "value": ".",
                    "units": "Dollars per Barrel"
                },
                {
                    "period": "2023-12-29",
                    "product-name": "Crude Oil Brent",
                    "value": "77.59",
                    "units": "Dollars per Barrel"
                }
            ]
        }
    })
}

fn spot_price_query() -> EiaSpotPriceQuery {
    EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 5)
        .unwrap_or_else(|e| panic!("query must construct: {e}"))
}

#[test]
fn cassette_spot_price_decodes_records_and_skips_missing_values() {
    let fetcher = EiaHttpSpotPriceFetcher::default();
    let records = fetcher
        .transform_data(&spot_price_query(), spot_price_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(records.len(), 2, "records={records:#?}");
    assert_eq!(records[0].period, "2024-01-02");
    assert_eq!(records[0].product_name, "Crude Oil WTI");
    assert_eq!(records[0].value, 72.36);
    assert_eq!(records[0].units, "Dollars per Barrel");
    assert_eq!(records[1].period, "2023-12-29");
    assert_eq!(records[1].value, 77.59);
}

#[test]
fn cassette_spot_price_rejects_bad_json() {
    let fetcher = EiaHttpSpotPriceFetcher::default();
    fetcher
        .transform_data(&spot_price_query(), Bytes::from_static(b"not json at all"))
        .expect_err("invalid JSON must be rejected");
}

#[test]
fn cassette_spot_price_rejects_non_numeric_value() {
    let fetcher = EiaHttpSpotPriceFetcher::default();
    let bad = cassette_bytes!({
        "response": {
            "data": [
                {
                    "period": "2024-01-02",
                    "product-name": "Crude Oil WTI",
                    "value": "N/A",
                    "units": "Dollars per Barrel"
                }
            ]
        }
    });
    fetcher
        .transform_data(&spot_price_query(), bad)
        .expect_err("non-numeric value must be rejected");
}

#[test]
fn spot_price_transform_query_validates_and_rejects_bad_length() {
    let query = EiaHttpSpotPriceFetcher::transform_query(serde_json::json!({
        "commodity": "crude_oil_wti",
        "length": 5
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.length, 5);
    assert_eq!(query.commodity, EiaCommodity::CrudeOilWti);

    EiaHttpSpotPriceFetcher::transform_query(serde_json::json!({
        "commodity": "crude_oil_wti",
        "length": 0
    }))
    .expect_err("length=0 must be rejected");
}

// ---------------------------------------------------------------------------
// Natural-gas cassette
// ---------------------------------------------------------------------------

fn natural_gas_cassette() -> Bytes {
    cassette_bytes!({
        "response": {
            "data": [
                {
                    "period": "2024-01",
                    "series-description": "Henry Hub Natural Gas Spot Price",
                    "value": "2.53",
                    "units": "Dollars per Million Btu"
                },
                {
                    "period": "2023-12",
                    "series-description": "Henry Hub Natural Gas Spot Price",
                    "value": ".",
                    "units": "Dollars per Million Btu"
                },
                {
                    "period": "2023-11",
                    "series-description": "Henry Hub Natural Gas Spot Price",
                    "value": "3.12",
                    "units": "Dollars per Million Btu"
                }
            ]
        }
    })
}

fn natural_gas_query() -> EiaNaturalGasQuery {
    EiaNaturalGasQuery::new(12).unwrap_or_else(|e| panic!("query must construct: {e}"))
}

#[test]
fn cassette_natural_gas_decodes_records_and_skips_missing_values() {
    let fetcher = EiaHttpNaturalGasFetcher::default();
    let records = fetcher
        .transform_data(&natural_gas_query(), natural_gas_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(records.len(), 2, "records={records:#?}");
    assert_eq!(records[0].period, "2024-01");
    assert_eq!(
        records[0].series_description,
        "Henry Hub Natural Gas Spot Price"
    );
    assert_eq!(records[0].value, 2.53);
    assert_eq!(records[0].units, "Dollars per Million Btu");
    assert_eq!(records[1].period, "2023-11");
    assert_eq!(records[1].value, 3.12);
}

#[test]
fn cassette_natural_gas_rejects_bad_json() {
    let fetcher = EiaHttpNaturalGasFetcher::default();
    fetcher
        .transform_data(&natural_gas_query(), Bytes::from_static(b"{bad}"))
        .expect_err("invalid JSON must be rejected");
}

#[test]
fn natural_gas_transform_query_validates_and_rejects_bad_length() {
    let query = EiaHttpNaturalGasFetcher::transform_query(serde_json::json!({ "length": 12 }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.length, 12);

    EiaHttpNaturalGasFetcher::transform_query(serde_json::json!({ "length": 10_001 }))
        .expect_err("length>10000 must be rejected");
}

// ---------------------------------------------------------------------------
// Catalog-facing report fetcher -> CommodityReportRow
// ---------------------------------------------------------------------------

fn report_cassette() -> Bytes {
    cassette_bytes!({
        "response": {
            "data": [
                {
                    "period": "2026-05-30",
                    "series": "WCRSTUS1",
                    "series-description": "Weekly U.S. Ending Stocks of Crude Oil",
                    "value": "440123",
                    "units": "thousand barrels"
                },
                {
                    "period": "2026-05-23",
                    "series": "WCRSTUS1",
                    "series-description": "Weekly U.S. Ending Stocks of Crude Oil",
                    "value": ".",
                    "units": "thousand barrels"
                },
                {
                    "period": "2026-05-16",
                    "series": "WCRSTUS1",
                    "series-description": "Weekly U.S. Ending Stocks of Crude Oil",
                    "value": "441900",
                    "units": "thousand barrels"
                }
            ]
        }
    })
}

#[test]
fn report_transform_query_injects_report_and_validates_length() {
    let query = EiaHttpReportFetcher::transform_query(
        serde_json::json!({ "report": "petroleum_status_report" }),
    )
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.report, EiaReport::PetroleumStatusReport);
    assert_eq!(query.length, 100);

    EiaHttpReportFetcher::transform_query(serde_json::json!({
        "report": "short_term_energy_outlook",
        "length": 0
    }))
    .expect_err("length=0 must be rejected");
}

#[test]
fn report_cassette_decodes_rows_and_keeps_missing_value_as_none() {
    let fetcher = EiaHttpReportFetcher::default();
    let query = EiaReportQuery::from_value(&serde_json::json!({
        "report": "petroleum_status_report"
    }))
    .unwrap_or_else(|e| panic!("query must build: {e}"));
    let rows = fetcher
        .transform_data(&query, report_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    // The missing-value row is retained (value=None), not skipped.
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert_eq!(rows[0].report, "petroleum_status_report");
    assert_eq!(rows[0].series_id, "WCRSTUS1");
    assert_eq!(rows[0].period, "2026-05-30");
    assert_eq!(rows[0].value, Some(440_123.0));
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[2].value, Some(441_900.0));
}

#[test]
fn report_cassette_rejects_non_numeric_value() {
    let fetcher = EiaHttpReportFetcher::default();
    let query = EiaReportQuery::from_value(&serde_json::json!({
        "report": "short_term_energy_outlook"
    }))
    .unwrap_or_else(|e| panic!("query must build: {e}"));
    let bad = cassette_bytes!({
        "response": { "data": [
            { "period": "2026-05", "series": "X", "value": "N/A", "units": "u" }
        ]}
    });
    fetcher
        .transform_data(&query, bad)
        .expect_err("non-numeric value must be rejected");
}

#[tokio::test]
async fn live_eia_report_returns_rows_when_env_vars_set() {
    if std::env::var("TDW_EIA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_EIA_LIVE != 1; skipping live EIA report integration test");
        return;
    }
    let query = EiaReportQuery::from_value(&serde_json::json!({
        "report": "petroleum_status_report",
        "length": 5
    }))
    .unwrap_or_else(|e| panic!("query must build: {e}"));
    let fetcher = EiaHttpReportFetcher::default();
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live response must include report rows");
}

// ---------------------------------------------------------------------------
// Live integration tests (opt-in via TDW_EIA_LIVE=1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_eia_spot_price_returns_records_when_env_vars_set() {
    if std::env::var("TDW_EIA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_EIA_LIVE != 1; skipping live EIA spot-price integration test");
        return;
    }

    let query = EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 5)
        .unwrap_or_else(|e| panic!("query must construct: {e}"));
    let fetcher = EiaHttpSpotPriceFetcher::default();
    let records = live_fetch_nonempty!(fetcher, query);

    assert!(
        !records.is_empty(),
        "live response must include at least one spot-price record"
    );
}

#[tokio::test]
async fn live_eia_natural_gas_returns_records_when_env_vars_set() {
    if std::env::var("TDW_EIA_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_EIA_LIVE != 1; skipping live EIA natural-gas integration test");
        return;
    }

    let query = EiaNaturalGasQuery::new(5).unwrap_or_else(|e| panic!("query must construct: {e}"));
    let fetcher = EiaHttpNaturalGasFetcher::default();
    let records = live_fetch_nonempty!(fetcher, query);

    assert!(
        !records.is_empty(),
        "live response must include at least one natural-gas record"
    );
}
