//! Offline example for `tdw-provider-eia`.
//!
//! Mirrors the cassette path: builds a query with `transform_query`, then
//! decodes an inline EIA v2 spot-price envelope with `transform_data` — no
//! network. EIA returns numeric values as JSON strings.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-eia --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_eia::EiaHttpSpotPriceFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = EiaHttpSpotPriceFetcher::default();

    let query = EiaHttpSpotPriceFetcher::transform_query(json!({
        "commodity": "crude_oil_wti",
        "length": 2,
    }))?;

    // Inline fixture identical in shape to a recorded EIA v2 response.
    let raw = Bytes::from(
        json!({
            "response": {
                "data": [
                    {
                        "period": "2024-01-03",
                        "product-name": "Crude Oil WTI",
                        "value": "72.70",
                        "units": "Dollars per Barrel"
                    },
                    {
                        "period": "2024-01-02",
                        "product-name": "Crude Oil WTI",
                        "value": "72.36",
                        "units": "Dollars per Barrel"
                    }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} EIA spot-price row(s):", rows.len());
    for row in &rows {
        println!(
            "  {} {} = {} {}",
            row.period, row.product_name, row.value, row.units
        );
    }

    Ok(())
}
