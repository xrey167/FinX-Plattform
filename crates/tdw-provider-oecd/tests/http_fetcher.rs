//! Tests for the real OECD HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! SDMX-JSON response shape. The live test is additionally gated by
//! `TDW_OECD_LIVE=1`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_oecd::{OecdHttpDataFetcher, OecdQuery};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn sample_query() -> OecdQuery {
    OecdHttpDataFetcher::transform_query(json!({
        "dataset": "QNA",
        "filter": "AUS.B1GQ.Q",
        "start_time": "2023",
        "end_time": "2024"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

/// A cassette recording of the OECD SDMX-JSON response shape with two
/// observations and one null value.
fn cassette_bytes() -> Bytes {
    cassette_bytes!({
        "dataSets": [{
            "observations": {
                "0:0:0:0": [1234.5, 0, null],
                "0:0:0:1": [null, 0, null],
                "0:0:0:2": [5678.9, 0, null]
            }
        }],
        "structure": {
            "dimensions": {
                "observation": [
                    {
                        "id": "TIME_PERIOD",
                        "values": [
                            { "id": "2023-Q1", "name": "2023-Q1" },
                            { "id": "2023-Q2", "name": "2023-Q2" },
                            { "id": "2023-Q3", "name": "2023-Q3" }
                        ]
                    }
                ]
            }
        }
    })
}

#[test]
fn cassette_replay_decodes_observations_and_skips_nulls() {
    let fetcher = OecdHttpDataFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    // null at index 1 is skipped; 0:0:0:0 and 0:0:0:2 survive.
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    // Sorted by key: "0:0:0:0" < "0:0:0:2"
    assert_eq!(rows[0].key, "0:0:0:0");
    assert_eq!(rows[0].period, "2023-Q1");
    assert_eq!(rows[0].value, 1234.5);
    assert_eq!(rows[0].dataset, "QNA");
    assert_eq!(rows[1].key, "0:0:0:2");
    assert_eq!(rows[1].period, "2023-Q3");
    assert_eq!(rows[1].value, 5678.9);
}

#[test]
fn cassette_replay_empty_data_sets_returns_empty_vec() {
    let fetcher = OecdHttpDataFetcher::default();
    let query = sample_query();
    let empty = cassette_bytes!({
        "dataSets": [],
        "structure": { "dimensions": { "observation": [] } }
    });
    let rows = fetcher
        .transform_data(&query, empty)
        .unwrap_or_else(|e| panic!("empty dataSets must succeed: {e}"));

    assert!(rows.is_empty(), "expected empty vec, got {rows:#?}");
}

#[test]
fn cassette_replay_missing_structure_falls_back_to_key_as_period() {
    let fetcher = OecdHttpDataFetcher::default();
    let query = sample_query();
    let no_structure = cassette_bytes!({
        "dataSets": [{
            "observations": {
                "0:0:0:0": [99.0, 0, null]
            }
        }]
    });
    let rows = fetcher
        .transform_data(&query, no_structure)
        .unwrap_or_else(|e| panic!("no-structure must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    // No TIME_PERIOD dimension → falls back to the obs key itself.
    assert_eq!(rows[0].period, "0:0:0:0");
    assert_eq!(rows[0].value, 99.0);
}

#[test]
fn transform_query_rejects_invalid_dataset() {
    let err = OecdHttpDataFetcher::transform_query(json!({
        "dataset": "QNA&drop=1",
        "filter": "AUS",
        "start_time": "2023",
        "end_time": "2024"
    }))
    .expect_err("invalid dataset must be rejected");

    assert!(err.to_string().contains("InvalidDataset") || err.to_string().contains("unsupported"));
}

#[test]
fn transform_query_rejects_missing_field() {
    let err = OecdHttpDataFetcher::transform_query(json!({
        "dataset": "QNA",
        "filter": "AUS"
        // start_time and end_time intentionally absent
    }))
    .expect_err("missing fields must be rejected");

    assert!(
        err.to_string().contains("start_time") || err.to_string().contains("must be a string"),
        "err={err}"
    );
}

#[tokio::test]
async fn live_oecd_returns_observations_when_env_var_set() {
    if std::env::var("TDW_OECD_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_OECD_LIVE != 1; skipping live OECD integration test");
        return;
    }

    let fetcher = OecdHttpDataFetcher::default();
    let query = OecdHttpDataFetcher::transform_query(json!({
        "dataset": "QNA",
        "filter": "AUS.B1GQ.Q",
        "start_time": "2022",
        "end_time": "2023"
    }))
    .unwrap_or_else(|e| panic!("live query must build: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one observation"
    );
    assert_eq!(rows[0].dataset, "QNA");
}
