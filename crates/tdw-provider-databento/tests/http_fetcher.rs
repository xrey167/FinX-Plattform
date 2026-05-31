#![cfg(feature = "http")]

use bytes::Bytes;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_databento::DatabentoTimeseriesQuery;
use tdw_provider_databento::http_fetcher::{
    DatabentoHttpTimeseriesFetcher, DatabentoMetadataFetcher, DatabentoMetadataQuery,
};

// ---------------------------------------------------------------------------
// Cassette tests — parse inline JSON without any network call
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_timeseries_response() {
    let raw = Bytes::from_static(
        br#"{"records":[{"ts_event":1704153600000000000,"open":4823.0,"high":4856.0,"low":4810.0,"close":4844.0,"volume":285000.0}]}"#,
    );
    let fetcher = DatabentoHttpTimeseriesFetcher::default();
    let query = DatabentoTimeseriesQuery::new(
        "GLBX.MDP3",
        vec!["ESH5".to_string()],
        "2024-01-01",
        "2024-01-31",
    )
    .expect("query should be valid");

    let bars = fetcher
        .transform_data(&query, raw)
        .expect("transform_data should succeed");

    assert_eq!(bars.len(), 1);
    let bar = &bars[0];
    assert_eq!(bar.symbol, "ESH5");
    assert_eq!(bar.venue, "GLBX.MDP3");
    assert_eq!(bar.ts, "2024-01-02T00:00:00Z");
    assert!((bar.open - 4823.0).abs() < f64::EPSILON);
    assert!((bar.high - 4856.0).abs() < f64::EPSILON);
    assert!((bar.low - 4810.0).abs() < f64::EPSILON);
    assert!((bar.close - 4844.0).abs() < f64::EPSILON);
    assert!((bar.volume - 285_000.0).abs() < f64::EPSILON);
    assert_eq!(bar.source, "databento");
}

#[test]
fn cassette_parse_metadata_response() {
    let raw = Bytes::from_static(br#"{"result":["GLBX.MDP3","XNAS.ITCH","OPRA.PILLAR"]}"#);
    let fetcher = DatabentoMetadataFetcher::default();
    let query = DatabentoMetadataQuery::default();

    let datasets = fetcher
        .transform_data(&query, raw)
        .expect("transform_data should succeed");

    assert_eq!(datasets.len(), 3);
    assert_eq!(datasets[0].id, "GLBX.MDP3");
    assert_eq!(datasets[1].id, "XNAS.ITCH");
    assert_eq!(datasets[2].id, "OPRA.PILLAR");
}

#[test]
fn cassette_parse_empty_timeseries_response() {
    let raw = Bytes::from_static(br#"{"records":[]}"#);
    let fetcher = DatabentoHttpTimeseriesFetcher::default();
    let query = DatabentoTimeseriesQuery::new(
        "GLBX.MDP3",
        vec!["ESH5".to_string()],
        "2024-01-01",
        "2024-01-31",
    )
    .expect("query should be valid");

    let bars = fetcher
        .transform_data(&query, raw)
        .expect("empty records should be valid");

    assert!(bars.is_empty());
}

// ---------------------------------------------------------------------------
// Live integration tests — skipped unless TDW_DATABENTO_LIVE=1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_timeseries_returns_data_when_env_var_set() {
    if std::env::var("TDW_DATABENTO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_DATABENTO_LIVE != 1; skipping live Databento timeseries integration test");
        return;
    }

    let fetcher = DatabentoHttpTimeseriesFetcher::default();
    let query = DatabentoTimeseriesQuery::new(
        "GLBX.MDP3",
        vec!["ESH5".to_string()],
        "2024-01-01",
        "2024-01-31",
    )
    .expect("query should be valid");
    let creds = Credentials::default();

    let result = fetcher.extract_data(&query, &creds).await;
    assert!(result.is_ok(), "live timeseries call failed: {result:?}");

    let bars = fetcher
        .transform_data(&query, result.expect("already checked"))
        .expect("transform_data should succeed on live data");
    assert!(!bars.is_empty(), "expected at least one bar from Databento");
}

#[tokio::test]
async fn live_metadata_returns_datasets_when_env_var_set() {
    if std::env::var("TDW_DATABENTO_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_DATABENTO_LIVE != 1; skipping live Databento metadata integration test");
        return;
    }

    let fetcher = DatabentoMetadataFetcher::default();
    let query = DatabentoMetadataQuery::default();
    let creds = Credentials::default();

    let result = fetcher.extract_data(&query, &creds).await;
    assert!(result.is_ok(), "live metadata call failed: {result:?}");

    let datasets = fetcher
        .transform_data(&query, result.expect("already checked"))
        .expect("transform_data should succeed on live metadata");
    assert!(
        !datasets.is_empty(),
        "expected at least one dataset from Databento"
    );
}
