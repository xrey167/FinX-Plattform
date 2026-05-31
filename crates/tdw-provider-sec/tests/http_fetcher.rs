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
use tdw_core::{Credentials, Fetcher};
use tdw_provider_sec::{SecFilingsHttpFetcher, SecFilingsQuery, SecXbrlHttpFetcher};

// ── Cassette helpers ──────────────────────────────────────────────────────────

fn cassette_submissions() -> Bytes {
    Bytes::from(
        json!({
            "cik": "320193",
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
        .to_string()
        .into_bytes(),
    )
}

fn cassette_xbrl() -> Bytes {
    Bytes::from(
        json!({
            "cik": 320193,
            "entityName": "Apple Inc.",
            "facts": {
                "us-gaap": {
                    "Revenue": {
                        "label": "Revenue",
                        "units": {
                            "USD": [
                                {
                                    "end": "2024-09-28",
                                    "val": 391035000000.0_f64,
                                    "form": "10-K"
                                },
                                {
                                    "end": "2023-09-30",
                                    "val": 383285000000.0_f64,
                                    "form": "10-K"
                                },
                                {
                                    "end": "2024-03-30",
                                    "val": 90753000000.0_f64,
                                    "form": "10-Q"
                                }
                            ]
                        }
                    }
                }
            }
        })
        .to_string()
        .into_bytes(),
    )
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
    assert_eq!(rows[0].close, 391_035_000_000.0);
    assert_eq!(rows[0].venue, "sec");
    assert_eq!(rows[0].source, "sec-xbrl");

    assert_eq!(rows[1].ts, "2023-09-30T00:00:00Z");
    assert_eq!(rows[1].close, 383_285_000_000.0);
}

#[test]
fn cassette_filings_transform_query_roundtrip() {
    let q = SecFilingsHttpFetcher::transform_query(json!({"cik": "0000320193"}))
        .expect("transform_query must succeed");
    assert_eq!(q.padded_cik(), "0000320193");
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

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .expect("live extract_data must succeed");

    let rows = fetcher
        .transform_data(&query, raw)
        .expect("live transform_data must succeed");

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

    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .expect("live extract_data must succeed");

    let rows = fetcher
        .transform_data(&query, raw)
        .expect("live transform_data must succeed");

    assert!(
        !rows.is_empty(),
        "live EDGAR XBRL must return at least one annual Revenue bar"
    );
    assert_eq!(rows[0].venue, "sec");
    assert_eq!(rows[0].source, "sec-xbrl");
}
