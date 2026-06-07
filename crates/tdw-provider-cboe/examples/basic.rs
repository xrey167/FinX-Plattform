//! Offline walkthrough of the CBOE fetchers.
//!
//! Mirrors the crate's cassette tests: it constructs each fetcher, validates a
//! query with `transform_query`, and decodes an inline `{ "data": ... }`
//! envelope via `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-cboe --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_cboe::{CboeHttpIndexFetcher, CboeHttpOptionsFetcher};

fn main() -> tdw_core::Result<()> {
    // --- Delayed options chain ------------------------------------------
    let options = CboeHttpOptionsFetcher::default();
    let options_query = CboeHttpOptionsFetcher::transform_query(json!({ "symbol": "aapl" }))?;
    let options_fixture = Bytes::from(
        json!({
            "data": {
                "options": [
                    {
                        "option": "AAPL240119C00180000",
                        "bid": 5.10, "ask": 5.30, "iv": 0.235,
                        "delta": 0.45, "gamma": 0.02, "theta": -0.05,
                        "open_interest": 12500
                    }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    );
    let contracts = options.transform_data(&options_query, options_fixture)?;
    println!("options contracts: {}", contracts.len());
    for c in &contracts {
        println!(
            "  {} bid={} ask={} oi={}",
            c.option, c.bid, c.ask, c.open_interest
        );
    }

    // --- US-index quote --------------------------------------------------
    let index = CboeHttpIndexFetcher::default();
    let index_query = CboeHttpIndexFetcher::transform_query(json!({ "index": "vix" }))?;
    let index_fixture = Bytes::from(
        json!({
            "data": { "symbol": "VIX", "price": 13.45, "change": -0.23, "volume": 0 }
        })
        .to_string()
        .into_bytes(),
    );
    let quotes = index.transform_data(&index_query, index_fixture)?;
    println!("index quotes: {}", quotes.len());
    for q in &quotes {
        println!("  {} price={} change={}", q.symbol, q.price, q.change);
    }

    Ok(())
}
