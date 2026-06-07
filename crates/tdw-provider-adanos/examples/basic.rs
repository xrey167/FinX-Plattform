//! Offline walkthrough of the Adanos fetchers.
//!
//! Mirrors the crate's cassette tests: it constructs each fetcher, validates a
//! query with `transform_query`, and decodes an inline fixture with
//! `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-adanos --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_adanos::{
    AdanosPolymarketHttpFetcher, AdanosSentimentHttpFetcher, AdanosTrendingHttpFetcher,
};

fn main() -> tdw_core::Result<()> {
    // --- Sentiment -------------------------------------------------------
    let sentiment_fixture = Bytes::from_static(
        br#"{
            "ticker": "AAPL",
            "timestamp": 1704153600,
            "sentimentScore": 0.72,
            "buzzScore": 85,
            "sources": { "reddit": 0.68, "twitter": 0.75, "news": 0.74 },
            "mentions": { "reddit": 1250, "twitter": 8500, "news": 45 },
            "trend": "bullish"
        }"#,
    );
    let sentiment = AdanosSentimentHttpFetcher::default();
    let sentiment_query = AdanosSentimentHttpFetcher::transform_query(json!({ "ticker": "aapl" }))?;
    let sentiment_rows = sentiment.transform_data(&sentiment_query, sentiment_fixture)?;
    println!("sentiment rows: {}", sentiment_rows.len());
    for row in &sentiment_rows {
        println!(
            "  {} score={} buzz={} trend={}",
            row.ticker, row.sentiment_score, row.buzz_score, row.trend
        );
    }

    // --- Trending --------------------------------------------------------
    let trending_fixture = Bytes::from_static(
        br#"{
            "timestamp": 1704153600,
            "trending": [
                { "ticker": "NVDA", "buzzScore": 98, "sentimentScore": 0.85, "mentions": 25000 },
                { "ticker": "AAPL", "buzzScore": 85, "sentimentScore": 0.72, "mentions": 18000 }
            ]
        }"#,
    );
    let trending = AdanosTrendingHttpFetcher::default();
    let trending_query = AdanosTrendingHttpFetcher::transform_query(json!({ "limit": 10 }))?;
    let trending_rows = trending.transform_data(&trending_query, trending_fixture)?;
    println!("trending rows: {}", trending_rows.len());
    for row in &trending_rows {
        println!(
            "  {} buzz={} mentions={}",
            row.ticker, row.buzz_score, row.mentions
        );
    }

    // --- Polymarket ------------------------------------------------------
    let polymarket_fixture = Bytes::from_static(
        br#"{
            "events": [
                {
                    "id": "abc123",
                    "title": "Will BTC exceed $100k by Dec 2024?",
                    "probability": 0.42,
                    "volume": 1500000.0,
                    "expiresAt": "2024-12-31T23:59:59Z"
                }
            ]
        }"#,
    );
    let polymarket = AdanosPolymarketHttpFetcher::default();
    let polymarket_query = AdanosPolymarketHttpFetcher::transform_query(json!({ "limit": 5 }))?;
    let polymarket_rows = polymarket.transform_data(&polymarket_query, polymarket_fixture)?;
    println!("polymarket rows: {}", polymarket_rows.len());
    for row in &polymarket_rows {
        println!("  {} p={} title={}", row.id, row.probability, row.title);
    }

    Ok(())
}
