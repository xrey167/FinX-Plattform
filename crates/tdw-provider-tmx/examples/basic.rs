//! Offline TMX example: validate a query, run the deterministic mock fetcher,
//! and decode a TMX `getquote` JSON body through the pure `parse_quote_response`
//! helper. No network access and no feature flags required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-tmx --example basic
//! ```

use serde_json::json;
use tdw_provider_tmx::{TmxMockQuoteFetcher, TmxQuoteQuery, parse_quote_response};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Validate a query ----------------------------------------------------
    let query = TmxQuoteQuery::from_params(&json!({ "symbol": "td" }))?;
    println!("validated symbol = {}", query.symbol); // upper-cased -> TD

    // --- Deterministic mock fetch (synchronous, offline) ---------------------
    let mocked = TmxMockQuoteFetcher::fetch_mock(&json!({ "symbol": "TD" }))?;
    for q in &mocked {
        println!("mock {} on {} last={}", q.symbol, q.exchange, q.last_price);
    }

    // --- Pure JSON parse of a TMX getquote envelope --------------------------
    let body = json!({
        "results": [
            {
                "symbol": "RY",
                "exchange": "TSX",
                "lastPrice": 134.10,
                "change": 0.85,
                "changePercent": 0.64,
                "volume": 2_750_000_u64,
                "marketCap": 189_000_000_000.0_f64,
                "open": 133.40,
                "high": 134.50,
                "low": 133.20,
                "close": 134.10
            }
        ]
    })
    .to_string();

    let parsed = parse_quote_response(body.as_bytes())?;
    for q in &parsed {
        println!(
            "parsed {} change%={} volume={}",
            q.symbol, q.change_percent, q.volume
        );
    }

    Ok(())
}
