//! Offline example for `tdw-provider-geckoterminal`.
//!
//! Mirrors the cassette path: builds a pool query with `transform_query`, then
//! decodes an inline GeckoTerminal single-pool JSON:API fixture with
//! `transform_data` — no network. Prices/volumes arrive as JSON strings.
//!
//! Run with:
//! ```bash
//! cargo run -p tdw-provider-geckoterminal --example basic --features http
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_geckoterminal::GeckoTerminalHttpFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = GeckoTerminalHttpFetcher::default();

    let query = GeckoTerminalHttpFetcher::transform_query(json!({
        "network": "eth",
        "pool_address": "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
    }))?;

    // Inline fixture identical in shape to a recorded GeckoTerminal response.
    let raw = Bytes::from(
        json!({
            "data": {
                "id": "eth_0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
                "attributes": {
                    "name": "USDC/WETH 0.05%",
                    "base_token_price_usd": "1.0001",
                    "quote_token_price_usd": "3125.50",
                    "reserve_in_usd": "125000000.5",
                    "volume_usd": { "h24": "85000000.0", "h6": "22000000.0", "h1": "4500000.0" },
                    "price_change_percentage": { "h24": "-0.25", "h6": "0.10", "h1": "0.05" },
                    "pool_created_at": "2021-05-04T00:00:00Z"
                }
            }
        })
        .to_string()
        .into_bytes(),
    );

    let pools = fetcher.transform_data(&query, raw)?;
    println!("decoded {} GeckoTerminal pool(s):", pools.len());
    for pool in &pools {
        println!(
            "  {} {} reserve_usd={:?} vol_24h={:?}",
            pool.network, pool.name, pool.reserve_in_usd, pool.volume_usd_h24
        );
    }

    Ok(())
}
