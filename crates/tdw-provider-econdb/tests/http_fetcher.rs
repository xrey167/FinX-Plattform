//! Tests for the real `EconDB` series HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded `EconDB` series
//! response shapes without network access. They cover BOTH documented `data`
//! shapes (the parallel `dates`/`values` arrays AND the list-of-`{date,value}`
//! records), the numeric-string→`f64` coercion, a null/missing value → row with
//! `None`, a non-object-root error path, malformed JSON, and ticker-validation
//! rejects. The live test is additionally gated by `TDW_ECONDB_LIVE=1` (keyless
//! public series).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_econdb::EcondbHttpMacroSeriesFetcher;
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

/// A series response in the parallel-arrays shape: a `data` object with parallel
/// `dates` and `values` arrays. The second value is `null` (a missing observation
/// that must decode to a `None` value while still emitting a row for its date);
/// the third value is a numeric string (must coerce to `f64`).
fn parallel_arrays_cassette() -> Bytes {
    cassette_bytes!({
        "ticker": "RGDPUS",
        "description": "Real Gross Domestic Product, United States",
        "geography": "US",
        "frequency": "Q",
        "units": "USD billions",
        "data": {
            "dates": ["2022-01-01", "2022-04-01", "2022-07-01"],
            "values": [100.5, null, "200.25"]
        }
    })
}

/// A series response in the list-of-records shape: `data` is an array of
/// `{date, value}` objects. One record omits its value entirely (→ `None` row).
fn record_list_cassette() -> Bytes {
    cassette_bytes!({
        "ticker": "RGDPGB",
        "description": "Real Gross Domestic Product, United Kingdom",
        "geography": "GB",
        "frequency": "A",
        "units": "GBP billions",
        "data": [
            { "date": "2021", "value": "10.0" },
            { "date": "2022" },
            { "date": "2023", "value": 12.5 }
        ]
    })
}

fn series_query() -> serde_json::Value {
    json!({
        "command": "economy/econdb/series",
        "ticker": "RGDPUS"
    })
}

#[test]
fn parallel_arrays_decode_with_null_and_string_value() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(series_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, parallel_arrays_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 3, "three dates -> three rows; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "RGDPUS");
    assert_eq!(rows[0].date, "2022-01-01");
    assert_eq!(rows[0].value, Some(100.5));
    assert_eq!(rows[0].country.as_deref(), Some("US"));
    assert_eq!(rows[0].frequency.as_deref(), Some("Q"));
    assert_eq!(rows[0].unit.as_deref(), Some("USD billions"));
    assert_eq!(
        rows[0].title.as_deref(),
        Some("Real Gross Domestic Product, United States")
    );
    // null value -> a row with a None value, not a dropped row.
    assert_eq!(rows[1].date, "2022-04-01");
    assert_eq!(rows[1].value, None);
    // The JSON string "200.25" is coerced to an f64.
    assert_eq!(rows[2].date, "2022-07-01");
    assert_eq!(rows[2].value, Some(200.25));
}

#[test]
fn record_list_decodes_all_rows_with_missing_value() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/econdb/series",
        "ticker": "RGDPGB"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, record_list_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert_eq!(rows[0].series_id, "RGDPGB");
    assert_eq!(rows[0].date, "2021");
    assert_eq!(rows[0].value, Some(10.0));
    // Missing `value` field -> a row with a None value, not a dropped row.
    assert_eq!(rows[1].date, "2022");
    assert_eq!(rows[1].value, None);
    assert_eq!(rows[2].date, "2023");
    assert_eq!(rows[2].value, Some(12.5));
    assert_eq!(rows[2].country.as_deref(), Some("GB"));
}

#[test]
fn limit_caps_emitted_rows() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/econdb/series",
        "ticker": "RGDPUS",
        "limit": 2
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = fetcher
        .transform_data(&query, parallel_arrays_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(rows.len(), 2, "limit=2 caps the three observations");
}

#[test]
fn missing_data_node_yields_empty_not_error() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(series_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    // An unknown ticker / out-of-range window returns an object with no `data`.
    let envelope = cassette_bytes!({ "ticker": "RGDPUS", "detail": "no observations" });
    let rows = fetcher
        .transform_data(&query, envelope)
        .unwrap_or_else(|error| panic!("empty envelope must decode to no rows: {error}"));
    assert!(rows.is_empty(), "rows={rows:#?}");
}

#[test]
fn transform_data_rejects_malformed_json() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(series_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let garbage = Bytes::from_static(b"{not valid json");
    let err = fetcher
        .transform_data(&query, garbage)
        .expect_err("malformed JSON must be propagated");
    assert!(err.to_string().contains("parse_json"), "got: {err}");
}

#[test]
fn transform_data_rejects_non_object_root() {
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(series_query())
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let array_root = cassette_bytes!([1, 2, 3]);
    let err = fetcher
        .transform_data(&query, array_root)
        .expect_err("non-object root must be propagated");
    assert!(
        err.to_string().contains("expected a JSON object"),
        "got: {err}"
    );
}

#[test]
fn transform_query_rejects_unknown_command_invalid_ticker_and_missing_ticker() {
    // Unknown command.
    assert!(
        EcondbHttpMacroSeriesFetcher::transform_query(json!({
            "command": "bogus",
            "ticker": "RGDPUS"
        }))
        .is_err()
    );
    // Missing ticker.
    assert!(
        EcondbHttpMacroSeriesFetcher::transform_query(json!({
            "command": "economy/econdb/series"
        }))
        .is_err()
    );
    // Invalid ticker (path-injection attempt rejected by the ticker grammar).
    assert!(
        EcondbHttpMacroSeriesFetcher::transform_query(json!({
            "command": "economy/econdb/series",
            "ticker": "RGDP/../x"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_econdb_series_returns_rows_when_env_set() {
    if std::env::var("TDW_ECONDB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ECONDB_LIVE != 1; skipping live EconDB series test");
        return;
    }
    let fetcher = EcondbHttpMacroSeriesFetcher::default();
    let query = EcondbHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/econdb/series",
        "ticker": "RGDPUS"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live EconDB series must include rows");
}
