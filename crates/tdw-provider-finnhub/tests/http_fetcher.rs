//! Tests for the real Finnhub HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded response
//! shapes without any network access. The live test is additionally gated by
//! `TDW_FINNHUB_LIVE=1` and requires `TDW_FINNHUB_API_KEY`.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_finnhub::{
    FinnhubCompanyNewsQuery, FinnhubHttpCompanyNewsFetcher, FinnhubHttpProfileFetcher,
    FinnhubHttpQuoteSnapshotFetcher, FinnhubHttpSymbolSearchFetcher, FinnhubProfileQuery,
    FinnhubSearchQuery,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_nonempty};

// ---------------------------------------------------------------------------
// Cassette helpers
// ---------------------------------------------------------------------------

fn profile_cassette() -> Bytes {
    cassette_bytes!({
        "ticker": "AAPL",
        "name": "Apple Inc",
        "currency": "USD",
        "exchange": "NASDAQ NMS - GLOBAL MARKET",
        "logo": "https://static2.finnhub.io/file/publicdatany/finnhubimage/stock_logo/AAPL.png",
        "marketCapitalization": 3_000_000.0_f64
    })
}

fn quote_cassette() -> Bytes {
    cassette_bytes!({
        "c": 189.30_f64,
        "d": 1.20_f64,
        "dp": 0.638_f64,
        "h": 190.0_f64,
        "l": 188.0_f64,
        "o": 188.5_f64,
        "pc": 188.10_f64,
        "t": 1_717_200_000_i64
    })
}

fn search_cassette() -> Bytes {
    cassette_bytes!({
        "count": 2,
        "result": [
            {
                "symbol": "AAPL",
                "displaySymbol": "AAPL",
                "description": "APPLE INC",
                "type": "Common Stock"
            },
            {
                "symbol": "AAPL.SW",
                "displaySymbol": "AAPL.SW",
                "description": "APPLE INC",
                "type": "Common Stock"
            }
        ]
    })
}

fn company_news_cassette() -> Bytes {
    cassette_bytes!([
        {
            "category": "company news",
            "datetime": 1_717_200_000_i64,
            "headline": "Apple unveils new product",
            "id": 123_456_i64,
            "image": "https://example.com/a.png",
            "related": "AAPL",
            "source": "Reuters",
            "summary": "Apple announced a new product today.",
            "url": "https://example.com/news/1"
        },
        {
            "category": "company news",
            "datetime": 1_717_286_400_i64,
            "headline": "Apple beats earnings",
            "id": 123_457_i64,
            "image": "https://example.com/b.png",
            "related": "AAPL",
            "source": "Bloomberg",
            "summary": "Apple reported strong quarterly results.",
            "url": "https://example.com/news/2"
        }
    ])
}

// ---------------------------------------------------------------------------
// Profile cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_profile_response() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, profile_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].ticker, "AAPL");
    assert_eq!(rows[0].name, "Apple Inc");
    assert_eq!(rows[0].currency, "USD");
    assert_eq!(rows[0].exchange, "NASDAQ NMS - GLOBAL MARKET");
    assert_eq!(rows[0].market_cap_millions, 3_000_000.0);
}

#[test]
fn profile_transform_query_normalises_symbol_and_rejects_invalid() {
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "aapl"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "AAPL");

    assert!(
        FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL/../../secret"})).is_err()
    );
    assert!(FinnhubHttpProfileFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn profile_empty_response_produces_empty_vec() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    // Finnhub returns an empty object `{}` when the symbol is not found.
    let raw = cassette_bytes!({});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert!(rows.is_empty());
}

#[test]
fn profile_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub profile parse_json"));
}

// ---------------------------------------------------------------------------
// Quote cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_quote_snapshot_response() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, quote_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].current_price, 189.30);
    assert_eq!(rows[0].change, 1.20);
    assert_eq!(rows[0].change_percent, 0.638);
    assert_eq!(rows[0].prev_close, 188.10);
    // Finnhub timestamp is seconds; fetcher multiplies by 1000 for ts_ms.
    assert_eq!(rows[0].ts_ms, 1_717_200_000_000);
}

#[test]
fn quote_snapshot_transform_query_normalises_symbol_and_rejects_invalid() {
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");

    assert!(
        FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "MSFT/../../secret"}))
            .is_err()
    );
    assert!(FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": ""})).is_err());
}

#[test]
fn quote_snapshot_missing_numerics_fall_back_to_zero() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    // Only timestamp provided — all other numeric fields should default to 0.0.
    let raw = cassette_bytes!({"t": 0_i64});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].current_price, 0.0);
    assert_eq!(rows[0].ts_ms, 0);
}

#[test]
fn quote_snapshot_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub quote parse_json"));
}

// ---------------------------------------------------------------------------
// Symbol-search cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_symbol_search_response() {
    let fetcher = FinnhubHttpSymbolSearchFetcher::default();
    let query = FinnhubHttpSymbolSearchFetcher::transform_query(json!({"query": "apple"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, search_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].symbol, "AAPL");
    assert_eq!(rows[0].display_symbol, "AAPL");
    assert_eq!(rows[0].description, "APPLE INC");
    assert_eq!(rows[0].kind, "Common Stock");
    assert_eq!(rows[1].symbol, "AAPL.SW");
}

#[test]
fn symbol_search_transform_query_accepts_free_text_and_rejects_blank() {
    let query = FinnhubHttpSymbolSearchFetcher::transform_query(json!({"query": "apple inc"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.query, "apple inc");

    // `q` alias also works.
    let aliased = FinnhubHttpSymbolSearchFetcher::transform_query(json!({"q": "msft"}))
        .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(aliased.query, "msft");

    assert!(FinnhubHttpSymbolSearchFetcher::transform_query(json!({"query": ""})).is_err());
    assert!(FinnhubHttpSymbolSearchFetcher::transform_query(json!({"query": "   "})).is_err());
}

#[test]
fn symbol_search_empty_result_produces_empty_vec() {
    let fetcher = FinnhubHttpSymbolSearchFetcher::default();
    let query = FinnhubSearchQuery::new("zzzz").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!({"count": 0, "result": []});

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert!(rows.is_empty());
}

#[test]
fn symbol_search_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpSymbolSearchFetcher::default();
    let query = FinnhubSearchQuery::new("apple").unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub search parse_json"));
}

// ---------------------------------------------------------------------------
// Company-news cassette tests
// ---------------------------------------------------------------------------

#[test]
fn cassette_parse_finnhub_company_news_response() {
    let fetcher = FinnhubHttpCompanyNewsFetcher::default();
    let query = FinnhubHttpCompanyNewsFetcher::transform_query(json!({
        "symbol": "AAPL",
        "from": "2024-06-01",
        "to": "2024-06-02"
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = fetcher
        .transform_data(&query, company_news_cassette())
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));

    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].id, 123_456);
    // Finnhub datetime is seconds; fetcher multiplies by 1000 for datetime_ms.
    assert_eq!(rows[0].datetime_ms, 1_717_200_000_000);
    assert_eq!(rows[0].headline, "Apple unveils new product");
    assert_eq!(rows[0].summary, "Apple announced a new product today.");
    assert_eq!(rows[0].source, "Reuters");
    assert_eq!(rows[0].url, "https://example.com/news/1");
    assert_eq!(rows[0].category, "company news");
    assert_eq!(rows[0].related, "AAPL");
    assert_eq!(rows[1].id, 123_457);
    assert_eq!(rows[1].source, "Bloomberg");
}

#[test]
fn company_news_transform_query_normalises_and_validates() {
    let query = FinnhubHttpCompanyNewsFetcher::transform_query(json!({
        "ticker": "msft",
        "from": "2024-01-01",
        "to": "2024-01-31"
    }))
    .unwrap_or_else(|e| panic!("query should transform: {e}"));
    assert_eq!(query.symbol, "MSFT");
    assert_eq!(query.from, "2024-01-01");
    assert_eq!(query.to, "2024-01-31");

    // Bad date shape is rejected.
    assert!(
        FinnhubHttpCompanyNewsFetcher::transform_query(json!({
            "symbol": "AAPL",
            "from": "2024/01/01",
            "to": "2024-01-31"
        }))
        .is_err()
    );
    // Invalid symbol is rejected.
    assert!(
        FinnhubHttpCompanyNewsFetcher::transform_query(json!({
            "symbol": "AAPL/../x",
            "from": "2024-01-01",
            "to": "2024-01-31"
        }))
        .is_err()
    );
    // Missing date param is rejected.
    assert!(
        FinnhubHttpCompanyNewsFetcher::transform_query(
            json!({"symbol": "AAPL", "from": "2024-01-01"})
        )
        .is_err()
    );
}

#[test]
fn company_news_empty_array_produces_empty_vec() {
    let fetcher = FinnhubHttpCompanyNewsFetcher::default();
    let query = FinnhubCompanyNewsQuery::new("AAPL", "2024-01-01", "2024-01-31")
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = cassette_bytes!([]);

    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("transform_data must succeed: {e}"));
    assert!(rows.is_empty());
}

#[test]
fn company_news_malformed_json_produces_provider_error() {
    let fetcher = FinnhubHttpCompanyNewsFetcher::default();
    let query = FinnhubCompanyNewsQuery::new("AAPL", "2024-01-01", "2024-01-31")
        .unwrap_or_else(|e| panic!("query: {e}"));
    let raw = Bytes::from(b"not valid json".to_vec());

    let err = fetcher
        .transform_data(&query, raw)
        .expect_err("malformed JSON must produce an error");
    assert!(err.to_string().contains("finnhub company-news parse_json"));
}

// ---------------------------------------------------------------------------
// Live tests (gated by TDW_FINNHUB_LIVE=1 and TDW_FINNHUB_API_KEY)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_finnhub_profile_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub profile integration test");
        return;
    }

    let fetcher = FinnhubHttpProfileFetcher::default();
    let query = FinnhubHttpProfileFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(!rows.is_empty(), "live profile response must include data");
    assert_eq!(rows[0].ticker, "AAPL");
}

#[tokio::test]
async fn live_finnhub_quote_snapshot_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub quote-snapshot integration test");
        return;
    }

    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"symbol": "AAPL"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    assert!(
        !rows.is_empty(),
        "live quote-snapshot response must include at least one entry"
    );
    assert_eq!(rows[0].symbol, "AAPL");
}

#[tokio::test]
async fn live_finnhub_quote_uses_ticker_param_alias() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub ticker-alias test");
        return;
    }

    let query = FinnhubHttpQuoteSnapshotFetcher::transform_query(json!({"ticker": "MSFT"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));
    assert_eq!(query.symbol, "MSFT");

    let fetcher = FinnhubHttpQuoteSnapshotFetcher::default();
    let rows = live_fetch_nonempty!(fetcher, query);
    assert_eq!(rows[0].symbol, "MSFT");
}

#[tokio::test]
async fn live_finnhub_symbol_search_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub symbol-search integration test");
        return;
    }

    let fetcher = FinnhubHttpSymbolSearchFetcher::default();
    let query = FinnhubHttpSymbolSearchFetcher::transform_query(json!({"query": "apple"}))
        .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let rows = live_fetch_nonempty!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live symbol-search response must include data"
    );
}

#[tokio::test]
async fn live_finnhub_company_news_returns_data_when_env_var_set() {
    if std::env::var("TDW_FINNHUB_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_FINNHUB_LIVE != 1; skipping live Finnhub company-news integration test");
        return;
    }

    let fetcher = FinnhubHttpCompanyNewsFetcher::default();
    let query = FinnhubHttpCompanyNewsFetcher::transform_query(json!({
        "symbol": "AAPL",
        "from": "2024-06-01",
        "to": "2024-06-07"
    }))
    .unwrap_or_else(|e| panic!("transform_query must succeed: {e}"));

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|e| panic!("live extract_data must succeed: {e}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|e| panic!("live transform_data must succeed: {e}"));

    // News volume varies; only assert the call succeeded and parsed cleanly.
    let _ = rows;
}
