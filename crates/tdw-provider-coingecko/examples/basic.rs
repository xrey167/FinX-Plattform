//! Offline walkthrough of the CoinGecko OHLC fetcher.
//!
//! Mirrors the crate's inline cassette test: it constructs the fetcher,
//! validates a query with `transform_query`, and decodes an inline OHLC
//! array-of-arrays via `transform_data`. No network access and no API key are
//! required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-coingecko --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_coingecko::CoinGeckoHttpOhlcFetcher;

fn main() -> tdw_core::Result<()> {
    let fetcher = CoinGeckoHttpOhlcFetcher::default();

    let query = CoinGeckoHttpOhlcFetcher::transform_query(json!({
        "coin_id": "bitcoin",
        "vs_currency": "usd",
        "days": 30
    }))?;

    // CoinGecko OHLC response: [[ts_ms, open, high, low, close], ...].
    let fixture = Bytes::from(
        json!([
            [1704067200000_i64, 42000.0, 43000.0, 41500.0, 42500.0],
            [1704153600000_i64, 42500.0, 44000.0, 42000.0, 43800.0]
        ])
        .to_string()
        .into_bytes(),
    );

    let bars = fetcher.transform_data(&query, fixture)?;
    println!("decoded {} bars for {}", bars.len(), query.coin_id);
    for bar in &bars {
        println!(
            "  {} O={} H={} L={} C={}",
            bar.ts, bar.open, bar.high, bar.low, bar.close
        );
    }

    Ok(())
}
