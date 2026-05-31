#![cfg(feature = "http")]
//! Tests for the real EIA HTTP fetchers.
//!
//! Cassette tests always run under the `http` feature and parse recorded
//! response shapes inline. The live tests are additionally gated by
//! `TDW_EIA_LIVE=1` and require `TDW_EIA_API_KEY`.

use tdw_provider_eia::{
    EiaCommodity, EiaHttpNaturalGasFetcher, EiaHttpSpotPriceFetcher, EiaNaturalGasQuery,
    EiaSpotPriceQuery,
};

// ---------------------------------------------------------------------------
// Spot-price cassette
// ---------------------------------------------------------------------------

fn spot_price_cassette() -> Vec<u8> {
    serde_json::json!({
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
    .to_string()
    .into_bytes()
}

#[test]
fn cassette_spot_price_decodes_records_and_skips_missing_values() {
    let fetcher = EiaHttpSpotPriceFetcher::default();
    let records = fetcher
        .parse_bytes(&spot_price_cassette())
        .unwrap_or_else(|e| panic!("parse_bytes must succeed: {e}"));

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
        .parse_bytes(b"not json at all")
        .expect_err("invalid JSON must be rejected");
}

#[test]
fn cassette_spot_price_rejects_non_numeric_value() {
    let fetcher = EiaHttpSpotPriceFetcher::default();
    let bad = serde_json::json!({
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
    })
    .to_string()
    .into_bytes();
    fetcher
        .parse_bytes(&bad)
        .expect_err("non-numeric value must be rejected");
}

// ---------------------------------------------------------------------------
// Natural-gas cassette
// ---------------------------------------------------------------------------

fn natural_gas_cassette() -> Vec<u8> {
    serde_json::json!({
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
    .to_string()
    .into_bytes()
}

#[test]
fn cassette_natural_gas_decodes_records_and_skips_missing_values() {
    let fetcher = EiaHttpNaturalGasFetcher::default();
    let records = fetcher
        .parse_bytes(&natural_gas_cassette())
        .unwrap_or_else(|e| panic!("parse_bytes must succeed: {e}"));

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
        .parse_bytes(b"{bad}")
        .expect_err("invalid JSON must be rejected");
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
    let records = fetcher
        .fetch(&query)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

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
    let records = fetcher
        .fetch(&query)
        .await
        .unwrap_or_else(|e| panic!("live fetch must succeed: {e}"));

    assert!(
        !records.is_empty(),
        "live response must include at least one natural-gas record"
    );
}
