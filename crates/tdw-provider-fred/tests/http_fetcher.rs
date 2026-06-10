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
    FredHttpSeriesSearchFetcher, FredHttpYieldCurveFetcher, FredSeriesObservationsQuery,
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

// ---------------------------------------------------------------------------
// Yield-curve fetcher (fixedincome/government/yield_curve -> YieldCurvePoint)
// ---------------------------------------------------------------------------

/// The combined-legs intermediate shape `FredHttpYieldCurveFetcher::extract_data`
/// produces and `transform_data` consumes: one entry per Treasury tenor with its
/// decoded (date, value) observations.
fn yield_curve_cassette() -> Bytes {
    cassette_bytes!([
        {
            "maturity": "3m",
            "series_id": "DGS3MO",
            "currency": "USD",
            "observations": [
                { "date": "2024-06-03", "value": 5.40 },
                { "date": "2024-06-04", "value": 5.41 }
            ]
        },
        {
            "maturity": "10y",
            "series_id": "DGS10",
            "currency": "USD",
            "observations": [
                { "date": "2024-06-03", "value": 4.42 }
            ]
        }
    ])
}

#[test]
fn yield_curve_fetcher_merges_legs_into_points() {
    let fetcher = FredHttpYieldCurveFetcher::default();
    let query = FredHttpYieldCurveFetcher::transform_query(json!({}))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, yield_curve_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(
        rows.len(),
        3,
        "two 3m points + one 10y point; rows={rows:#?}"
    );
    assert!(rows.iter().all(|p| p.curve_id == "us_treasury"));
    assert!(rows.iter().all(|p| p.currency.as_deref() == Some("USD")));
    let three_month: Vec<_> = rows.iter().filter(|p| p.maturity == "3m").collect();
    assert_eq!(three_month.len(), 2);
    assert_eq!(three_month[0].date, "2024-06-03");
    assert_eq!(three_month[0].value, Some(5.40));
    let ten_year: Vec<_> = rows.iter().filter(|p| p.maturity == "10y").collect();
    assert_eq!(ten_year.len(), 1);
    assert_eq!(ten_year[0].value, Some(4.42));
}

#[tokio::test]
async fn live_fred_yield_curve_returns_points_when_env_vars_set() {
    if std::env::var("TDW_FRED_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FRED_LIVE != 1; skipping live FRED yield-curve integration test");
        return;
    }

    let fetcher = FredHttpYieldCurveFetcher::default();
    let query = FredHttpYieldCurveFetcher::transform_query(json!({ "limit": 5 }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(!rows.is_empty(), "live yield curve must include points");
    assert!(rows.iter().all(|p| p.curve_id == "us_treasury"));
}

// ---------------------------------------------------------------------------
// Series-search fetcher (economy/fred_search -> SeriesSearchResult)
// ---------------------------------------------------------------------------

fn search_cassette() -> Bytes {
    cassette_bytes!({
        "realtime_start": "2026-05-27",
        "realtime_end": "2026-05-27",
        "order_by": "search_rank",
        "sort_order": "desc",
        "count": 2,
        "offset": 0,
        "limit": 1000,
        "seriess": [
            {
                "id": "CPIAUCSL",
                "title": "Consumer Price Index for All Urban Consumers: All Items",
                "frequency": "Monthly",
                "units": "Index 1982-1984=100",
                "popularity": 95
            },
            {
                "id": "",
                "title": "skipped: empty id"
            }
        ]
    })
}

#[test]
fn series_search_fetcher_normalizes_results_and_skips_empty_ids() {
    let fetcher = FredHttpSeriesSearchFetcher::default();
    let query = FredHttpSeriesSearchFetcher::transform_query(json!({ "search_text": "cpi" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, search_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "empty-id row skipped; rows={rows:#?}");
    assert_eq!(rows[0].series_id, "CPIAUCSL");
    assert_eq!(rows[0].frequency.as_deref(), Some("Monthly"));
    assert_eq!(rows[0].popularity, Some(95));
    assert!(
        rows[0]
            .title
            .as_deref()
            .unwrap_or_default()
            .contains("Consumer Price Index")
    );
}

#[test]
fn series_search_fetcher_rejects_blank_query_and_surfaces_error_envelope() {
    assert!(FredHttpSeriesSearchFetcher::transform_query(json!({ "search_text": "   " })).is_err());
    assert!(FredHttpSeriesSearchFetcher::transform_query(json!({})).is_err());

    let fetcher = FredHttpSeriesSearchFetcher::default();
    let query = FredHttpSeriesSearchFetcher::transform_query(json!({ "search_text": "cpi" }))
        .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let envelope = cassette_bytes!({
        "error_code": 400,
        "error_message": "Bad Request."
    });
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");
    assert!(err.to_string().contains("fred api error 400"));
}

#[tokio::test]
async fn live_fred_search_returns_results_when_env_vars_set() {
    if std::env::var("TDW_FRED_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FRED_LIVE != 1; skipping live FRED search integration test");
        return;
    }

    let fetcher = FredHttpSeriesSearchFetcher::default();
    let query = FredHttpSeriesSearchFetcher::transform_query(json!({
        "search_text": "unemployment rate",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(!rows.is_empty(), "live search must include results");
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
