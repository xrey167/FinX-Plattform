//! Offline example for `tdw-provider-oecd`.
//!
//! Mirrors the cassette path: builds a query with `transform_query`, then
//! decodes an inline OECD SDMX-JSON fixture with `transform_data` — no network.
//! The last colon component of each observation key indexes TIME_PERIOD.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-oecd --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_oecd::OecdHttpDataFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = OecdHttpDataFetcher::default();

    let query = OecdHttpDataFetcher::transform_query(json!({
        "dataset": "QNA",
        "filter": "AUS.B1_GE.Q",
        "start_time": "2023",
        "end_time": "2023",
    }))?;

    // Inline fixture identical in shape to a recorded OECD SDMX-JSON response.
    let raw = Bytes::from(
        json!({
            "dataSets": [{
                "observations": {
                    "0:0:0:0": [1234.5, 0, null],
                    "0:0:0:1": [1250.0, 0, null]
                }
            }],
            "structure": {
                "dimensions": {
                    "observation": [{
                        "id": "TIME_PERIOD",
                        "values": [
                            { "id": "2023-Q1", "name": "2023-Q1" },
                            { "id": "2023-Q2", "name": "2023-Q2" }
                        ]
                    }]
                }
            }
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} OECD observation(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} ({}) = {}",
            row.dataset, row.period, row.key, row.value
        );
    }

    Ok(())
}
