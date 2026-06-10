//! Tests for the real SEC EDGAR HTTP fetchers.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded EDGAR
//! response shapes without network access.
//!
//! The live integration test is additionally gated by `TDW_SEC_LIVE=1` and
//! talks to the real `https://data.sec.gov` endpoint. No API key is required.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_sec::{
    SecCikMapHttpFetcher, SecEtfHoldingsHttpFetcher, SecFailsToDeliverHttpFetcher,
    SecFilingsHttpFetcher, SecFilingsQuery, SecForm13FHttpFetcher, SecHistoricalQuery,
    SecXbrlHttpFetcher,
};
use tdw_provider_testkit::{cassette_bytes, live_fetch_rows_expect};

// ── Cassette helpers ──────────────────────────────────────────────────────────

fn cassette_submissions() -> Bytes {
    cassette_bytes!({
        "cik": "0000320193",
        "name": "Apple Inc.",
        "filings": {
            "recent": {
                "accessionNumber": [
                    "0000320193-24-000123",
                    "0000320193-23-000106"
                ],
                "form": ["10-K", "10-Q"],
                "filingDate": ["2024-10-01", "2023-08-04"]
            }
        }
    })
}

fn cassette_xbrl() -> Bytes {
    cassette_bytes!({
        "cik": 320_193,
        "entityName": "Apple Inc.",
        "facts": {
            "us-gaap": {
                "RevenueFromContractWithCustomerExcludingAssessedTax": {
                    "label": "Revenue from Contract with Customer, Excluding Assessed Tax",
                    "units": {
                        "USD": [
                            {
                                "end": "2024-09-28",
                                "val": 391_035_000_000.0_f64,
                                "form": "10-K"
                            },
                            {
                                "end": "2023-09-30",
                                "val": 383_285_000_000.0_f64,
                                "form": "10-K"
                            },
                            {
                                "end": "2024-03-30",
                                "val": 90_753_000_000.0_f64,
                                "form": "10-Q"
                            }
                        ]
                    }
                }
            }
        }
    })
}

// ── Cassette tests (always run with --features http) ─────────────────────────

#[test]
fn cassette_parse_submissions_response() {
    let fetcher = SecFilingsHttpFetcher::default();
    let query = SecFilingsQuery::new("320193").expect("valid cik");

    let rows = fetcher
        .transform_data(&query, cassette_submissions())
        .expect("transform_data must succeed");

    assert_eq!(rows.len(), 2, "expected two filings rows, got {rows:#?}");

    assert_eq!(rows[0].cik, "320193");
    assert_eq!(rows[0].entity_name, "Apple Inc.");
    assert_eq!(rows[0].accession_number, "0000320193-24-000123");
    assert_eq!(rows[0].form, "10-K");
    assert_eq!(rows[0].filing_date, "2024-10-01");

    assert_eq!(rows[1].form, "10-Q");
    assert_eq!(rows[1].filing_date, "2023-08-04");
}

#[test]
fn cassette_parse_xbrl_response_filters_annual_only() {
    let fetcher = SecXbrlHttpFetcher::default();
    // Pass CIK as the "symbol" field — the XBRL fetcher expects a numeric CIK.
    let query = SecXbrlHttpFetcher::transform_query(json!({"symbol": "320193"}))
        .expect("transform_query must succeed");

    let rows = fetcher
        .transform_data(&query, cassette_xbrl())
        .expect("transform_data must succeed");

    // 10-Q fact should be excluded; only the two 10-K rows survive.
    assert_eq!(rows.len(), 2, "expected 2 annual rows, got {rows:#?}");
    assert_eq!(rows[0].ts, "2024-09-28T00:00:00Z");
    assert!((rows[0].close - 391_035_000_000.0_f64).abs() < 1.0);
    assert_eq!(rows[0].venue, "sec");
    assert_eq!(rows[0].source, "sec-xbrl");

    assert_eq!(rows[1].ts, "2023-09-30T00:00:00Z");
    assert!((rows[1].close - 383_285_000_000.0_f64).abs() < 1.0);
}

#[test]
fn cassette_filings_transform_query_roundtrip() {
    let q = SecFilingsHttpFetcher::transform_query(json!({"cik": "0000320193"}))
        .expect("transform_query must succeed");
    assert_eq!(q.padded_cik(), "0000320193");
}

// ── G003 keyless wave: cik_map / form_13f / fails_to_deliver / etf holdings ───

#[test]
fn cassette_parse_cik_map_response() {
    let fetcher = SecCikMapHttpFetcher::default();
    let query = SecCikMapHttpFetcher::transform_query(json!({})).expect("empty query");
    // company_tickers.json is an object keyed by integer-as-string indices.
    let raw = cassette_bytes!({
        "0": { "cik_str": 320_193, "ticker": "aapl", "title": "Apple Inc." },
        "1": { "cik_str": 789_019, "ticker": "MSFT", "title": "MICROSOFT CORP" },
        "2": { "cik_str": 0, "ticker": "", "title": "skipped: empty ticker" }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 2, "empty-ticker row skipped; rows={rows:#?}");
    let aapl = rows.iter().find(|r| r.symbol == "AAPL").expect("aapl row");
    assert_eq!(aapl.cik, "320193");
    assert_eq!(aapl.name.as_deref(), Some("Apple Inc."));
    let msft = rows.iter().find(|r| r.symbol == "MSFT").expect("msft row");
    assert_eq!(msft.cik, "789019");
}

#[test]
fn cassette_parse_form_13f_selects_only_13f_filings() {
    let fetcher = SecForm13FHttpFetcher::default();
    let query = SecFilingsQuery::new("1067983").expect("valid cik");
    let raw = cassette_bytes!({
        "cik": "0001067983",
        "name": "BERKSHIRE HATHAWAY INC",
        "filings": {
            "recent": {
                "accessionNumber": ["0000950123-24-000111", "0000950123-24-000222", "0000950123-24-000333"],
                "form": ["13F-HR", "10-K", "13F-HR/A"],
                "filingDate": ["2024-05-15", "2024-02-26", "2024-05-20"]
            }
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    // 10-K excluded; 13F-HR and 13F-HR/A retained.
    assert_eq!(rows.len(), 2, "only 13F filings; rows={rows:#?}");
    assert!(rows.iter().all(|r| r.kind == "form_13f"));
    assert_eq!(rows[0].symbol, "1067983");
    assert_eq!(rows[0].relationship.as_deref(), Some("13F-HR"));
    assert_eq!(rows[0].date.as_deref(), Some("2024-05-15"));
    assert_eq!(rows[1].relationship.as_deref(), Some("13F-HR/A"));
}

#[test]
fn cassette_parse_fails_to_deliver_filters_by_symbol() {
    let fetcher = SecFailsToDeliverHttpFetcher::default();
    let query = SecHistoricalQuery::new("AAPL").expect("valid symbol");
    // SEC pipe-delimited FTD format with a header row.
    let ftd = "\
SETTLEMENT DATE|CUSIP|SYMBOL|QUANTITY (FAILS)|DESCRIPTION|PRICE
20240603|037833100|AAPL|12345|APPLE INC|194.03
20240603|594918104|MSFT|6789|MICROSOFT CORP|414.67
20240604|037833100|AAPL|2222|APPLE INC|195.87";
    let raw = Bytes::from(ftd.as_bytes().to_vec());
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 2, "only AAPL rows; rows={rows:#?}");
    assert!(rows.iter().all(|r| r.symbol == "AAPL"));
    assert!(rows.iter().all(|r| r.kind == "fails_to_deliver"));
    assert_eq!(rows[0].date.as_deref(), Some("2024-06-03"));
    assert_eq!(rows[0].shares, Some(12345.0));
    assert_eq!(rows[0].value, Some(194.03));
    assert_eq!(rows[0].holder.as_deref(), Some("APPLE INC"));
    assert_eq!(rows[0].relationship.as_deref(), Some("037833100"));
}

#[test]
fn cassette_parse_nport_etf_holdings() {
    let fetcher = SecEtfHoldingsHttpFetcher::default();
    let query = SecFilingsQuery::new("884394").expect("valid cik");
    // Minimal NPORT-P primary_doc.xml shape: report date + two invstOrSec blocks.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<edgarSubmission>
  <formData>
    <genInfo><repPdDate>2024-03-31</repPdDate></genInfo>
    <invstOrSecs>
      <invstOrSec>
        <name>APPLE INC</name>
        <cusip>037833100</cusip>
        <identifiers><isin value="US0378331005"/></identifiers>
        <balance>1000000</balance>
        <valUSD>194030000.00</valUSD>
        <pctVal>6.85</pctVal>
      </invstOrSec>
      <invstOrSec>
        <name>MICROSOFT CORP</name>
        <cusip>594918104</cusip>
        <identifiers><isin value="US5949181045"/></identifiers>
        <balance>500000</balance>
        <valUSD>207335000.00</valUSD>
        <pctVal>7.32</pctVal>
      </invstOrSec>
    </invstOrSecs>
  </formData>
</edgarSubmission>"#;
    let raw = Bytes::from(xml.as_bytes().to_vec());
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 2, "two holdings; rows={rows:#?}");
    assert_eq!(rows[0].fund_symbol, "884394");
    assert_eq!(rows[0].report_date.as_deref(), Some("2024-03-31"));
    assert_eq!(rows[0].holding_name, "APPLE INC");
    assert_eq!(rows[0].cusip.as_deref(), Some("037833100"));
    assert_eq!(rows[0].isin.as_deref(), Some("US0378331005"));
    assert_eq!(rows[0].balance, Some(1_000_000.0));
    assert_eq!(rows[0].value_usd, Some(194_030_000.0));
    assert_eq!(rows[0].weight_pct, Some(6.85));
    assert_eq!(rows[1].holding_name, "MICROSOFT CORP");
}

// ── Live integration tests (gated by TDW_SEC_LIVE=1) ─────────────────────────

#[tokio::test]
async fn live_sec_filings_returns_data_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC EDGAR filings integration test");
        return;
    }

    // Apple Inc. CIK — a stable, well-known public filer.
    let fetcher = SecFilingsHttpFetcher::default();
    let query = SecFilingsQuery::new("320193").expect("valid cik");

    let rows = live_fetch_rows_expect!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live EDGAR submissions must contain at least one filing"
    );
    assert_eq!(rows[0].cik, "320193");
    // Apple's EDGAR name is stable.
    assert!(
        rows[0].entity_name.to_lowercase().contains("apple"),
        "entity_name should contain 'apple', got: {}",
        rows[0].entity_name
    );
}

#[tokio::test]
async fn live_sec_xbrl_returns_revenue_bars_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC EDGAR XBRL integration test");
        return;
    }

    let fetcher = SecXbrlHttpFetcher::default();
    let query = SecXbrlHttpFetcher::transform_query(json!({"symbol": "320193"}))
        .expect("transform_query must succeed");

    let rows = live_fetch_rows_expect!(fetcher, query);

    assert!(
        !rows.is_empty(),
        "live EDGAR XBRL must return at least one annual Revenue bar"
    );
    assert_eq!(rows[0].venue, "sec");
    assert_eq!(rows[0].source, "sec-xbrl");
}

#[tokio::test]
async fn live_sec_cik_map_returns_apple_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC cik_map integration test");
        return;
    }
    let fetcher = SecCikMapHttpFetcher::default();
    let query = SecCikMapHttpFetcher::transform_query(json!({})).expect("empty query");
    let rows = live_fetch_rows_expect!(fetcher, query);
    assert!(!rows.is_empty(), "live cik_map must return mappings");
    assert!(
        rows.iter().any(|r| r.symbol == "AAPL" && r.cik == "320193"),
        "live cik_map must contain AAPL -> 320193"
    );
}

#[tokio::test]
async fn live_sec_etf_holdings_returns_constituents_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC etf holdings integration test");
        return;
    }
    // SPDR S&P 500 ETF Trust filer CIK (a prolific N-PORT filer).
    let fetcher = SecEtfHoldingsHttpFetcher::default();
    let query = SecEtfHoldingsHttpFetcher::transform_query(json!({"cik": "884394"}))
        .expect("transform_query must succeed");
    let rows = live_fetch_rows_expect!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live N-PORT must return at least one holding"
    );
    assert!(rows.iter().all(|r| r.fund_symbol == "884394"));
}
