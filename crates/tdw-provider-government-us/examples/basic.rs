//! Offline example for `tdw-provider-government-us`.
//!
//! Mirrors the cassette path: builds a query with `transform_query`, then
//! decodes an inline `FiscalData` auctions fixture with `transform_data` — no
//! network. `FiscalData` reports numeric columns as strings; the fetcher coerces
//! them.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-government-us --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_government_us::GovUsTreasuryAuctionsHttpFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = GovUsTreasuryAuctionsHttpFetcher::default();
    let query = GovUsTreasuryAuctionsHttpFetcher::transform_query(json!({
        "command": "fixedincome/government/treasury_auctions"
    }))?;

    let raw = Bytes::from(
        json!({
            "data": [
                {
                    "cusip": "912797GZ8",
                    "security_type": "Bill",
                    "security_term": "4-Week",
                    "auction_date": "2024-06-04",
                    "high_yield": "5.270",
                    "offering_amount": "80000000000"
                }
            ]
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} Treasury auction(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} high_yield={:?}",
            row.cusip, row.auction_date, row.high_yield
        );
    }
    Ok(())
}
