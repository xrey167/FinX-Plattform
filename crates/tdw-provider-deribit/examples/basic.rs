//! Offline walkthrough of the Deribit fetchers.
//!
//! Mirrors the crate's cassette tests: it constructs each fetcher, validates a
//! query with `transform_query`, and decodes an inline `{ "result": ... }`
//! envelope via `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-deribit --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_deribit::{
    DeribitHttpFundingFetcher, DeribitHttpInstrumentsFetcher, DeribitHttpOrderBookFetcher,
};

fn main() -> tdw_core::Result<()> {
    // --- Instruments -----------------------------------------------------
    let instruments = DeribitHttpInstrumentsFetcher::default();
    let instruments_query = DeribitHttpInstrumentsFetcher::transform_query(
        json!({ "currency": "btc", "kind": "option" }),
    )?;
    let instruments_fixture = Bytes::from(
        json!({
            "result": [
                { "instrument_name": "BTC-19JAN24-40000-C", "kind": "option", "strike": 40000.0, "expiration_timestamp": 1705651200000_u64, "option_type": "call", "is_active": true }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let rows = instruments.transform_data(&instruments_query, instruments_fixture)?;
    println!("instruments: {}", rows.len());
    for inst in &rows {
        println!(
            "  {} kind={} active={}",
            inst.instrument_name, inst.kind, inst.is_active
        );
    }

    // --- Order book ------------------------------------------------------
    let order_book = DeribitHttpOrderBookFetcher::default();
    let order_book_query = DeribitHttpOrderBookFetcher::transform_query(json!({
        "instrument_name": "BTC-19JAN24-40000-C",
        "depth": 5
    }))?;
    let order_book_fixture = Bytes::from(
        json!({
            "result": {
                "instrument_name": "BTC-19JAN24-40000-C",
                "bid_iv": 45.2, "ask_iv": 46.8, "mark_iv": 46.0, "underlying_price": 42500.0,
                "bids": [[5.1, 10.0]], "asks": [[5.3, 8.0]],
                "greeks": { "delta": 0.35, "gamma": 0.00002, "vega": 85.0, "theta": -120.0 }
            }
        })
        .to_string()
        .into_bytes(),
    );
    let books = order_book.transform_data(&order_book_query, order_book_fixture)?;
    println!("order books: {}", books.len());
    for book in &books {
        println!(
            "  {} bids={} asks={} mark_iv={:?}",
            book.instrument_name,
            book.bids.len(),
            book.asks.len(),
            book.mark_iv
        );
    }

    // --- Funding rate history -------------------------------------------
    let funding = DeribitHttpFundingFetcher::default();
    let funding_query = DeribitHttpFundingFetcher::transform_query(json!({
        "instrument_name": "BTC-PERPETUAL",
        "start_ms": 1_704_153_600_000_u64,
        "end_ms": 1_704_240_000_000_u64,
        "count": 100
    }))?;
    let funding_fixture = Bytes::from(
        json!({
            "result": [
                { "timestamp": 1_704_153_600_000_u64, "interest": 0.0001, "index_price": 42000.0 }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let records = funding.transform_data(&funding_query, funding_fixture)?;
    println!("funding records: {}", records.len());
    for rec in &records {
        println!(
            "  ts={} interest={} index={}",
            rec.timestamp, rec.interest, rec.index_price
        );
    }

    Ok(())
}
