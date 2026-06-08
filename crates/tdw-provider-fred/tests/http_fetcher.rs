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
use tdw_core::Fetcher;
use tdw_provider_fred::{
    FredHttpMacroSeriesFetcher, FredHttpRateObservationFetcher, FredHttpSeriesObservationsFetcher,
    FredSeriesObservationsQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

fn sample_query() -> FredSeriesObservationsQuery {
    FredHttpSeriesObservationsFetcher::transform_query(json!({ "series_id": "gdp" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    cassette_bytes!({
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
    let envelope = cassette_bytes!({
        "error_code": 400,
        "error_message": "Bad Request. Variable api_key is required."
    });
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
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one observation"
    );
    assert_eq!(rows[0].series_id, "GDP");
}

// ---------------------------------------------------------------------------
// Macro-series fetcher (economy/* cluster -> MacroSeries)
// ---------------------------------------------------------------------------

fn cpi_cassette() -> Bytes {
    cassette_bytes!({
        "observations": [
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-01-01", "value": "308.417" },
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-02-01", "value": "." },
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-03-01", "value": "312.230" }
        ]
    })
}

#[test]
fn macro_series_fetcher_normalizes_cpi_to_macro_series() {
    let fetcher = FredHttpMacroSeriesFetcher::default();
    let query = FredHttpMacroSeriesFetcher::transform_query(json!({ "command": "economy/cpi" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, cpi_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "missing values skipped; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "CPIAUCSL");
    assert_eq!(rows[0].date, "2024-01-01");
    assert_eq!(rows[0].value, Some(308.417));
    assert_eq!(rows[0].frequency.as_deref(), Some("monthly"));
    assert_eq!(rows[0].unit.as_deref(), Some("index"));
    assert!(
        rows[0]
            .title
            .as_deref()
            .unwrap_or_default()
            .contains("Consumer Price Index")
    );
    assert_eq!(rows[1].date, "2024-03-01");
}

#[test]
fn macro_series_fetcher_rejects_unknown_command() {
    assert!(
        FredHttpMacroSeriesFetcher::transform_query(json!({ "command": "economy/not_real" }))
            .is_err()
    );
    assert!(FredHttpMacroSeriesFetcher::transform_query(json!({})).is_err());
}

// ---------------------------------------------------------------------------
// Rate-observation fetcher (fixedincome/* cluster -> RateObservation)
// ---------------------------------------------------------------------------

fn sofr_cassette() -> Bytes {
    cassette_bytes!({
        "observations": [
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-06-03", "value": "5.31" },
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-06-04", "value": "." },
            { "realtime_start": "2026-05-27", "realtime_end": "2026-05-27", "date": "2024-06-05", "value": "5.32" }
        ]
    })
}

#[test]
fn rate_observation_fetcher_normalizes_sofr_to_rate_observation() {
    let fetcher = FredHttpRateObservationFetcher::default();
    let query = FredHttpRateObservationFetcher::transform_query(
        json!({ "command": "fixedincome/rate/sofr" }),
    )
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, sofr_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 2, "missing values skipped; rows={rows:#?}");
    assert_eq!(rows[0].rate_id, "SOFR");
    assert_eq!(rows[0].date, "2024-06-03");
    assert_eq!(rows[0].value, Some(5.31));
    assert_eq!(rows[0].maturity.as_deref(), Some("overnight"));
    assert_eq!(rows[0].currency.as_deref(), Some("USD"));
    assert_eq!(rows[1].date, "2024-06-05");
}

#[test]
fn rate_observation_fetcher_tags_spread_tenor_and_eur_currency() {
    let fetcher = FredHttpRateObservationFetcher::default();

    let spread_query = FredHttpRateObservationFetcher::transform_query(json!({
        "command": "fixedincome/spreads/tcm/10y2y"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let spread_rows = fetcher
        .transform_data(&spread_query, sofr_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(spread_rows[0].rate_id, "T10Y2Y");
    assert_eq!(spread_rows[0].maturity.as_deref(), Some("10y-2y"));

    let estr_query = FredHttpRateObservationFetcher::transform_query(json!({
        "command": "fixedincome/rate/estr"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let estr_rows = fetcher
        .transform_data(&estr_query, sofr_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert_eq!(estr_rows[0].currency.as_deref(), Some("EUR"));
}

#[test]
fn catalog_fetchers_surface_fred_error_envelope() {
    let macro_fetcher = FredHttpMacroSeriesFetcher::default();
    let query =
        FredHttpMacroSeriesFetcher::transform_query(json!({ "command": "economy/gdp/nominal" }))
            .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let envelope = cassette_bytes!({
        "error_code": 400,
        "error_message": "Bad Request. Variable api_key is required."
    });
    let err = macro_fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");
    assert!(err.to_string().contains("fred api error 400"));
}

#[tokio::test]
async fn live_fred_macro_series_returns_data_when_env_vars_set() {
    if std::env::var("TDW_FRED_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FRED_LIVE != 1; skipping live FRED macro-series integration test");
        return;
    }

    let fetcher = FredHttpMacroSeriesFetcher::default();
    let query = FredHttpMacroSeriesFetcher::transform_query(json!({
        "command": "economy/unemployment",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live macro response must include observations"
    );
    assert_eq!(rows[0].series_id, "UNRATE");
}

#[tokio::test]
async fn live_fred_rate_observation_returns_data_when_env_vars_set() {
    if std::env::var("TDW_FRED_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FRED_LIVE != 1; skipping live FRED rate-observation integration test");
        return;
    }

    let fetcher = FredHttpRateObservationFetcher::default();
    let query = FredHttpRateObservationFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_rates/10y",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live rate response must include observations"
    );
    assert_eq!(rows[0].rate_id, "DGS10");
}
