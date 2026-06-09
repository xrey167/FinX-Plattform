//! Offline walkthrough of the CCData fetchers.
//!
//! Mirrors the crate's cassette tests: it constructs each fetcher, validates a
//! query with `transform_query`, and decodes an inline `{ "Data": ... }`
//! envelope via `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-ccdata --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_ccdata::CCDataHttpFetcher;
use tdw_provider_ccdata::http_fetcher::CCDataAssetHttpFetcher;

fn main() -> tdw_core::Result<()> {
    // --- Daily OHLCV -----------------------------------------------------
    let ohlcv = CCDataHttpFetcher::default();
    let ohlcv_query = CCDataHttpFetcher::transform_query(json!({
        "market": "ccix",
        "instrument": "BTC-USD",
        "limit": 30
    }))?;
    let ohlcv_fixture = Bytes::from(
        json!({
            "Data": {
                "Instrument": "BTC-USD",
                "Entries": [
                    { "TIMESTAMP": 1704067200, "OPEN": 42000.0, "HIGH": 45000.0, "LOW": 41500.0, "CLOSE": 44000.0, "VOLUME": 850000.0 },
                    { "TIMESTAMP": 1704153600, "OPEN": 44000.0, "HIGH": 46000.0, "LOW": 43000.0, "CLOSE": 45500.0, "VOLUME": 920000.0 }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    );
    let bars = ohlcv.transform_data(&ohlcv_query, ohlcv_fixture)?;
    println!("ohlcv bars: {}", bars.len());
    for bar in &bars {
        println!("  {} {} close={}", bar.ts, bar.symbol, bar.close);
    }

    // --- Asset metadata --------------------------------------------------
    let asset = CCDataAssetHttpFetcher::default();
    let asset_query = CCDataAssetHttpFetcher::transform_query(json!({ "symbol": "btc" }))?;
    let asset_fixture = Bytes::from(
        json!({
            "Data": {
                "ID": 1182,
                "SYMBOL": "BTC",
                "NAME": "Bitcoin",
                "ASSET_TYPE": "BLOCKCHAIN",
                "LAUNCH_DATE": 1230940800,
                "CIRCULATING_SUPPLY": 19500000.0,
                "MARKET_CAP_USD": 858000000000.0
            }
        })
        .to_string()
        .into_bytes(),
    );
    let assets = asset.transform_data(&asset_query, asset_fixture)?;
    println!("asset rows: {}", assets.len());
    for row in &assets {
        println!(
            "  {} ({}) mcap_usd={}",
            row.symbol, row.name, row.market_cap_usd
        );
    }

    Ok(())
}
