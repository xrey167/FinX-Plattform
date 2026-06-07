//! Tests for the real BLS HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! `timeseries/data` response shape. The live test is additionally gated
//! by `TDW_BLS_LIVE=1` and benefits from `TDW_BLS_API_KEY` (optional).

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_bls::{BlsHttpTimeSeriesFetcher, BlsSeriesQuery};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn sample_query() -> BlsSeriesQuery {
    BlsHttpTimeSeriesFetcher::transform_query(json!({
        "series_ids": ["CUUR0000SA0"],
        "start_year": 2023,
        "end_year": 2024
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn cassette_bytes() -> Bytes {
    cassette_bytes!({
        "status": "REQUEST_SUCCEEDED",
        "responseTime": 51,
        "message": [],
        "Results": {
            "series": [
                {
                    "seriesID": "CUUR0000SA0",
                    "data": [
                        {
                            "year": "2024",
                            "period": "M01",
                            "periodName": "January",
                            "value": "308.417",
                            "footnotes": [{}]
                        },
                        {
                            "year": "2023",
                            "period": "M12",
                            "periodName": "December",
                            "value": "306.746",
                            "footnotes": [{}]
                        },
                        {
                            "year": "2023",
                            "period": "M01",
                            "periodName": "January",
                            "value": "299.170",
                            "footnotes": [{}]
                        }
                    ]
                }
            ]
        }
    })
}

#[test]
fn cassette_replay_decodes_all_data_points() {
    let fetcher = BlsHttpTimeSeriesFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    assert_eq!(rows[0].series_id, "CUUR0000SA0");
    assert_eq!(rows[0].year, "2024");
    assert_eq!(rows[0].period, "M01");
    assert_eq!(rows[0].period_name, "January");
    assert!((rows[0].value - 308.417).abs() < f64::EPSILON);
    assert_eq!(rows[2].year, "2023");
    assert_eq!(rows[2].period, "M01");
    assert!((rows[2].value - 299.170).abs() < f64::EPSILON);
}

#[test]
fn cassette_replay_surfaces_failed_status() {
    let fetcher = BlsHttpTimeSeriesFetcher::default();
    let query = sample_query();
    let envelope = cassette_bytes!({
        "status": "REQUEST_FAILED",
        "responseTime": 10,
        "message": ["Your request could not be processed."],
        "Results": {}
    });

    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("failed status must propagate as error");

    assert!(
        err.to_string().contains("REQUEST_FAILED")
            || err
                .to_string()
                .contains("Your request could not be processed"),
        "unexpected error: {err}"
    );
}

#[test]
fn transform_query_validates_series_ids_and_years() {
    // valid
    let q = BlsHttpTimeSeriesFetcher::transform_query(json!({
        "series_ids": ["LNS14000000", "CES0000000001"],
        "start_year": 2020,
        "end_year": 2023
    }))
    .unwrap_or_else(|e| panic!("valid params must succeed: {e}"));

    assert_eq!(q.series_ids.len(), 2);
    assert_eq!(q.start_year, 2020);

    // empty series_ids
    assert!(
        BlsHttpTimeSeriesFetcher::transform_query(json!({
            "series_ids": [],
            "start_year": 2020,
            "end_year": 2023
        }))
        .is_err(),
        "empty series_ids must fail"
    );

    // inverted years
    assert!(
        BlsHttpTimeSeriesFetcher::transform_query(json!({
            "series_ids": ["CUUR0000SA0"],
            "start_year": 2024,
            "end_year": 2020
        }))
        .is_err(),
        "inverted year range must fail"
    );

    // injection attempt in series ID
    assert!(
        BlsHttpTimeSeriesFetcher::transform_query(json!({
            "series_ids": ["CPI&evil=1"],
            "start_year": 2020,
            "end_year": 2024
        }))
        .is_err(),
        "series ID with injection chars must fail"
    );
}

#[test]
fn multi_series_cassette_decodes_all_series() {
    let fetcher = BlsHttpTimeSeriesFetcher::default();
    let query = BlsHttpTimeSeriesFetcher::transform_query(json!({
        "series_ids": ["CUUR0000SA0", "LNS14000000"],
        "start_year": 2024,
        "end_year": 2024
    }))
    .unwrap_or_else(|e| panic!("query must build: {e}"));

    let multi_cassette = cassette_bytes!({
        "status": "REQUEST_SUCCEEDED",
        "responseTime": 75,
        "message": [],
        "Results": {
            "series": [
                {
                    "seriesID": "CUUR0000SA0",
                    "data": [
                        {
                            "year": "2024",
                            "period": "M01",
                            "periodName": "January",
                            "value": "308.417",
                            "footnotes": [{}]
                        }
                    ]
                },
                {
                    "seriesID": "LNS14000000",
                    "data": [
                        {
                            "year": "2024",
                            "period": "M01",
                            "periodName": "January",
                            "value": "3.7",
                            "footnotes": [{}]
                        }
                    ]
                }
            ]
        }
    });

    let rows = fetcher
        .transform_data(&query, multi_cassette)
        .unwrap_or_else(|e| panic!("multi-series transform must succeed: {e}"));

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].series_id, "CUUR0000SA0");
    assert_eq!(rows[1].series_id, "LNS14000000");
    assert!((rows[1].value - 3.7).abs() < f64::EPSILON);
}

#[tokio::test]
async fn live_bls_returns_cpi_data_when_env_vars_set() {
    if std::env::var("TDW_BLS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_BLS_LIVE != 1; skipping live BLS integration test");
        return;
    }

    let fetcher = BlsHttpTimeSeriesFetcher::default();
    let query = BlsHttpTimeSeriesFetcher::transform_query(json!({
        "series_ids": ["CUUR0000SA0"],
        "start_year": 2023,
        "end_year": 2024
    }))
    .unwrap_or_else(|e| panic!("query must build: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live BLS response must include at least one data point"
    );
    assert_eq!(rows[0].series_id, "CUUR0000SA0");
}
