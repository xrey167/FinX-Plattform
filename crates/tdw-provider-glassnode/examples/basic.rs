//! Offline example for `tdw-provider-glassnode`.
//!
//! Mirrors the cassette path: builds a metric query with `transform_query`,
//! then decodes an inline Glassnode `[{t, v}]` fixture with `transform_data`
//! — no network.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-glassnode --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_glassnode::GlassnodeHttpFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = GlassnodeHttpFetcher::default();

    let query = GlassnodeHttpFetcher::transform_query(json!({
        "asset": "BTC",
        "metric": "mvrv_z_score",
        "interval": "24h",
    }))?;

    // Inline fixture identical in shape to a recorded Glassnode response.
    let raw = Bytes::from(
        json!([
            { "t": 1_704_067_200_i64, "v": 1.72 },
            { "t": 1_704_153_600_i64, "v": 1.85 }
        ])
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, raw)?;
    println!("decoded {} Glassnode data point(s):", rows.len());
    for row in &rows {
        println!("  {} t={} value={}", row.asset, row.timestamp, row.value);
    }

    Ok(())
}
