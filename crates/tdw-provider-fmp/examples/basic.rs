//! Offline example for `tdw-provider-fmp`.
//!
//! Mirrors the cassette path: builds a fundamentals query with
//! `transform_query`, then decodes an inline FMP income-statement fixture with
//! `transform_data` — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-fmp --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_fmp::FmpHttpIncomeFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = FmpHttpIncomeFetcher::default();

    let query = FmpHttpIncomeFetcher::transform_query(json!({
        "symbol": "AAPL",
        "statement": "income",
        "limit": 2,
    }))?;

    // Inline fixture identical in shape to a recorded FMP income-statement body
    // (a bare JSON array with camelCase numeric fields).
    let raw = Bytes::from(
        json!([
            {
                "date": "2024-09-28",
                "symbol": "AAPL",
                "revenue": 391035000000_i64,
                "grossProfit": 180683000000_i64,
                "netIncome": 93736000000_i64
            }
        ])
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} FMP income row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} revenue={} net_income={}",
            row.symbol, row.date, row.revenue, row.net_income
        );
    }

    Ok(())
}
