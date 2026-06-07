//! Offline walkthrough of the AkShare historical-bar fetcher.
//!
//! Mirrors the crate's cassette test: it constructs the fetcher, validates a
//! query with `transform_query`, and decodes an inline fixture (with the
//! original Chinese field names) via `transform_data`. No network access and no
//! API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-akshare --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_akshare::AkShareHttpFetcher;

fn main() -> tdw_core::Result<()> {
    let fetcher = AkShareHttpFetcher::default();

    let query = AkShareHttpFetcher::transform_query(json!({
        "symbol": "000001",
        "market": "AShares",
        "start_date": "20240101",
        "end_date": "20240131"
    }))?;

    // Recorded AkShare response shape (daily A-share bars).
    let fixture = Bytes::from(
        json!([
            { "日期": "2024-01-02", "开盘": 10.5, "收盘": 10.8, "最高": 10.9, "最低": 10.4, "成交量": 8_500_000.0 },
            { "日期": "2024-01-03", "开盘": 10.8, "收盘": 11.0, "最高": 11.2, "最低": 10.7, "成交量": 9_200_000.0 }
        ])
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, fixture)?;
    println!(
        "decoded {} bars (venue={})",
        rows.len(),
        query.market.venue()
    );
    for bar in &rows {
        println!(
            "  {} O={} H={} L={} C={} V={}",
            bar.ts, bar.open, bar.high, bar.low, bar.close, bar.volume
        );
    }

    Ok(())
}
