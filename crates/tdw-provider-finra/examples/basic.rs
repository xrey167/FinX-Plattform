//! Offline example for `tdw-provider-finra`.
//!
//! Mirrors the cassette test: builds a query, then decodes an inline
//! pipe-delimited short-interest fixture with `transform_data` — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-finra --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_finra::FinraShortInterestHttpFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = FinraShortInterestHttpFetcher::default();

    let query = FinraShortInterestHttpFetcher::transform_query(serde_json::json!({
        "limit": 25,
        "offset": 0,
    }))?;

    // Inline fixture identical in shape to a recorded FINRA pipe-delimited body.
    let raw = Bytes::from(concat!(
        "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15\n",
        "TESLA INC|TSLA|G|55000000|54000000|1.85|22000000|2.5|2024-01-15\n",
    ));

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} FINRA short-interest row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} ({:?}) short={:?} days_to_cover={:?}",
            row.symbol, row.settlement_date, row.current_short_interest, row.days_to_cover
        );
    }

    Ok(())
}
