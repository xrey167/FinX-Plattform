//! Tests for the real `congress.gov` v3 HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded `congress.gov`
//! response shapes without network access. The live tests are additionally gated
//! by `TDW_CONGRESS_GOV_LIVE=1` and require a free `CONGRESS_GOV_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_congress_gov::{
    CongressGovBillInfoFetcher, CongressGovBillTextFetcher, CongressGovBillsFetcher,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

#[test]
fn base_url_uses_tls() {
    assert!(
        tdw_provider_congress_gov::BASE_URL.starts_with("https://"),
        "congress.gov base URL must use TLS"
    );
}

fn bills_cassette() -> Bytes {
    cassette_bytes!({
        "bills": [
            {
                "congress": 118,
                "type": "HR",
                "number": "3076",
                "title": "Postal Service Reform Act of 2022",
                "originChamber": "House",
                "introducedDate": "2021-05-11",
                "latestAction": {
                    "actionDate": "2022-04-06",
                    "text": "Became Public Law No: 117-108."
                },
                "updateDate": "2022-09-29",
                "url": "https://api.congress.gov/v3/bill/118/hr/3076?format=json"
            },
            {
                "congress": 118,
                "type": "HR",
                "number": "",
                "title": "Missing number — must be skipped"
            }
        ]
    })
}

#[test]
fn bills_cassette_decodes_and_normalizes() {
    let fetcher = CongressGovBillsFetcher::default();
    let query = CongressGovBillsFetcher::transform_query(json!({
        "command": "uscongress/bills",
        "congress": 118,
        "bill_type": "hr"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, bills_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(
        rows.len(),
        1,
        "row with empty number skipped; rows={rows:#?}"
    );
    assert_eq!(rows[0].congress, 118);
    assert_eq!(rows[0].bill_type, "hr");
    assert_eq!(rows[0].bill_number, "3076");
    assert_eq!(
        rows[0].title.as_deref(),
        Some("Postal Service Reform Act of 2022")
    );
    assert_eq!(rows[0].origin_chamber.as_deref(), Some("House"));
    assert_eq!(rows[0].introduced_date.as_deref(), Some("2021-05-11"));
    assert_eq!(
        rows[0].latest_action.as_deref(),
        Some("Became Public Law No: 117-108.")
    );
    assert_eq!(rows[0].latest_action_date.as_deref(), Some("2022-04-06"));
}

fn bill_info_cassette() -> Bytes {
    cassette_bytes!({
        "bill": {
            "congress": 118,
            "type": "HR",
            "number": "3076",
            "title": "Postal Service Reform Act of 2022",
            "introducedDate": "2021-05-11",
            "policyArea": { "name": "Government Operations and Politics" },
            "sponsors": [ { "fullName": "Rep. Maloney, Carolyn B." } ],
            "latestAction": {
                "actionDate": "2022-04-06",
                "text": "Became Public Law No: 117-108."
            },
            "url": "https://api.congress.gov/v3/bill/118/hr/3076?format=json"
        }
    })
}

#[test]
fn bill_info_cassette_maps_sponsor_and_policy_area() {
    let fetcher = CongressGovBillInfoFetcher::default();
    let query = CongressGovBillInfoFetcher::transform_query(json!({
        "command": "uscongress/bill_info",
        "congress": 118,
        "bill_type": "hr",
        "bill_number": "3076"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, bill_info_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sponsor.as_deref(), Some("Rep. Maloney, Carolyn B."));
    assert_eq!(
        rows[0].policy_area.as_deref(),
        Some("Government Operations and Politics")
    );
}

fn bill_text_cassette() -> Bytes {
    cassette_bytes!({
        "textVersions": [
            {
                "type": "Introduced in House",
                "date": "2021-05-11",
                "formats": [
                    { "type": "Formatted Text", "url": "https://example.congress.gov/hr3076/ih.htm" },
                    { "type": "PDF", "url": "https://example.congress.gov/hr3076/ih.pdf" }
                ]
            },
            {
                "type": "No formats — skipped",
                "date": "2021-05-12"
            }
        ]
    })
}

#[test]
fn bill_text_cassette_flattens_versions_and_formats() {
    let fetcher = CongressGovBillTextFetcher::default();
    let query = CongressGovBillTextFetcher::transform_query(json!({
        "command": "uscongress/bill_text_urls",
        "congress": 118,
        "bill_type": "hr",
        "bill_number": "3076"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, bill_text_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "two formats of one version; rows={rows:#?}");
    assert_eq!(rows[0].version_type.as_deref(), Some("Introduced in House"));
    assert_eq!(rows[0].format_type, "Formatted Text");
    assert_eq!(rows[0].url, "https://example.congress.gov/hr3076/ih.htm");
    assert_eq!(rows[1].format_type, "PDF");
}

#[test]
fn bills_transform_data_rejects_malformed_json() {
    let fetcher = CongressGovBillsFetcher::default();
    let query = CongressGovBillsFetcher::transform_query(json!({
        "command": "uscongress/bills",
        "congress": 118,
        "bill_type": "hr"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let garbage = Bytes::from_static(b"{not valid json");
    let err = fetcher
        .transform_data(&query, garbage)
        .expect_err("malformed JSON must be propagated");
    assert!(err.to_string().contains("parse_json"), "got: {err}");
}

#[test]
fn transform_query_rejects_unknown_command() {
    assert!(CongressGovBillsFetcher::transform_query(json!({ "command": "bogus" })).is_err());
    assert!(CongressGovBillsFetcher::transform_query(json!({})).is_err());
}

#[tokio::test]
async fn live_congress_bills_returns_rows_when_env_set() {
    if std::env::var("TDW_CONGRESS_GOV_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_CONGRESS_GOV_LIVE != 1; skipping live congress.gov bills test");
        return;
    }
    let fetcher = CongressGovBillsFetcher::default();
    let query = CongressGovBillsFetcher::transform_query(json!({
        "command": "uscongress/bills",
        "congress": 118,
        "bill_type": "hr",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live congress.gov bills must include rows"
    );
}
