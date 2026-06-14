//! Tests for the real `Intrinio` v2 HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded `Intrinio`
//! response shapes without network access. The live tests are additionally gated
//! by `TDW_INTRINIO_LIVE=1` AND require the PAID `INTRINIO_API_KEY`; they skip
//! cleanly when either is absent, so unattended CI stays offline and key-free.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_intrinio::{
    IntrinioHttpForwardPeFetcher, IntrinioHttpHistoricalAttributesFetcher,
    IntrinioHttpLatestAttributesFetcher, IntrinioHttpOptionsSnapshotsFetcher,
    IntrinioHttpOptionsUnusualFetcher, IntrinioHttpReportedFinancialsFetcher,
    IntrinioHttpSearchAttributesFetcher,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

/// `TDW_INTRINIO_LIVE=1` AND a non-empty `INTRINIO_API_KEY` together gate the
/// live tests; absence of either skips cleanly.
fn live_enabled() -> bool {
    std::env::var("TDW_INTRINIO_LIVE").ok().as_deref() == Some("1")
        && std::env::var("INTRINIO_API_KEY")
            .ok()
            .is_some_and(|key| !key.trim().is_empty())
}

// ── historical_attributes ────────────────────────────────────────────────────

fn historical_attributes_cassette() -> Bytes {
    cassette_bytes!({
        "historical_data": [
            { "date": "2024-09-28T00:00:00.000Z", "value": 3400000000000.0 },
            { "date": "2024-06-29", "value": "3200000000000" },
            { "date": "", "value": null }
        ]
    })
}

#[test]
fn historical_attributes_cassette_normalizes_dates_and_values() {
    let fetcher = IntrinioHttpHistoricalAttributesFetcher::default();
    let query = IntrinioHttpHistoricalAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/historical_attributes",
        "symbol": "AAPL",
        "tag": "marketcap"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, historical_attributes_cassette())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(
        rows.len(),
        3,
        "all rows kept (blank date allowed); {rows:#?}"
    );
    assert_eq!(rows[0].symbol.as_deref(), Some("AAPL"));
    assert_eq!(rows[0].tag, "marketcap");
    assert_eq!(rows[0].date.as_deref(), Some("2024-09-28"));
    assert_eq!(rows[0].value, Some(3_400_000_000_000.0));
    // A numeric string coerces to f64.
    assert_eq!(rows[1].value, Some(3_200_000_000_000.0));
}

// ── latest_attributes ────────────────────────────────────────────────────────

#[test]
fn latest_attributes_cassette_returns_single_row() {
    let fetcher = IntrinioHttpLatestAttributesFetcher::default();
    let query = IntrinioHttpLatestAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/latest_attributes",
        "identifier": "AAPL",
        "tag": "marketcap"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, cassette_bytes!({ "value": 3400000000000.0 }))
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol.as_deref(), Some("AAPL"));
    assert_eq!(rows[0].tag, "marketcap");
    assert_eq!(rows[0].value, Some(3_400_000_000_000.0));
    assert!(rows[0].text_value.is_none());
}

#[test]
fn latest_attributes_textual_value_lands_in_text_field() {
    let fetcher = IntrinioHttpLatestAttributesFetcher::default();
    let query = IntrinioHttpLatestAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/latest_attributes",
        "identifier": "AAPL",
        "tag": "ceo"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let rows = fetcher
        .transform_data(&query, cassette_bytes!({ "value": "Tim Cook" }))
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert!(rows[0].value.is_none());
    assert_eq!(rows[0].text_value.as_deref(), Some("Tim Cook"));
}

// ── search_attributes ────────────────────────────────────────────────────────

#[test]
fn search_attributes_cassette_maps_tag_dictionary() {
    let fetcher = IntrinioHttpSearchAttributesFetcher::default();
    let query = IntrinioHttpSearchAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/search_attributes",
        "query": "marketcap"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "tags": [
            { "tag": "marketcap", "name": "Market Capitalization", "type": "usd" },
            { "tag": "", "name": "skip me" }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "blank-tag row skipped; {rows:#?}");
    assert!(rows[0].symbol.is_none(), "search rows carry no symbol");
    assert_eq!(rows[0].tag, "marketcap");
    assert_eq!(rows[0].name.as_deref(), Some("Market Capitalization"));
    assert_eq!(rows[0].unit.as_deref(), Some("usd"));
}

// ── reported_financials ──────────────────────────────────────────────────────

#[test]
fn reported_financials_cassette_collapses_line_items() {
    let fetcher = IntrinioHttpReportedFinancialsFetcher::default();
    let query = IntrinioHttpReportedFinancialsFetcher::transform_query(json!({
        "command": "equity/fundamental/reported_financials",
        "identifier": "fun_abc123"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "reported_financials": [
            { "xbrl_tag": "Revenues", "value": 391035000000.0 },
            { "reported_tag": { "tag": "NetIncomeLoss" }, "value": 93736000000.0 },
            { "xbrl_tag": "Skipped", "value": null }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "fun_abc123");
    assert_eq!(rows[0].line_items.get("Revenues"), Some(&391_035_000_000.0));
    assert_eq!(
        rows[0].line_items.get("NetIncomeLoss"),
        Some(&93_736_000_000.0)
    );
    assert!(!rows[0].line_items.contains_key("Skipped"));
}

#[test]
fn reported_financials_empty_payload_yields_no_rows() {
    let fetcher = IntrinioHttpReportedFinancialsFetcher::default();
    let query = IntrinioHttpReportedFinancialsFetcher::transform_query(json!({
        "command": "equity/fundamental/reported_financials",
        "identifier": "fun_empty"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = fetcher
        .transform_data(&query, cassette_bytes!({ "reported_financials": [] }))
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));
    assert!(rows.is_empty());
}

// ── forward_pe estimates ─────────────────────────────────────────────────────

#[test]
fn forward_pe_cassette_tags_kind_and_reads_estimates() {
    let fetcher = IntrinioHttpForwardPeFetcher::default();
    let query = IntrinioHttpForwardPeFetcher::transform_query(json!({
        "command": "equity/estimates/forward_pe",
        "identifier": "AAPL"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "forward_pe_estimates": [
            { "fiscal_year": "2026", "mean": 28.4, "low": 24.0, "high": 33.0, "count": 12 }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].kind, "forward_pe");
    assert_eq!(rows[0].fiscal_period.as_deref(), Some("2026"));
    assert_eq!(rows[0].value, Some(28.4));
    assert_eq!(rows[0].mean, Some(28.4));
    assert_eq!(rows[0].number_of_analysts, Some(12));
}

// ── options/unusual ──────────────────────────────────────────────────────────

#[test]
fn options_unusual_cassette_maps_contract_metadata() {
    let fetcher = IntrinioHttpOptionsUnusualFetcher::default();
    let query = IntrinioHttpOptionsUnusualFetcher::transform_query(json!({
        "command": "derivatives/options/unusual",
        "symbol": "AAPL"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "trades": [
            {
                "code": "AAPL_241220C00200000",
                "expiration_date": "2024-12-20",
                "strike": 200.0,
                "type": "call",
                "last": 5.25,
                "volume": 1500,
                "open_interest": 4200
            },
            { "expiration_date": "2024-12-20", "strike": 200.0 }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(
        rows.len(),
        1,
        "row missing option_type is skipped; {rows:#?}"
    );
    assert_eq!(rows[0].underlying_symbol, "AAPL");
    assert_eq!(
        rows[0].contract_symbol.as_deref(),
        Some("AAPL_241220C00200000")
    );
    assert_eq!(rows[0].expiration, "2024-12-20");
    assert_eq!(rows[0].strike, 200.0);
    assert_eq!(rows[0].option_type, "call");
    assert_eq!(rows[0].last_price, Some(5.25));
    assert_eq!(rows[0].volume, Some(1500));
    assert_eq!(rows[0].open_interest, Some(4200));
}

// ── options/snapshots ────────────────────────────────────────────────────────

#[test]
fn options_snapshots_cassette_reads_nested_blocks() {
    let fetcher = IntrinioHttpOptionsSnapshotsFetcher::default();
    let query = IntrinioHttpOptionsSnapshotsFetcher::transform_query(json!({
        "command": "derivatives/options/snapshots",
        "symbol": "AAPL"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    let raw = cassette_bytes!({
        "contracts": [
            {
                "contract": {
                    "underlying": "AAPL",
                    "code": "AAPL_241220P00150000",
                    "expiration_date": "2024-12-20",
                    "strike": 150.0,
                    "type": "put"
                },
                "price": { "bid": 1.10, "ask": 1.20, "last": 1.15, "volume": 300, "open_interest": 900 },
                "greeks": { "implied_volatility": 0.31, "delta": -0.22, "gamma": 0.01 }
            }
        ]
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].underlying_symbol, "AAPL");
    assert_eq!(rows[0].option_type, "put");
    assert_eq!(rows[0].strike, 150.0);
    assert_eq!(rows[0].bid, Some(1.10));
    assert_eq!(rows[0].ask, Some(1.20));
    assert_eq!(rows[0].implied_volatility, Some(0.31));
    assert_eq!(rows[0].delta, Some(-0.22));
}

// ── shared: unknown-command + malformed JSON guards ──────────────────────────

#[test]
fn transform_query_rejects_unknown_command() {
    assert!(IntrinioHttpForwardPeFetcher::transform_query(json!({ "command": "bogus" })).is_err());
    assert!(IntrinioHttpForwardPeFetcher::transform_query(json!({})).is_err());
}

#[test]
fn transform_data_rejects_malformed_json() {
    let fetcher = IntrinioHttpHistoricalAttributesFetcher::default();
    let query = IntrinioHttpHistoricalAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/historical_attributes",
        "symbol": "AAPL",
        "tag": "marketcap"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let garbage = Bytes::from_static(b"{not valid json");
    let err = fetcher
        .transform_data(&query, garbage)
        .expect_err("malformed JSON must be propagated");
    assert!(err.to_string().contains("parse_json"), "got: {err}");
}

// ── live (gated by TDW_INTRINIO_LIVE=1 + INTRINIO_API_KEY) ────────────────────

#[tokio::test]
async fn live_intrinio_historical_attributes_returns_rows_when_env_set() {
    if !live_enabled() {
        eprintln!("TDW_INTRINIO_LIVE != 1 or INTRINIO_API_KEY unset; skipping live test");
        return;
    }
    let fetcher = IntrinioHttpHistoricalAttributesFetcher::default();
    let query = IntrinioHttpHistoricalAttributesFetcher::transform_query(json!({
        "command": "equity/fundamental/historical_attributes",
        "symbol": "AAPL",
        "tag": "marketcap",
        "limit": 5
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live historical attributes must include rows"
    );
}

#[tokio::test]
async fn live_intrinio_options_unusual_returns_rows_when_env_set() {
    if !live_enabled() {
        eprintln!("TDW_INTRINIO_LIVE != 1 or INTRINIO_API_KEY unset; skipping live test");
        return;
    }
    let fetcher = IntrinioHttpOptionsUnusualFetcher::default();
    let query = IntrinioHttpOptionsUnusualFetcher::transform_query(json!({
        "command": "derivatives/options/unusual",
        "symbol": "AAPL"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));
    let rows = live_fetch_nonempty!(fetcher, query);
    assert!(!rows.is_empty(), "live unusual options must include rows");
}
