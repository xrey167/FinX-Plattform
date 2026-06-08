//! Offline Trading Economics example: validate queries and run the
//! deterministic offline stub fetchers. No network access and no feature flags
//! required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-trading-economics --example basic
//! ```

use tdw_provider_trading_economics::{
    TradingEconomicsCalendarQuery, TradingEconomicsIndicatorQuery,
    TradingEconomicsMockCalendarFetcher, TradingEconomicsMockIndicatorFetcher,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Economic calendar (importance-filtered) -----------------------------
    let calendar_query = TradingEconomicsCalendarQuery::new(3)?; // high importance
    let events = TradingEconomicsMockCalendarFetcher::fetch_stub(&calendar_query)?;
    for e in &events {
        println!(
            "calendar: {} | {} | importance={}",
            e.date, e.event, e.importance
        );
    }

    // --- Country indicator ----------------------------------------------------
    let indicator_query = TradingEconomicsIndicatorQuery::new("United States", "gdp-growth-rate")?;
    let rows = TradingEconomicsMockIndicatorFetcher::fetch_stub(&indicator_query)?;
    for r in &rows {
        println!(
            "indicator: {} {} = {} ({})",
            r.country, r.category, r.value, r.frequency
        );
    }

    Ok(())
}
