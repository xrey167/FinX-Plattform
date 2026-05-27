//! Tests for the real FRED HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded
//! `series/observations` response shape. The live test is additionally
//! gated by `TDW_FRED_LIVE=1` and requires `FRED_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_fred::{FredHttpSeriesObservationsFetcher, FredSeriesObservationsQuery};

fn sample_query() -> FredSeriesObservationsQuery {
    FredHttpSeriesObservationsFetcher::transform_query(json!({ "series_id": "gdp" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!({
            "realtime_start": "2026-05-27",
            "realtime_end": "2026-05-27",
            "observation_start": "1776-07-04",
            "observation_end": "9999-12-31",
            "units": "lin",
            "output_type": 1,
            "file_type": "json",
            "order_by": "observation_date",
            "sort_order": "asc",
            "count": 3,
            "offset": 0,
            "limit": 100000,
            "observations": [
                {
                    "realtime_start": "2026-05-27",
                    "realtime_end": "2026-05-27",
                    "date": "2023-01-01",
                    "value": "27164.359"
                },
                {
                    "realtime_start": "2026-05-27",
                    "realtime_end": "2026-05-27",
                    "date": "2023-04-01",
                    "value": "."
                },
                {
                    "realtime_start": "2026-05-27",
                    "realtime_end": "2026-05-27",
                    "date": "2023-07-01",
                    "value": "27967.697"
                }
            ]
        })
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_observations_and_skips_missing_values() {
    let fetcher = FredHttpSeriesObservationsFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].series_id, "GDP");
    assert_eq!(rows[0].date, "2023-01-01");
    assert_eq!(rows[0].value, 27164.359);
    assert_eq!(rows[1].date, "2023-07-01");
    assert_eq!(rows[1].value, 27967.697);
}

#[test]
fn cassette_replay_surfaces_fred_error_envelope() {
    let fetcher = FredHttpSeriesObservationsFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "error_code": 400,
            "error_message": "Bad Request. Variable api_key is required."
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("fred api error 400"));
}

#[test]
fn transform_query_normalizes_series_id_and_rejects_query_injection() {
    let query =
        FredHttpSeriesObservationsFetcher::transform_query(json!({ "series_id": "unrate" }))
            .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.series_id, "UNRATE");
    assert!(
        FredHttpSeriesObservationsFetcher::transform_query(
            json!({ "series_id": "GDP&file_type=xml" })
        )
        .is_err()
    );
}

#[tokio::test]
async fn live_fred_returns_recent_observations_when_env_vars_set() {
    if std::env::var("TDW_FRED_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FRED_LIVE != 1; skipping live FRED integration test");
        return;
    }

    let fetcher = FredHttpSeriesObservationsFetcher::default().with_limit(5);
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must include at least one observation"
    );
    assert_eq!(rows[0].series_id, "GDP");
}
