//! Tests for the Alpha Vantage HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes. The live test is additionally gated by `TDW_ALPHA_VANTAGE_LIVE=1`
//! and requires `TDW_ALPHA_VANTAGE_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_alpha_vantage::{AlphaVantageHttpFetcher, AlphaVantageQuery};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn time_series_daily_query() -> AlphaVantageQuery {
    AlphaVantageHttpFetcher::transform_query(json!({
        "symbol": "msft",
        "function": "TIME_SERIES_DAILY"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn global_quote_query() -> AlphaVantageQuery {
    AlphaVantageHttpFetcher::transform_query(json!({
        "symbol": "aapl",
        "function": "GLOBAL_QUOTE"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"))
}

fn time_series_daily_cassette() -> Bytes {
    cassette_bytes!({
        "Meta Data": {
            "1. Information": "Daily Prices (open, high, low, close) and Volumes",
            "2. Symbol": "MSFT",
            "3. Last Refreshed": "2024-01-03",
            "4. Output Size": "Compact",
            "5. Time Zone": "US/Eastern"
        },
        "Time Series (Daily)": {
            "2024-01-02": {
                "1. open": "373.86",
                "2. high": "375.90",
                "3. low": "366.77",
                "4. close": "370.87",
                "5. volume": "25258600"
            },
            "2024-01-03": {
                "1. open": "369.01",
                "2. high": "373.26",
                "3. low": "366.09",
                "4. close": "367.94",
                "5. volume": "23083500"
            }
        }
    })
}

fn global_quote_cassette() -> Bytes {
    cassette_bytes!({
        "Global Quote": {
            "01. symbol": "AAPL",
            "02. open": "185.00",
            "03. high": "186.10",
            "04. low": "184.40",
            "05. price": "185.20",
            "06. volume": "55000000",
            "07. latest trading day": "2024-01-03",
            "08. previous close": "184.90",
            "09. change": "0.30",
            "10. change percent": "0.1623%"
        }
    })
}

// ---------------------------------------------------------------------------
// Cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_time_series_daily_into_market_bars() {
    let fetcher = AlphaVantageHttpFetcher::default();
    let query = time_series_daily_query();
    let rows = fetcher
        .transform_data(&query, time_series_daily_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    // Rows are sorted ascending by ts.
    assert_eq!(rows[0].symbol, "MSFT");
    assert_eq!(rows[0].venue, "alpha_vantage");
    assert_eq!(rows[0].ts, "2024-01-02T00:00:00Z");
    assert_eq!(rows[0].open, 373.86);
    assert_eq!(rows[0].close, 370.87);
    assert_eq!(rows[0].volume, 25_258_600.0);
    assert_eq!(rows[1].ts, "2024-01-03T00:00:00Z");
}

#[test]
fn cassette_replay_decodes_global_quote_into_market_bar() {
    let fetcher = AlphaVantageHttpFetcher::default();
    let query = global_quote_query();
    let rows = fetcher
        .transform_data(&query, global_quote_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].close, 185.20);
    assert_eq!(rows[0].volume, 55_000_000.0);
}

#[test]
fn cassette_replay_surfaces_information_message_as_error() {
    let fetcher = AlphaVantageHttpFetcher::default();
    let query = time_series_daily_query();
    let rate_limit_body = cassette_bytes!({
        "Information": "Thank you for using Alpha Vantage! \
        Our standard API rate limit is 25 requests per day."
    });
    let err = fetcher
        .transform_data(&query, rate_limit_body)
        .expect_err("information envelope must be propagated as error");

    assert!(
        err.to_string().contains("alpha_vantage api message"),
        "unexpected error: {err}"
    );
}

#[test]
fn cassette_replay_surfaces_empty_global_quote_as_error() {
    let fetcher = AlphaVantageHttpFetcher::default();
    let query = global_quote_query();
    let empty_body = cassette_bytes!({ "Global Quote": {} });
    let err = fetcher
        .transform_data(&query, empty_body)
        .expect_err("empty global_quote must be propagated as error");

    assert!(
        err.to_string().contains("empty object"),
        "unexpected error: {err}"
    );
}

#[test]
fn transform_query_normalizes_symbol_and_rejects_injection() {
    let q = AlphaVantageHttpFetcher::transform_query(json!({
        "ticker": "msft",
        "function": "GLOBAL_QUOTE"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(q.symbol, "MSFT");

    assert!(
        AlphaVantageHttpFetcher::transform_query(json!({
            "symbol": "AAPL?apikey=injected",
            "function": "GLOBAL_QUOTE"
        }))
        .is_err()
    );

    assert!(
        AlphaVantageHttpFetcher::transform_query(json!({
            "symbol": "AAPL/../secret",
            "function": "GLOBAL_QUOTE"
        }))
        .is_err()
    );
}

#[test]
fn transform_query_rejects_unknown_function() {
    assert!(
        AlphaVantageHttpFetcher::transform_query(json!({
            "symbol": "AAPL",
            "function": "UNKNOWN_FUNCTION"
        }))
        .is_err()
    );
}

// ---------------------------------------------------------------------------
// Live integration test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_alpha_vantage_returns_data_when_env_var_set() {
    if std::env::var("TDW_ALPHA_VANTAGE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_ALPHA_VANTAGE_LIVE != 1; skipping live Alpha Vantage integration test");
        return;
    }

    let fetcher = AlphaVantageHttpFetcher::default();
    let query = global_quote_query();
    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}
