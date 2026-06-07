//! Offline example for `tdw-provider-nasdaq`.
//!
//! Mirrors the cassette path: builds a dataset query with `transform_query`,
//! then decodes an inline NASDAQ Data Link `dataset_data` fixture with
//! `transform_data` — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-nasdaq --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_nasdaq::NasdaqHttpDatasetFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = NasdaqHttpDatasetFetcher::default();

    let query = NasdaqHttpDatasetFetcher::transform_query(json!({
        "database": "WIKI",
        "dataset": "AAPL",
    }))?;

    // Inline fixture identical in shape to a recorded NASDAQ Data Link response.
    let raw = Bytes::from(
        json!({
            "dataset_data": {
                "column_names": ["Date", "Open", "High", "Low", "Close", "Volume"],
                "data": [
                    ["2024-01-03", 184.2, 185.9, 183.4, 184.0, 47000000],
                    ["2024-01-02", 185.6, 186.1, 184.4, 185.2, 55000000]
                ]
            }
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} NASDAQ row(s):", rows.len());
    for row in &rows {
        println!("  {}/{} {:?}", row.database, row.dataset, row.values);
    }

    Ok(())
}
