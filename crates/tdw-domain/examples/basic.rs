//! Offline `tdw-domain` example: construct a market-data record, validate it,
//! and build a fixed-width reference identifier.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-domain --example basic
//! ```

use tdw_domain::{Figi, MarketDataBar, TimeGranularity};
use validator::Validate;

fn main() {
    // Construct a canonical OHLCV bar on inline data.
    let bar = MarketDataBar {
        symbol: "AAPL".to_string(),
        venue: "XNAS".to_string(),
        granularity: TimeGranularity::Day,
        ts: "2026-05-21T00:00:00Z".to_string(),
        open: 100.0,
        high: 101.0,
        low: 99.0,
        close: 100.5,
        volume: 1_000.0,
        source: "example".to_string(),
    };

    // Run the meaningful operation: validate the record's field invariants.
    match bar.validate() {
        Ok(()) => println!(
            "valid bar: {} @ {} close={} ({:?})",
            bar.symbol, bar.venue, bar.close, bar.granularity
        ),
        Err(errors) => println!("invalid bar: {errors}"),
    }

    // A malformed record is rejected (negative price violates range(min = 0.0)).
    let bad = MarketDataBar {
        close: -1.0,
        ..bar.clone()
    };
    println!("negative-close bar rejected: {}", bad.validate().is_err());

    // Reference-id newtypes parse-then-construct: bad ids never exist.
    let figi = Figi::new("BBG000B9XRY4").expect("12-character FIGI is valid");
    println!("parsed FIGI: {}", figi.as_str());
    println!("short FIGI rejected: {}", Figi::new("NOPE").is_err());
}
