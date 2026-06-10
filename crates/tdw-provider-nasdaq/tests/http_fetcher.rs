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
use tdw_core::Fetcher;
use tdw_provider_nasdaq::{
    NasdaqCalendarQuery, NasdaqDatasetQuery, NasdaqHttpCalendarFetcher, NasdaqHttpDatasetFetcher,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

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
    cassette_bytes!({
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
    let envelope = cassette_bytes!({
        "quandl_error": {
            "code": "QEAx01",
            "message": "You have submitted an incorrect Quandl code."
        }
    });
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
    let empty = cassette_bytes!({});
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

// ---------------------------------------------------------------------------
// Catalog-facing calendar fetcher -> CalendarEvent
// ---------------------------------------------------------------------------

#[test]
fn calendar_dividends_cassette_decodes_events() {
    let fetcher = NasdaqHttpCalendarFetcher::default();
    let query = NasdaqHttpCalendarFetcher::transform_query(json!({ "calendar": "dividends" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let raw = cassette_bytes!({
        "data": {
            "calendar": {
                "rows": [
                    {
                        "symbol": "AAPL",
                        "companyName": "Apple Inc.",
                        "dividend_Ex_Date": "2026-05-12",
                        "dividend_Rate": "0.25",
                        "payment_Date": "2026-05-15",
                        "record_Date": "2026-05-13"
                    }
                ]
            }
        }
    });
    let events = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(events.len(), 1, "events={events:#?}");
    assert_eq!(events[0].kind, "dividend");
    assert_eq!(events[0].symbol, "AAPL");
    assert_eq!(events[0].name.as_deref(), Some("Apple Inc."));
    assert_eq!(events[0].date.as_deref(), Some("2026-05-12"));
    assert_eq!(events[0].dividend, Some(0.25));
    assert_eq!(events[0].payment_date.as_deref(), Some("2026-05-15"));
}

#[test]
fn calendar_earnings_cassette_decodes_eps_estimate() {
    let fetcher = NasdaqHttpCalendarFetcher::default();
    let query = NasdaqHttpCalendarFetcher::transform_query(json!({ "calendar": "earnings" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let raw = cassette_bytes!({
        "data": {
            "rows": [
                {
                    "symbol": "MSFT",
                    "name": "Microsoft Corp.",
                    "date": "2026-04-28",
                    "epsForecast": "$2.93",
                    "fiscalQuarterEnding": "Mar/2026"
                }
            ]
        }
    });
    let events = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(events.len(), 1, "events={events:#?}");
    assert_eq!(events[0].kind, "earnings");
    assert_eq!(events[0].symbol, "MSFT");
    assert_eq!(events[0].eps_estimate, Some(2.93));
    assert_eq!(events[0].fiscal_period.as_deref(), Some("Mar/2026"));
}

#[test]
fn calendar_ipo_cassette_decodes_offer_fields() {
    let fetcher = NasdaqHttpCalendarFetcher::default();
    let query = NasdaqHttpCalendarFetcher::transform_query(json!({ "calendar": "ipo" }))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    let raw = cassette_bytes!({
        "data": {
            "upcoming": {
                "upcomingTable": {
                    "rows": [
                        {
                            "proposedTickerSymbol": "NEWCO",
                            "companyName": "NewCo Holdings",
                            "proposedSharePrice": "18.00",
                            "sharesOffered": "10,000,000",
                            "proposedExchange": "NASDAQ Global",
                            "expectedPriceDate": "2026-06-15"
                        }
                    ]
                }
            }
        }
    });
    let events = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(events.len(), 1, "events={events:#?}");
    assert_eq!(events[0].kind, "ipo");
    assert_eq!(events[0].symbol, "NEWCO");
    assert_eq!(events[0].price, Some(18.0));
    assert_eq!(events[0].shares, Some(10_000_000.0));
    assert_eq!(events[0].exchange.as_deref(), Some("NASDAQ Global"));
    assert_eq!(events[0].date.as_deref(), Some("2026-06-15"));
}

#[test]
fn calendar_transform_query_validates_kind_and_date() {
    assert!(
        NasdaqHttpCalendarFetcher::transform_query(json!({ "calendar": "splits" })).is_err(),
        "unknown calendar kind must be rejected"
    );
    assert!(
        NasdaqHttpCalendarFetcher::transform_query(json!({
            "calendar": "dividends",
            "date": "2026/05/12"
        }))
        .is_err(),
        "malformed date must be rejected"
    );
    let query = NasdaqHttpCalendarFetcher::transform_query(json!({
        "calendar": "dividends",
        "date": "2026-05-12"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.date.as_deref(), Some("2026-05-12"));
}

#[tokio::test]
async fn live_nasdaq_calendar_returns_events_when_env_var_set() {
    if std::env::var("TDW_NASDAQ_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_NASDAQ_LIVE != 1; skipping live NASDAQ calendar integration test");
        return;
    }
    let fetcher = NasdaqHttpCalendarFetcher::default();
    let query = NasdaqCalendarQuery::from_value(&json!({ "calendar": "dividends" }))
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let events = live_fetch_nonempty!(fetcher, query);
    assert!(
        !events.is_empty(),
        "live response must include calendar events"
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

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one row"
    );
    assert_eq!(rows[0].database, "FRED");
    assert_eq!(rows[0].dataset, "GDP");
}
