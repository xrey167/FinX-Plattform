#![cfg(feature = "http")]
//! Tests for the real Trading Economics HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse a recorded API
//! response shape. The live test is additionally gated by
//! `TDW_TRADING_ECONOMICS_LIVE=1` and requires
//! `TDW_TRADING_ECONOMICS_API_KEY`.

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_trading_economics::{
    TradingEconomicsCalendarQuery, TradingEconomicsHttpCalendarFetcher,
    TradingEconomicsHttpIndicatorFetcher, TradingEconomicsIndicatorQuery,
};

// ---------------------------------------------------------------------------
// Calendar cassette helpers
// ---------------------------------------------------------------------------

fn calendar_query() -> TradingEconomicsCalendarQuery {
    TradingEconomicsHttpCalendarFetcher::transform_query(json!({ "importance_min": 2 }))
        .unwrap_or_else(|e| panic!("calendar query should transform: {e}"))
}

fn calendar_cassette() -> Bytes {
    Bytes::from(
        json!([
            {
                "Date": "2024-01-05T13:30:00",
                "Country": "United States",
                "Category": "Non Farm Payrolls",
                "Event": "Non Farm Payrolls",
                "Actual": "",
                "Previous": "199000",
                "Forecast": "170000",
                "TEForecast": "175000",
                "Importance": 3
            },
            {
                "Date": "2024-01-05T14:00:00",
                "Country": "United States",
                "Category": "Unemployment Rate",
                "Event": "Unemployment Rate",
                "Actual": "",
                "Previous": "3.7",
                "Forecast": "3.8",
                "TEForecast": "3.7",
                "Importance": 1
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Indicator cassette helpers
// ---------------------------------------------------------------------------

fn indicator_query() -> TradingEconomicsIndicatorQuery {
    TradingEconomicsHttpIndicatorFetcher::transform_query(
        json!({ "country": "united+states", "indicator": "gdp-growth-rate" }),
    )
    .unwrap_or_else(|e| panic!("indicator query should transform: {e}"))
}

fn indicator_cassette() -> Bytes {
    Bytes::from(
        json!([
            {
                "Country": "United States",
                "Category": "GDP Growth Rate",
                "DateTime": "2023-10-01",
                "Value": "1.2",
                "Frequency": "Quarter",
                "HistoricalDataSymbol": "UNITEDSTAGSQDP"
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Calendar tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_calendar_filters_by_importance_min() {
    let fetcher = TradingEconomicsHttpCalendarFetcher::default();
    let query = calendar_query(); // importance_min = 2
    let rows = fetcher
        .transform_data(&query, calendar_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    // The cassette has one importance=3 and one importance=1 event.
    // With importance_min=2 only the importance=3 row survives.
    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].country, "United States");
    assert_eq!(rows[0].category, "Non Farm Payrolls");
    assert_eq!(rows[0].importance, 3);
    assert_eq!(rows[0].previous, "199000");
    assert_eq!(rows[0].te_forecast, "175000");
}

#[test]
fn cassette_calendar_returns_all_when_importance_min_is_one() {
    let fetcher = TradingEconomicsHttpCalendarFetcher::default();
    let query =
        TradingEconomicsHttpCalendarFetcher::transform_query(json!({ "importance_min": 1 }))
            .unwrap_or_else(|e| panic!("transform: {e}"));
    let rows = fetcher
        .transform_data(&query, calendar_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
}

#[test]
fn transform_query_calendar_rejects_invalid_importance() {
    assert!(
        TradingEconomicsHttpCalendarFetcher::transform_query(json!({ "importance_min": 0 }))
            .is_err()
    );
    assert!(
        TradingEconomicsHttpCalendarFetcher::transform_query(json!({ "importance_min": 4 }))
            .is_err()
    );
}

#[test]
fn transform_query_calendar_defaults_to_importance_one_when_field_absent() {
    let query = TradingEconomicsHttpCalendarFetcher::transform_query(json!({}))
        .unwrap_or_else(|e| panic!("should default: {e}"));
    assert_eq!(query.importance_min, 1);
}

// ---------------------------------------------------------------------------
// Indicator tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_indicator_decodes_row() {
    let fetcher = TradingEconomicsHttpIndicatorFetcher::default();
    let query = indicator_query();
    let rows = fetcher
        .transform_data(&query, indicator_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].country, "United States");
    assert_eq!(rows[0].category, "GDP Growth Rate");
    assert_eq!(rows[0].date_time, "2023-10-01");
    assert_eq!(rows[0].value, "1.2");
    assert_eq!(rows[0].frequency, "Quarter");
    assert_eq!(rows[0].historical_data_symbol, "UNITEDSTAGSQDP");
}

#[test]
fn transform_query_indicator_rejects_missing_fields() {
    assert!(
        TradingEconomicsHttpIndicatorFetcher::transform_query(json!({ "country": "us" })).is_err()
    );
    assert!(
        TradingEconomicsHttpIndicatorFetcher::transform_query(json!({ "indicator": "gdp" }))
            .is_err()
    );
}

#[test]
fn transform_query_indicator_rejects_injection_characters() {
    assert!(
        TradingEconomicsHttpIndicatorFetcher::transform_query(
            json!({ "country": "us&hack=1", "indicator": "gdp" })
        )
        .is_err()
    );
    assert!(
        TradingEconomicsHttpIndicatorFetcher::transform_query(
            json!({ "country": "us", "indicator": "gdp?x=1" })
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// Live integration tests (gated by env var)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_calendar_returns_events_when_env_vars_set() {
    if std::env::var("TDW_TRADING_ECONOMICS_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_TRADING_ECONOMICS_LIVE != 1; skipping live Trading Economics calendar test");
        return;
    }

    let fetcher = TradingEconomicsHttpCalendarFetcher::default();
    let query = calendar_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live calendar response must include at least one event"
    );
}

#[tokio::test]
async fn live_indicator_returns_rows_when_env_vars_set() {
    if std::env::var("TDW_TRADING_ECONOMICS_LIVE").ok().as_deref() != Some("1") {
        eprintln!(
            "TDW_TRADING_ECONOMICS_LIVE != 1; skipping live Trading Economics indicator test"
        );
        return;
    }

    let fetcher = TradingEconomicsHttpIndicatorFetcher::default();
    let query = indicator_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live indicator response must include at least one row"
    );
}
