//! Offline example for `tdw-provider-federal-reserve`.
//!
//! Mirrors the cassette path: builds a query with `transform_query`, then
//! decodes an inline Fed observation-list fixture with `transform_data` — no
//! network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-federal-reserve --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_federal_reserve::FedMacroSeriesHttpFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = FedMacroSeriesHttpFetcher::default();
    let query = FedMacroSeriesHttpFetcher::transform_query(json!({
        "command": "economy/money_measures"
    }))?;

    let raw = Bytes::from(
        json!({
            "observations": [
                { "date": "2024-01-01", "value": "20800.5" },
                { "date": "2024-02-01", "value": "." }
            ]
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} H.6 money-measure observation(s):", rows.len());
    for row in &rows {
        println!("  {} {} = {:?}", row.series_id, row.date, row.value);
    }
    Ok(())
}
