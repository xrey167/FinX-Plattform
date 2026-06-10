//! Tests for the real Federal Reserve HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise). Cassette tests always
//! run under the feature; live tests are additionally gated by
//! `TDW_FEDERAL_RESERVE_LIVE=1` (keyless portals).

#![cfg(feature = "http")]

use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_federal_reserve::{FedFomcDocumentsHttpFetcher, FedMacroSeriesHttpFetcher};
use tdw_provider_testkit::cassette_bytes;

#[test]
fn macro_fetcher_normalizes_money_measures_observations() {
    let fetcher = FedMacroSeriesHttpFetcher::default();
    let query = FedMacroSeriesHttpFetcher::transform_query(json!({
        "command": "economy/money_measures"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "observations": [
            { "date": "2024-01-01", "value": "20800.5" },
            { "date": "2024-02-01", "value": "." },
            { "date": "2024-03-01", "value": "20950.1" }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 3, "missing value kept as None; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "H6_M2_SA");
    assert_eq!(rows[0].value, Some(20800.5));
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[0].frequency.as_deref(), Some("monthly"));
    assert!(
        rows[0]
            .title
            .as_deref()
            .unwrap_or_default()
            .contains("Money Stock")
    );
}

#[test]
fn macro_fetcher_tags_dealer_stats_series_id() {
    let fetcher = FedMacroSeriesHttpFetcher::default();
    let query = FedMacroSeriesHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/dealer_stats"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let raw = cassette_bytes!({
        "observations": [ { "date": "2024-06-05", "value": "123456" } ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows[0].series_id, "PD_NET_POSITIONS");
    assert_eq!(rows[0].value, Some(123_456.0));
}

#[test]
fn macro_fetcher_rejects_fomc_command_and_unknown() {
    // The macro fetcher must reject the document-index command and unknowns.
    assert!(
        FedMacroSeriesHttpFetcher::transform_query(json!({
            "command": "regulators/fed/fomc_documents"
        }))
        .is_err()
    );
    assert!(FedMacroSeriesHttpFetcher::transform_query(json!({ "command": "bogus" })).is_err());
}

#[test]
fn macro_fetcher_surfaces_api_error() {
    let fetcher = FedMacroSeriesHttpFetcher::default();
    let query = FedMacroSeriesHttpFetcher::transform_query(json!({
        "command": "economy/money_measures"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let raw = cassette_bytes!({ "error": "service unavailable" });
    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("error envelope must propagate");
    assert!(err.to_string().contains("api error"));
}

#[test]
fn fomc_fetcher_normalizes_document_index() {
    let fetcher = FedFomcDocumentsHttpFetcher::default();
    let query = FedFomcDocumentsHttpFetcher::transform_query(json!({
        "command": "regulators/fed/fomc_documents"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let raw = cassette_bytes!({
        "documents": [
            {
                "type": "statement",
                "date": "2024-05-01",
                "title": "FOMC Statement May 2024",
                "url": "https://www.federalreserve.gov/newsevents/pressreleases/monetary20240501a.htm"
            },
            {
                "type": "",
                "date": "2024-05-01",
                "title": "skipped: empty type"
            }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows.len(), 1, "empty-type row skipped; rows={rows:#?}");
    assert_eq!(rows[0].doc_type, "statement");
    assert_eq!(rows[0].date.as_deref(), Some("2024-05-01"));
    assert!(
        rows[0]
            .title
            .as_deref()
            .unwrap_or_default()
            .contains("FOMC")
    );
}

#[test]
fn fomc_fetcher_rejects_macro_command() {
    assert!(
        FedFomcDocumentsHttpFetcher::transform_query(json!({
            "command": "economy/money_measures"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_federal_reserve_money_measures_when_env_set() {
    if std::env::var("TDW_FEDERAL_RESERVE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FEDERAL_RESERVE_LIVE != 1; skipping live Fed money_measures test");
        return;
    }
    let fetcher = FedMacroSeriesHttpFetcher::default();
    let query = FedMacroSeriesHttpFetcher::transform_query(json!({
        "command": "economy/money_measures",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    // Live: a successful fetch must decode without error (rows may be empty if
    // the release JSON shape differs; the standardized contract is no panic).
    let raw = fetcher
        .extract_data(&query, &tdw_core::Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let _ = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));
}
