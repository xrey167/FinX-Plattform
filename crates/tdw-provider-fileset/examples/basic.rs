//! Offline example for `tdw-provider-fileset`.
//!
//! The fileset provider is fixture-backed, so this example needs no network
//! and no async runtime. It builds a query with `transform_query`, serialises
//! the canned `fixture_rows` to JSON (exactly what `extract_data` would
//! return), then decodes them with `transform_data`.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-fileset --example tdw-provider-fileset-basic
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_fileset::{FilesetEquityHistoricalFetcher, fixture_rows};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = FilesetEquityHistoricalFetcher;

    let query =
        FilesetEquityHistoricalFetcher::transform_query(serde_json::json!({ "symbol": "aapl" }))?;
    println!("normalised symbol: {}", query.symbol);

    // `extract_data` serialises the fixture rows to JSON; reproduce that here
    // without an async runtime, then decode via `transform_data`.
    let raw = Bytes::from(serde_json::to_vec(&fixture_rows(&query.symbol))?);
    let rows = fetcher.transform_data(&query, raw)?;

    println!("decoded {} fileset bar(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} o={} h={} l={} c={} v={}",
            row.symbol, row.date, row.open, row.high, row.low, row.close, row.volume
        );
    }

    Ok(())
}
