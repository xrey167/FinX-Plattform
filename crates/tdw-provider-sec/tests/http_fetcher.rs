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
use tdw_provider_sec::http_fetcher::WWW_BASE_URL;
use tdw_provider_sec::{
    BASE_URL, SecCikMapHttpFetcher, SecCompanyFactsHttpFetcher, SecEtfHoldingsHttpFetcher,
    SecFailsToDeliverHttpFetcher, SecFilingHeadersHttpFetcher, SecFilingsHttpFetcher,
    SecFilingsQuery, SecForm13FHttpFetcher, SecHistoricalQuery, SecInstitutionsSearchHttpFetcher,
    SecLatestFinancialReportsHttpFetcher, SecRssLitigationHttpFetcher, SecSchemaFilesHttpFetcher,
    SecSicSearchHttpFetcher, SecSymbolMapHttpFetcher, SecXbrlHttpFetcher,
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

// ── P4W8 keyless regulator utilities (cassette tests) ────────────────────────

fn company_tickers_cassette() -> Bytes {
    cassette_bytes!({
        "0": { "cik_str": 320_193, "ticker": "aapl", "title": "Apple Inc." },
        "1": { "cik_str": 19_617, "ticker": "JPM", "title": "JPMORGAN CHASE & CO" },
        "2": { "cik_str": 886_982, "ticker": "GS", "title": "GOLDMAN SACHS GROUP INC" }
    })
}

#[test]
fn cassette_symbol_map_parses_company_tickers() {
    let fetcher = SecSymbolMapHttpFetcher::default();
    let query = SecSymbolMapHttpFetcher::transform_query(json!({})).expect("empty query");
    let rows = fetcher
        .transform_data(&query, company_tickers_cassette())
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 3, "rows={rows:#?}");
    let aapl = rows.iter().find(|r| r.symbol == "AAPL").expect("aapl row");
    assert_eq!(aapl.cik, "320193");
}

#[test]
fn cassette_institutions_search_filters_by_name() {
    let fetcher = SecInstitutionsSearchHttpFetcher::default();
    let query = SecInstitutionsSearchHttpFetcher::transform_query(json!({"query": "goldman"}))
        .expect("search query");
    let rows = fetcher
        .transform_data(&query, company_tickers_cassette())
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 1, "only the goldman row; rows={rows:#?}");
    assert_eq!(rows[0].cik, "886982");
    assert_eq!(rows[0].symbol.as_deref(), Some("GS"));
    assert!(rows[0].name.to_lowercase().contains("goldman"));
}

#[test]
fn institutions_search_empty_query_lists_all() {
    let fetcher = SecInstitutionsSearchHttpFetcher::default();
    let query =
        SecInstitutionsSearchHttpFetcher::transform_query(json!({})).expect("empty search query");
    let rows = fetcher
        .transform_data(&query, company_tickers_cassette())
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 3, "empty query lists all; rows={rows:#?}");
}

#[test]
fn sic_search_filters_by_code_or_description() {
    let fetcher = SecSicSearchHttpFetcher::default();
    let query = SecSicSearchHttpFetcher::transform_query(json!({"query": "software"}))
        .expect("search query");
    // The SIC table is embedded; extract_data returns empty bytes by design.
    let rows = fetcher
        .transform_data(&query, Bytes::new())
        .expect("transform_data must succeed");
    assert!(
        rows.iter().any(|r| r.code == "7372"),
        "expected Prepackaged Software (7372); rows={rows:#?}"
    );
    assert!(
        rows.iter()
            .all(|r| r.description.to_lowercase().contains("software")),
        "all rows match the needle; rows={rows:#?}"
    );

    // Numeric-code needle.
    let by_code =
        SecSicSearchHttpFetcher::transform_query(json!({"query": "3571"})).expect("code query");
    let code_rows = fetcher
        .transform_data(&by_code, Bytes::new())
        .expect("transform_data must succeed");
    assert_eq!(code_rows.len(), 1);
    assert_eq!(code_rows[0].code, "3571");
}

#[test]
fn cassette_filing_headers_parses_index() {
    let fetcher = SecFilingHeadersHttpFetcher::default();
    let query = SecFilingHeadersHttpFetcher::transform_query(
        json!({"cik": "320193", "accession": "0000320193-24-000123"}),
    )
    .expect("accession query");
    let raw = cassette_bytes!({
        "directory": {
            "name": "/Archives/edgar/data/320193/000032019324000123",
            "item": []
        },
        "formType": "10-K",
        "filingDate": "2024-11-01",
        "periodOfReport": "2024-09-28",
        "description": "Annual report"
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].cik, "320193");
    assert_eq!(rows[0].accession_number, "0000320193-24-000123");
    assert_eq!(rows[0].form_type.as_deref(), Some("10-K"));
    assert_eq!(rows[0].period_of_report.as_deref(), Some("2024-09-28"));
}

#[test]
fn cassette_schema_files_lists_directory_items() {
    let fetcher = SecSchemaFilesHttpFetcher::default();
    let query = SecSchemaFilesHttpFetcher::transform_query(
        json!({"cik": "320193", "accession": "0000320193-24-000123"}),
    )
    .expect("accession query");
    let raw = cassette_bytes!({
        "directory": {
            "item": [
                {"name": "aapl-20240928.htm", "type": "10-K", "size": "1234567",
                 "last-modified": "2024-11-01 18:01:14"},
                {"name": "aapl-20240928.xsd", "type": "EX-101.SCH", "size": 8910},
                {"name": "", "type": "ignored"}
            ]
        }
    });
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 2, "empty-name row skipped; rows={rows:#?}");
    assert_eq!(rows[0].name, "aapl-20240928.htm");
    assert_eq!(rows[0].file_type.as_deref(), Some("10-K"));
    assert_eq!(rows[0].size, Some(1_234_567));
    assert_eq!(rows[1].size, Some(8910));
}

#[test]
fn cassette_rss_litigation_parses_items() {
    let fetcher = SecRssLitigationHttpFetcher::default();
    let query = SecRssLitigationHttpFetcher::transform_query(json!({})).expect("empty query");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>SEC Litigation Releases</title>
    <item>
      <title><![CDATA[SEC Charges Example Corp]]></title>
      <link>https://www.sec.gov/litigation/litreleases/lr-12345</link>
      <pubDate>Mon, 03 Jun 2024 12:00:00 EST</pubDate>
      <description>The SEC announced charges against Example Corp.</description>
    </item>
    <item>
      <title>SEC Settles With John Doe</title>
      <link>https://www.sec.gov/litigation/litreleases/lr-12346</link>
      <pubDate>Tue, 04 Jun 2024 09:30:00 EST</pubDate>
    </item>
  </channel>
</rss>"#;
    let raw = Bytes::from(xml.as_bytes().to_vec());
    let rows = fetcher
        .transform_data(&query, raw)
        .expect("transform_data must succeed");
    assert_eq!(rows.len(), 2, "rows={rows:#?}");
    assert_eq!(rows[0].title, "SEC Charges Example Corp");
    assert_eq!(
        rows[0].link,
        "https://www.sec.gov/litigation/litreleases/lr-12345"
    );
    assert!(
        rows[0]
            .summary
            .as_deref()
            .unwrap_or_default()
            .contains("Example Corp")
    );
    assert_eq!(rows[1].title, "SEC Settles With John Doe");
    assert_eq!(rows[1].summary, None);
}

#[test]
fn base_urls_use_tls() {
    assert!(
        BASE_URL.starts_with("https://"),
        "data base URL: {BASE_URL}"
    );
    assert!(
        WWW_BASE_URL.starts_with("https://"),
        "www base URL: {WWW_BASE_URL}"
    );
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

#[tokio::test]
async fn live_sec_company_facts_returns_facts_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC company_facts integration test");
        return;
    }
    let fetcher = SecCompanyFactsHttpFetcher::default();
    let query = SecCompanyFactsHttpFetcher::transform_query(json!({"cik": "320193"}))
        .expect("transform_query must succeed");
    let rows = live_fetch_rows_expect!(fetcher, query);
    assert!(!rows.is_empty(), "live company_facts must return facts");
    assert_eq!(rows[0].cik, "320193");
}

#[tokio::test]
async fn live_sec_latest_financial_reports_returns_data_when_env_var_set() {
    if std::env::var("TDW_SEC_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_SEC_LIVE != 1; skipping live SEC latest_financial_reports integration test");
        return;
    }
    let fetcher = SecLatestFinancialReportsHttpFetcher::default();
    let query = SecLatestFinancialReportsHttpFetcher::transform_query(json!({"cik": "320193"}))
        .expect("transform_query must succeed");
    let rows = live_fetch_rows_expect!(fetcher, query);
    assert!(
        !rows.is_empty(),
        "live latest_financial_reports must return at least one periodic report"
    );
    assert!(
        rows.iter()
            .all(|r| ["10-K", "10-Q", "20-F", "40-F"].contains(&r.form_type.as_str())),
        "all rows must be periodic financial reports"
    );
}
