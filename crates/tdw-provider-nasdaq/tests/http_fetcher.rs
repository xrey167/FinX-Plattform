//! Tests for the real NASDAQ Data Link HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! `dataset_data` response shape. The live test is additionally gated
//! by `TDW_NASDAQ_LIVE=1` and requires `TDW_NASDAQ_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_nasdaq::{NasdaqDatasetQuery, NasdaqHttpDatasetFetcher};

fn sample_query() -> NasdaqDatasetQuery {
    NasdaqHttpDatasetFetcher::transform_query(json!({
        "database": "WIKI",
        "dataset": "AAPL",
        "start_date": "2024-01-02",
        "end_date": "2024-01-05"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!({
            "dataset_data": {
                "id": 9775687,
                "dataset_code": "AAPL",
                "database_code": "WIKI",
                "start_date": "2024-01-02",
                "end_date": "2024-01-05",
                "column_names": ["Date", "Open", "High", "Low", "Close", "Volume"],
                "frequency": "daily",
                "data": [
                    ["2024-01-02", 185.6, 186.1, 184.4, 185.2, 55000000],
                    ["2024-01-03", 184.0, 185.5, 183.1, 184.8, 48000000],
                    ["2024-01-04", 183.9, 185.0, 182.7, 184.1, 52000000],
                    ["2024-01-05", 184.5, 186.3, 184.0, 185.9, 61000000]
                ]
            }
        })
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_nasdaq_dataset_into_rows() {
    let fetcher = NasdaqHttpDatasetFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 4, "rows={rows:#?}");
    assert_eq!(rows[0].database, "WIKI");
    assert_eq!(rows[0].dataset, "AAPL");
    assert_eq!(
        rows[0].column_names,
        ["Date", "Open", "High", "Low", "Close", "Volume"]
    );
    assert_eq!(
        rows[0].values[0],
        serde_json::Value::String("2024-01-02".to_string())
    );
    assert_eq!(
        rows[3].values[0],
        serde_json::Value::String("2024-01-05".to_string())
    );
}

#[test]
fn cassette_replay_surfaces_nasdaq_error_envelope() {
    let fetcher = NasdaqHttpDatasetFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "quandl_error": {
                "code": "QEAx01",
                "message": "You have submitted an incorrect Quandl code."
            }
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("nasdaq api error"));
    assert!(err.to_string().contains("QEAx01"));
}

#[test]
fn cassette_replay_errors_on_missing_dataset_data() {
    let fetcher = NasdaqHttpDatasetFetcher::default();
    let query = sample_query();
    let empty = Bytes::from(json!({}).to_string().into_bytes());
    let err = fetcher
        .transform_data(&query, empty)
        .expect_err("missing dataset_data must be an error");

    assert!(err.to_string().contains("missing dataset_data"));
}

#[test]
fn transform_query_normalizes_identifiers_and_rejects_injection() {
    let query = NasdaqHttpDatasetFetcher::transform_query(json!({
        "database": "wiki",
        "dataset": "aapl"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.database, "WIKI");
    assert_eq!(query.dataset, "AAPL");

    assert!(
        NasdaqHttpDatasetFetcher::transform_query(json!({
            "database": "WIKI/../../secret",
            "dataset": "AAPL"
        }))
        .is_err()
    );
    assert!(
        NasdaqHttpDatasetFetcher::transform_query(json!({
            "database": "WIKI",
            "dataset": "AAPL?api_key=stolen"
        }))
        .is_err()
    );
}

#[test]
fn transform_query_passes_through_optional_dates() {
    let query = NasdaqHttpDatasetFetcher::transform_query(json!({
        "database": "FRED",
        "dataset": "GDP",
        "start_date": "2023-01-01",
        "end_date": "2023-12-31"
    }))
    .unwrap_or_else(|error| panic!("query with dates should transform: {error}"));

    assert_eq!(query.start_date.as_deref(), Some("2023-01-01"));
    assert_eq!(query.end_date.as_deref(), Some("2023-12-31"));
}

#[test]
fn transform_query_rejects_bad_dates() {
    assert!(
        NasdaqHttpDatasetFetcher::transform_query(json!({
            "database": "WIKI",
            "dataset": "AAPL",
            "start_date": "20240101"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_nasdaq_returns_data_when_env_var_set() {
    if std::env::var("TDW_NASDAQ_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_NASDAQ_LIVE != 1; skipping live NASDAQ Data Link integration test");
        return;
    }

    let fetcher = NasdaqHttpDatasetFetcher::default();
    let query = NasdaqHttpDatasetFetcher::transform_query(json!({
        "database": "FRED",
        "dataset": "GDP",
        "start_date": "2023-01-01",
        "end_date": "2023-12-31"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one row"
    );
    assert_eq!(rows[0].database, "FRED");
    assert_eq!(rows[0].dataset, "GDP");
}
