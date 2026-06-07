//! Offline walkthrough of the Alpha Vantage fetcher.
//!
//! Mirrors the crate's cassette tests: it constructs the fetcher, validates a
//! query with `transform_query`, and decodes inline fixtures for both supported
//! functions via `transform_data`. No network access and no API key are
//! required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-alpha-vantage --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_alpha_vantage::AlphaVantageHttpFetcher;

fn main() -> tdw_core::Result<()> {
    let fetcher = AlphaVantageHttpFetcher::default();

    // --- TIME_SERIES_DAILY ----------------------------------------------
    let daily_query = AlphaVantageHttpFetcher::transform_query(json!({
        "symbol": "msft",
        "function": "TIME_SERIES_DAILY"
    }))?;
    let daily_fixture = Bytes::from(
        json!({
            "Meta Data": { "2. Symbol": "MSFT" },
            "Time Series (Daily)": {
                "2024-01-02": { "1. open": "373.86", "2. high": "375.90", "3. low": "366.77", "4. close": "370.87", "5. volume": "25258600" },
                "2024-01-03": { "1. open": "369.01", "2. high": "373.26", "3. low": "366.09", "4. close": "367.94", "5. volume": "23083500" }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let daily_rows = fetcher.transform_data(&daily_query, daily_fixture)?;
    println!("TIME_SERIES_DAILY rows: {}", daily_rows.len());
    for bar in &daily_rows {
        println!("  {} close={} vol={}", bar.ts, bar.close, bar.volume);
    }

    // --- GLOBAL_QUOTE ----------------------------------------------------
    let quote_query = AlphaVantageHttpFetcher::transform_query(json!({
        "symbol": "aapl",
        "function": "GLOBAL_QUOTE"
    }))?;
    let quote_fixture = Bytes::from(
        json!({
            "Global Quote": {
                "01. symbol": "AAPL",
                "05. price": "185.20",
                "06. volume": "55000000"
            }
        })
        .to_string()
        .into_bytes(),
    );
    let quote_rows = fetcher.transform_data(&quote_query, quote_fixture)?;
    println!("GLOBAL_QUOTE rows: {}", quote_rows.len());
    for bar in &quote_rows {
        println!("  {} price={} vol={}", bar.symbol, bar.close, bar.volume);
    }

    Ok(())
}
