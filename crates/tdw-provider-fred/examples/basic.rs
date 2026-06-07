//! Offline example for `tdw-provider-fred`.
//!
//! Mirrors the cassette path: builds a query with `transform_query`, then
//! decodes an inline FRED `series/observations` fixture with `transform_data`
//! — no network. FRED reports values as JSON strings and uses "." for gaps.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-fred --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_fred::FredHttpSeriesObservationsFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = FredHttpSeriesObservationsFetcher::default();

    let query = FredHttpSeriesObservationsFetcher::transform_query(json!({
        "series_id": "UNRATE",
    }))?;

    // Inline fixture identical in shape to a recorded FRED response.
    let raw = Bytes::from(
        json!({
            "observations": [
                {
                    "realtime_start": "2024-02-01",
                    "realtime_end": "2024-02-01",
                    "date": "2024-01-01",
                    "value": "3.7"
                },
                {
                    "realtime_start": "2024-02-01",
                    "realtime_end": "2024-02-01",
                    "date": "2024-02-01",
                    "value": "."
                }
            ]
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} FRED observation(s) (gaps skipped):", rows.len());
    for row in &rows {
        println!("  {} {} = {}", row.series_id, row.date, row.value);
    }

    Ok(())
}
