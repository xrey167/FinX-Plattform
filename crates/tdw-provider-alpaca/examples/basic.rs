//! Offline walkthrough of the Alpaca stock-bars fetcher.
//!
//! Mirrors the crate's cassette test: it constructs the fetcher, validates a
//! query with `transform_query`, and decodes an inline Alpaca envelope via
//! `transform_data`. No network access and no API keys are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-alpaca --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_alpaca::AlpacaHttpStockBarsFetcher;

fn main() -> tdw_core::Result<()> {
    let fetcher = AlpacaHttpStockBarsFetcher::default();

    let query = AlpacaHttpStockBarsFetcher::transform_query(json!({
        "symbol": "aapl",
        "start": "2024-01-02",
        "end": "2024-01-05",
        "timeframe": "1Day",
        "feed": "iex",
        "limit": 5
    }))?;

    // Recorded Alpaca stock-bars response shape.
    let fixture = Bytes::from(
        json!({
            "bars": {
                "AAPL": [
                    { "t": "2024-01-02T05:00:00Z", "o": 187.15, "h": 188.44, "l": 183.89, "c": 185.64, "v": 82_488_700.0 },
                    { "t": "2024-01-03T05:00:00Z", "o": 184.22, "h": 185.88, "l": 183.43, "c": 184.25, "v": 58_414_500.0 }
                ]
            },
            "next_page_token": null
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, fixture)?;
    println!("decoded {} bars for {}", rows.len(), query.symbol);
    for bar in &rows {
        println!(
            "  {} O={} H={} L={} C={} V={}",
            bar.ts, bar.open, bar.high, bar.low, bar.close, bar.volume
        );
    }

    Ok(())
}
