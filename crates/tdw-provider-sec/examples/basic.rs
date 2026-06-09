//! Offline SEC EDGAR example: feed inline EDGAR-shaped fixtures through the
//! real `transform_data` path. No network access and no API key required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-sec --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_sec::{
    SecFilingsHttpFetcher, SecFilingsQuery, SecHistoricalQuery, SecXbrlHttpFetcher,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Filings: GET /submissions/CIK*.json shape ---------------------------
    let filings_fetcher = SecFilingsHttpFetcher::default();
    let filings_query = SecFilingsQuery::new("320193")?; // Apple Inc.
    println!("filings padded CIK = {}", filings_query.padded_cik());

    let filings_fixture = Bytes::from(
        serde_json::json!({
            "cik": "320193",
            "name": "Apple Inc.",
            "filings": {
                "recent": {
                    "accessionNumber": ["0000320193-24-000123"],
                    "form": ["10-K"],
                    "filingDate": ["2024-10-01"]
                }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let filings = filings_fetcher.transform_data(&filings_query, filings_fixture)?;
    for f in &filings {
        println!("filing: {} {} ({})", f.entity_name, f.form, f.filing_date);
    }

    // --- XBRL company-facts: only 10-K Revenue facts become bars -------------
    let xbrl_fetcher = SecXbrlHttpFetcher::default();
    let xbrl_query = SecHistoricalQuery::new("320193")?; // CIK passed as symbol

    let xbrl_fixture = Bytes::from(
        serde_json::json!({
            "cik": 320193,
            "entityName": "Apple Inc.",
            "facts": {
                "us-gaap": {
                    "Revenue": {
                        "label": "Revenue",
                        "units": {
                            "USD": [
                                {"end": "2024-09-28", "val": 391_035_000_000.0_f64, "form": "10-K"},
                                {"end": "2024-03-30", "val": 90_753_000_000.0_f64, "form": "10-Q"}
                            ]
                        }
                    }
                }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let bars = xbrl_fetcher.transform_data(&xbrl_query, xbrl_fixture)?;
    println!("annual revenue bars (10-Q excluded): {}", bars.len());
    for bar in &bars {
        println!("  {} close={} source={}", bar.ts, bar.close, bar.source);
    }

    Ok(())
}
