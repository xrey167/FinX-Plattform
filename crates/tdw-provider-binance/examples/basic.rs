//! Offline walkthrough of the Binance ticker fetcher and trade decoder.
//!
//! Mirrors the crate's cassette + decoder tests: it constructs the ticker
//! fetcher, validates a query with `transform_query`, decodes an inline fixture
//! via `transform_data`, and then decodes a recorded trade frame with the pure
//! `decode_trade_frame` function. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-binance --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_binance::{BinanceHttpTickerPriceFetcher, decode_trade_frame};

fn main() -> tdw_core::Result<()> {
    // --- REST ticker price ----------------------------------------------
    let fetcher = BinanceHttpTickerPriceFetcher::default();
    let query = BinanceHttpTickerPriceFetcher::transform_query(json!({ "symbol": "btcusdt" }))?;
    let fixture = Bytes::from(
        json!({ "symbol": "BTCUSDT", "price": "67432.12000000" })
            .to_string()
            .into_bytes(),
    );
    let rows = fetcher.transform_data(&query, fixture)?;
    println!("ticker rows: {}", rows.len());
    for row in &rows {
        println!("  {} = {}", row.symbol, row.price);
    }

    // --- Trade frame decode (pure, no feature needed) -------------------
    let frame =
        r#"{"e":"trade","s":"btcusdt","p":"68000.50","q":"0.01","T":1700000000000,"m":false}"#;
    let ticks = decode_trade_frame(frame)?;
    println!("decoded ticks: {}", ticks.len());
    for tick in &ticks {
        println!(
            "  {} {} price={} size={} ts={}",
            tick.symbol, tick.venue, tick.price, tick.size, tick.ts
        );
    }

    Ok(())
}
