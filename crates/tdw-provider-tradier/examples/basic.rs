//! Offline Tradier example: feed inline Tradier-shaped fixtures through the
//! real `transform_data` path. No network access and no API token required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-tradier --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_tradier::{
    TradierHttpOptionsFetcher, TradierHttpQuoteFetcher, TradierOptionsQuery, TradierQuoteQuery,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Quote: GET /markets/quotes shape ------------------------------------
    let quote_fetcher = TradierHttpQuoteFetcher::default();
    let quote_query = TradierQuoteQuery::new("aapl")?;
    println!("quote symbol = {}", quote_query.symbol);

    let quote_fixture = Bytes::from(
        serde_json::json!({
            "quotes": {
                "quote": {
                    "symbol": "AAPL",
                    "last": 185.20,
                    "bid": 185.15,
                    "ask": 185.25,
                    "volume": 55_000_000,
                    "open": 184.50,
                    "high": 186.10,
                    "low": 184.20,
                    "close": 185.20,
                    "change": -0.50
                }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let quotes = quote_fetcher.transform_data(&quote_query, quote_fixture)?;
    for q in &quotes {
        println!("{} bid={} ask={}", q.symbol, q.bid, q.ask);
    }

    // --- Options chain: GET /markets/options/chains shape --------------------
    let options_fetcher = TradierHttpOptionsFetcher::default();
    let options_query = TradierOptionsQuery::new("AAPL", "2024-01-19")?;

    let options_fixture = Bytes::from(
        serde_json::json!({
            "options": {
                "option": [
                    {
                        "symbol": "AAPL240119C00180000",
                        "option_type": "call",
                        "strike": 180.0,
                        "bid": 5.10,
                        "ask": 5.30,
                        "open_interest": 12_500,
                        "volume": 850,
                        "expiration_date": "2024-01-19"
                    }
                ]
            }
        })
        .to_string()
        .into_bytes(),
    );
    let chain = options_fetcher.transform_data(&options_query, options_fixture)?;
    println!("option contracts parsed: {}", chain.len());

    Ok(())
}
