//! Tests for the real FMP HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes without any network access. The live test is additionally gated by
//! `TDW_FMP_LIVE=1` and requires `TDW_FMP_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_fmp::{
    FmpFundamentalsQuery, FmpHistoricalQuery, FmpHttpHistoricalFetcher, FmpHttpIncomeFetcher,
    FmpStatement,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn historical_cassette() -> Bytes {
    cassette_bytes!({
        "symbol": "AAPL",
        "historical": [
            {
                "date": "2024-01-02",
                "open": 185.6,
                "high": 186.1,
                "low": 184.4,
                "close": 185.2,
                "volume": 55000000.0
            },
            {
                "date": "2024-01-03",
                "open": 184.2,
                "high": 185.9,
                "low": 183.1,
                "close": 184.8,
                "volume": 48000000.0
            }
        ]
    })
}

fn income_cassette() -> Bytes {
    cassette_bytes!([
        {
            "date": "2024-09-28",
            "symbol": "AAPL",
            "revenue": 391035000000_i64,
            "grossProfit": 180683000000_i64,
            "netIncome": 93736000000_i64
        },
        {
            "date": "2023-09-30",
            "symbol": "AAPL",
            "revenue": 383285000000_i64,
            "grossProfit": 169148000000_i64,
            "netIncome": 96995000000_i64
        }
    ])
}

// ---------------------------------------------------------------------------
// Cassette tests (no network, always run when feature enabled)
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_fmp_historical_response() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, historical_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].venue, "fmp");
    assert_eq!(rows[0].ts, "2024-01-02");
    assert_eq!(rows[0].open, 185.6);
    assert_eq!(rows[0].close, 185.2);
    assert_eq!(rows[0].volume, 55_000_000.0);
    assert_eq!(rows[1].ts, "2024-01-03");
}

#[test]
fn cassette_parse_fmp_income_response() {
    let fetcher = FmpHttpIncomeFetcher::default();
    let query =
        FmpHttpIncomeFetcher::transform_query(json!({"symbol": "AAPL", "statement": "income"}))
            .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, income_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].date, "2024-09-28");
    assert_eq!(rows[0].revenue, 391_035_000_000);
    assert_eq!(rows[0].gross_profit, 180_683_000_000);
    assert_eq!(rows[0].net_income, 93_736_000_000);
    assert_eq!(rows[1].date, "2023-09-30");
}

#[test]
fn transform_query_normalises_symbol_and_rejects_path_injection() {
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(query.symbol, "MSFT");

    assert!(
        FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "MSFT/../../secret"})).is_err()
    );
    assert!(FmpHttpHistoricalFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn income_transform_query_defaults_statement_to_income_and_limit_to_five() {
    let query = FmpHttpIncomeFetcher::transform_query(json!({"symbol": "aapl"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));

    assert_eq!(query.symbol, "AAPL");
    assert_eq!(query.statement, FmpStatement::Income);
    assert_eq!(query.limit, 5);
}

#[test]
fn income_transform_query_rejects_unknown_statement() {
    assert!(
        FmpHttpIncomeFetcher::transform_query(json!({"symbol": "AAPL", "statement": "bogus"}))
            .is_err()
    );
}

#[test]
fn empty_historical_response_produces_empty_vec() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHistoricalQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!({"symbol": "AAPL", "historical": []});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert!(rows.is_empty());
}

#[test]
fn malformed_json_produces_provider_error() {
    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHistoricalQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");

    assert!(err.to_string().contains("fmp parse_json"));
}

// ---------------------------------------------------------------------------
// Live test (gated by TDW_FMP_LIVE=1 and TDW_FMP_API_KEY)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_fmp_historical_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP historical integration test");
        return;
    }

    let fetcher = FmpHttpHistoricalFetcher::default();
    let query = FmpHttpHistoricalFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live response must include at least one bar"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_fmp_income_returns_data_when_env_var_set() {
    if std::env::var("TDW_FMP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FMP_LIVE != 1; skipping live FMP income integration test");
        return;
    }

    let fetcher = FmpHttpIncomeFetcher::default();
    let query = FmpFundamentalsQuery::new("AAPL", FmpStatement::Income, 3)
        .unwrap_or_else(|e| panic!("query: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live income response must include at least one statement"
    );
}
