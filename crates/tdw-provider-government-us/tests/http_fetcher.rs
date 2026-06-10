//! Tests for the real US Treasury `FiscalData` HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded `FiscalData`
//! `{ "data": [...] }` response shapes without network access. The live tests
//! are additionally gated by `TDW_GOVERNMENT_US_LIVE=1` (keyless API).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_government_us::{
    GovUsTreasuryAuctionsHttpFetcher, GovUsTreasuryPricesHttpFetcher,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn auctions_cassette() -> Bytes {
    // FiscalData returns numeric columns as strings; the fetcher coerces them.
    cassette_bytes!({
        "data": [
            {
                "cusip": "912797GZ8",
                "security_type": "Bill",
                "security_term": "4-Week",
                "auction_date": "2024-06-04",
                "issue_date": "2024-06-06",
                "maturity_date": "2024-07-02",
                "high_yield": "5.270",
                "interest_rate": "",
                "offering_amount": "80000000000",
                "bid_to_cover_ratio": "2.85"
            },
            {
                "cusip": "",
                "auction_date": "2024-06-04",
                "security_type": "skipped: empty cusip"
            }
        ],
        "meta": { "count": 2 }
    })
}

#[test]
fn auctions_cassette_decodes_and_coerces_string_numbers() {
    let fetcher = GovUsTreasuryAuctionsHttpFetcher::default();
    let query = GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_auctions"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, auctions_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "empty-cusip row skipped; rows={rows:#?}");
    assert_eq!(rows[0].cusip, "912797GZ8");
    assert_eq!(rows[0].security_type.as_deref(), Some("Bill"));
    assert_eq!(rows[0].security_term.as_deref(), Some("4-Week"));
    assert_eq!(rows[0].auction_date, "2024-06-04");
    assert_eq!(rows[0].high_yield, Some(5.270));
    assert_eq!(rows[0].interest_rate, None);
    assert_eq!(rows[0].offering_amount, Some(80_000_000_000.0));
    assert_eq!(rows[0].bid_to_cover_ratio, Some(2.85));
}

#[test]
fn auctions_cassette_surfaces_api_error() {
    let fetcher = GovUsTreasuryAuctionsHttpFetcher::default();
    let query = GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_auctions"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let envelope = cassette_bytes!({ "error": "Bad Request" });
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");
    assert!(err.to_string().contains("api error"));
}

#[test]
fn auctions_transform_query_rejects_unknown_command() {
    assert!(
        GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({ "command": "bogus" })).is_err()
    );
    assert!(GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({})).is_err());
}

fn prices_cassette() -> Bytes {
    cassette_bytes!({
        "data": [
            {
                "cusip": "91282CKQ1",
                "security_type": "Note",
                "record_date": "2024-06-03",
                "price": "99.875",
                "interest_rate": "4.375",
                "maturity_date": "2034-05-15"
            }
        ],
        "meta": { "count": 1 }
    })
}

#[test]
fn prices_cassette_decodes_records() {
    let fetcher = GovUsTreasuryPricesHttpFetcher::default();
    let query = GovUsTreasuryPricesHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_prices"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, prices_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].cusip, "91282CKQ1");
    assert_eq!(rows[0].date, "2024-06-03");
    assert_eq!(rows[0].price, Some(99.875));
    assert_eq!(rows[0].coupon_rate, Some(4.375));
    assert_eq!(rows[0].security_type.as_deref(), Some("Note"));
}

#[tokio::test]
async fn live_government_us_auctions_returns_rows_when_env_set() {
    if std::env::var("TDW_GOVERNMENT_US_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_GOVERNMENT_US_LIVE != 1; skipping live FiscalData auctions test");
        return;
    }
    let fetcher = GovUsTreasuryAuctionsHttpFetcher::default();
    let query = GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_auctions",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live auctions must include rows");
}

#[tokio::test]
async fn live_government_us_prices_returns_rows_when_env_set() {
    if std::env::var("TDW_GOVERNMENT_US_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_GOVERNMENT_US_LIVE != 1; skipping live FiscalData prices test");
        return;
    }
    let fetcher = GovUsTreasuryPricesHttpFetcher::default();
    let query = GovUsTreasuryPricesHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_prices",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live prices must include rows");
}
