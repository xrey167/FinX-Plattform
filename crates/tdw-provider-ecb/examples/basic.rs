//! Offline example for `tdw-provider-ecb`.
//!
//! Mirrors the cassette test: builds a query with `transform_query`, then
//! decodes an inline ECB `jsondata` fixture with `transform_data` — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-ecb --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_ecb::EcbHttpDataFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = EcbHttpDataFetcher::default();

    let query = EcbHttpDataFetcher::transform_query(json!({
        "flow": "EXR",
        "key": "D.USD.EUR.SP00.A",
        "start_period": "2024-01-01",
        "end_period": "2024-01-31",
    }))?;

    // Inline fixture identical in shape to a recorded SDW `jsondata` response.
    let raw = Bytes::from(
        json!({
            "dataSets": [{
                "series": {
                    "0:0:0:0:0": {
                        "observations": {
                            "0": [1.0934, 0, null],
                            "1": [1.0945, 0, null]
                        }
                    }
                }
            }],
            "structure": {
                "dimensions": {
                    "observation": [{
                        "id": "TIME_PERIOD",
                        "values": [
                            { "id": "2024-01-02" },
                            { "id": "2024-01-03" }
                        ]
                    }]
                }
            }
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} ECB observation(s):", rows.len());
    for row in &rows {
        println!("  {} {} = {}", row.date, row.key, row.value);
    }

    Ok(())
}
