//! Tests for the real Deribit HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes inline. The live tests are additionally gated by `TDW_DERIBIT_LIVE=1`
//! and make real network calls to deribit.com.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_deribit::{
    DeribitFundingQuery, DeribitHttpFundingFetcher, DeribitHttpInstrumentsFetcher,
    DeribitHttpOrderBookFetcher, DeribitInstrumentsQuery, DeribitKind, DeribitOrderBookQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn instruments_cassette() -> Bytes {
    cassette_bytes!({
        "result": [
            {
                "instrument_name": "BTC-19JAN24-40000-C",
                "kind": "option",
                "strike": 40000.0,
                "expiration_timestamp": 1705651200000u64,
                "option_type": "call",
                "is_active": true
            },
            {
                "instrument_name": "BTC-19JAN24-40000-P",
                "kind": "option",
                "strike": 40000.0,
                "expiration_timestamp": 1705651200000u64,
                "option_type": "put",
                "is_active": true
            }
        ]
    })
}

fn order_book_cassette() -> Bytes {
    cassette_bytes!({
        "result": {
            "instrument_name": "BTC-19JAN24-40000-C",
            "bid_iv": 45.2,
            "ask_iv": 46.8,
            "mark_iv": 46.0,
            "underlying_price": 42500.0,
            "bids": [[5.1, 10.0]],
            "asks": [[5.3, 8.0]],
            "greeks": {
                "delta": 0.35,
                "gamma": 0.00002,
                "vega": 85.0,
                "theta": -120.0
            }
        }
    })
}

fn funding_cassette() -> Bytes {
    cassette_bytes!({
        "result": [
            {
                "timestamp": 1704153600000u64,
                "interest": 0.0001,
                "index_price": 42000.0
            },
            {
                "timestamp": 1704157200000u64,
                "interest": 0.00012,
                "index_price": 42050.0
            }
        ]
    })
}

fn sample_instruments_query() -> DeribitInstrumentsQuery {
    DeribitHttpInstrumentsFetcher::transform_query(json!({ "currency": "BTC", "kind": "option" }))
        .unwrap_or_else(|e| panic!("instruments query should transform: {e}"))
}

fn sample_order_book_query() -> DeribitOrderBookQuery {
    DeribitHttpOrderBookFetcher::transform_query(
        json!({ "instrument_name": "BTC-19JAN24-40000-C", "depth": 5 }),
    )
    .unwrap_or_else(|e| panic!("order_book query should transform: {e}"))
}

fn sample_funding_query() -> DeribitFundingQuery {
    DeribitHttpFundingFetcher::transform_query(json!({
        "instrument_name": "BTC-PERPETUAL",
        "start_ms": 1704153600000u64,
        "end_ms":   1704240000000u64,
        "count":    100
    }))
    .unwrap_or_else(|e| panic!("funding query should transform: {e}"))
}

// ---------------------------------------------------------------------------
// Cassette tests — instruments
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_instruments_list() {
    let fetcher = DeribitHttpInstrumentsFetcher::default();
    let query = sample_instruments_query();
    let instruments = fetcher
        .transform_data(&query, instruments_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(instruments.len(), 2, "instruments={instruments:#?}");
    assert_eq!(instruments[0].instrument_name, "BTC-19JAN24-40000-C");
    assert_eq!(instruments[0].kind, "option");
    assert_eq!(instruments[0].strike, Some(40_000.0));
    assert_eq!(instruments[0].expiration_timestamp, Some(1_705_651_200_000));
    assert_eq!(instruments[0].option_type.as_deref(), Some("call"));
    assert!(instruments[0].is_active);
    assert_eq!(instruments[1].option_type.as_deref(), Some("put"));
}

#[test]
fn instruments_cassette_malformed_json_returns_error() {
    let fetcher = DeribitHttpInstrumentsFetcher::default();
    let query = sample_instruments_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must return an error");
    assert!(
        err.to_string().contains("deribit instruments parse_json"),
        "error={err}"
    );
}

// ---------------------------------------------------------------------------
// Cassette tests — order book
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_order_book() {
    let fetcher = DeribitHttpOrderBookFetcher::default();
    let query = sample_order_book_query();
    let rows = fetcher
        .transform_data(&query, order_book_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    let book = &rows[0];
    assert_eq!(book.instrument_name, "BTC-19JAN24-40000-C");
    assert_eq!(book.bid_iv, Some(45.2));
    assert_eq!(book.ask_iv, Some(46.8));
    assert_eq!(book.mark_iv, Some(46.0));
    assert_eq!(book.underlying_price, Some(42_500.0));
    assert_eq!(book.bids.len(), 1);
    assert_eq!(book.asks.len(), 1);
    let greeks = book.greeks.as_ref().expect("greeks must be present");
    assert_eq!(greeks.delta, 0.35);
    assert_eq!(greeks.vega, 85.0);
    assert_eq!(greeks.theta, -120.0);
}

#[test]
fn order_book_cassette_malformed_json_returns_error() {
    let fetcher = DeribitHttpOrderBookFetcher::default();
    let query = sample_order_book_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must return an error");
    assert!(
        err.to_string().contains("deribit order_book parse_json"),
        "error={err}"
    );
}

// ---------------------------------------------------------------------------
// Cassette tests — funding rate history
// ---------------------------------------------------------------------------

#[test]
fn cassette_replay_decodes_funding_records() {
    let fetcher = DeribitHttpFundingFetcher::default();
    let query = sample_funding_query();
    let records = fetcher
        .transform_data(&query, funding_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(records.len(), 2, "records={records:#?}");
    assert_eq!(records[0].timestamp, 1_704_153_600_000);
    assert_eq!(records[0].interest, 0.0001);
    assert_eq!(records[0].index_price, 42_000.0);
    assert_eq!(records[1].timestamp, 1_704_157_200_000);
    assert!(records[1].interest > records[0].interest);
}

#[test]
fn funding_cassette_malformed_json_returns_error() {
    let fetcher = DeribitHttpFundingFetcher::default();
    let query = sample_funding_query();
    let err = fetcher
        .transform_data(&query, Bytes::from(b"not json".as_slice()))
        .expect_err("malformed JSON must return an error");
    assert!(
        err.to_string().contains("deribit funding parse_json"),
        "error={err}"
    );
}

// ---------------------------------------------------------------------------
// transform_query tests
// ---------------------------------------------------------------------------

#[test]
fn transform_query_instruments_normalises_currency() {
    let query = DeribitHttpInstrumentsFetcher::transform_query(
        json!({ "currency": "eth", "kind": "future" }),
    )
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.currency, "ETH");
    assert_eq!(query.kind, DeribitKind::Future);
}

#[test]
fn transform_query_instruments_defaults_kind_to_option() {
    let query = DeribitHttpInstrumentsFetcher::transform_query(json!({ "currency": "BTC" }))
        .unwrap_or_else(|e| panic!("query should transform without kind: {e}"));
    assert_eq!(query.kind, DeribitKind::Option);
}

#[test]
fn transform_query_instruments_rejects_invalid_currency() {
    assert!(
        DeribitHttpInstrumentsFetcher::transform_query(
            json!({ "currency": "BTC1", "kind": "option" })
        )
        .is_err(),
        "numeric characters in currency must be rejected"
    );
}

#[test]
fn transform_query_order_book_normalises_instrument_name() {
    let query = DeribitHttpOrderBookFetcher::transform_query(
        json!({ "instrument_name": "btc-perpetual", "depth": 10 }),
    )
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.instrument_name, "BTC-PERPETUAL");
    assert_eq!(query.depth, 10);
}

#[test]
fn transform_query_order_book_defaults_depth_to_5() {
    let query =
        DeribitHttpOrderBookFetcher::transform_query(json!({ "instrument_name": "BTC-PERPETUAL" }))
            .unwrap_or_else(|e| panic!("query should transform without depth: {e}"));
    assert_eq!(query.depth, 5);
}

#[test]
fn transform_query_order_book_rejects_injection() {
    assert!(
        DeribitHttpOrderBookFetcher::transform_query(
            json!({ "instrument_name": "BTC-PERP&depth=99", "depth": 5 })
        )
        .is_err(),
        "query injection must be rejected"
    );
}

#[test]
fn transform_query_funding_builds_correct_query() {
    let query = DeribitHttpFundingFetcher::transform_query(json!({
        "instrument_name": "ETH-PERPETUAL",
        "start_ms": 1704153600000u64,
        "end_ms":   1704240000000u64,
        "count":    50
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.instrument_name, "ETH-PERPETUAL");
    assert_eq!(query.count, 50);
}

#[test]
fn transform_query_funding_rejects_inverted_time_range() {
    assert!(
        DeribitHttpFundingFetcher::transform_query(json!({
            "instrument_name": "BTC-PERPETUAL",
            "start_ms": 2_000_000u64,
            "end_ms":   1_000_000u64,
            "count": 100
        }))
        .is_err(),
        "inverted time range must be rejected"
    );
}

// ---------------------------------------------------------------------------
// Live tests — gated by TDW_DERIBIT_LIVE=1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_deribit_instruments_returns_list_when_env_var_set() {
    if std::env::var("TDW_DERIBIT_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_DERIBIT_LIVE != 1; skipping live Deribit instruments integration test");
        return;
    }

    let fetcher = DeribitHttpInstrumentsFetcher::default();
    let query = DeribitInstrumentsQuery::new("BTC", DeribitKind::Option)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let instruments = live_fetch_nonempty!(fetcher, query);

    assert!(
        !instruments.is_empty(),
        "live response must include at least one instrument"
    );
}

#[tokio::test]
async fn live_deribit_order_book_returns_data_when_env_var_set() {
    if std::env::var("TDW_DERIBIT_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_DERIBIT_LIVE != 1; skipping live Deribit order_book integration test");
        return;
    }

    let fetcher = DeribitHttpOrderBookFetcher::default();
    let query = DeribitOrderBookQuery::new("BTC-PERPETUAL", 5)
        .unwrap_or_else(|e| panic!("query should build: {e}"));
    let rows = live_fetch_nonempty!(fetcher, query);

    assert_eq!(
        rows.len(),
        1,
        "live response must include one order book row"
    );
    assert_eq!(rows[0].instrument_name, "BTC-PERPETUAL");
}

#[tokio::test]
async fn live_deribit_funding_returns_records_when_env_var_set() {
    if std::env::var("TDW_DERIBIT_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_DERIBIT_LIVE != 1; skipping live Deribit funding integration test");
        return;
    }

    let fetcher = DeribitHttpFundingFetcher::default();
    let query =
        DeribitFundingQuery::new("BTC-PERPETUAL", 1_704_153_600_000, 1_704_240_000_000, 100)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
    let records = live_fetch_nonempty!(fetcher, query);

    assert!(
        !records.is_empty(),
        "live response must include funding records"
    );
}
