//! Offline example for `tdw-provider-polygon`.
//!
//! Mirrors the cassette path: builds an aggregates query with `transform_query`,
//! then decodes an inline Polygon aggregates fixture with `transform_data` — no
//! network. Polygon timestamps are Unix milliseconds.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-polygon --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_polygon::PolygonHttpAggregatesFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = PolygonHttpAggregatesFetcher::default();

    let query = PolygonHttpAggregatesFetcher::transform_query(json!({
        "ticker": "MSFT",
        "from": "2024-01-02",
        "to": "2024-01-03",
    }))?;

    // Inline fixture identical in shape to a recorded Polygon aggregates body.
    let raw = Bytes::from(
        json!({
            "ticker": "MSFT",
            "status": "OK",
            "results": [
                { "o": 374.0, "h": 376.5, "l": 372.1, "c": 375.0, "v": 25000000.0, "t": 1_704_153_600_000_i64 },
                { "o": 375.5, "h": 378.0, "l": 374.0, "c": 377.2, "v": 23000000.0, "t": 1_704_240_000_000_i64 }
            ]
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} Polygon bar(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} close={} volume={}",
            row.symbol, row.ts, row.close, row.volume
        );
    }

    Ok(())
}
