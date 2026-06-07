//! Offline walkthrough of the Benzinga fetchers.
//!
//! Mirrors the crate's inline `transform_data` tests: it constructs each
//! fetcher, validates a query with `transform_query`, and decodes an inline
//! fixture via `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-benzinga --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_benzinga::{BenzingaEarningsHttpFetcher, BenzingaNewsHttpFetcher};

fn main() -> tdw_core::Result<()> {
    // --- News ------------------------------------------------------------
    let news_fixture = Bytes::from_static(
        br#"[
            {
                "id": "abc123",
                "title": "Apple Q1 Beats Estimates",
                "teaser": "Apple reported strong Q1 results.",
                "url": "https://example.benzinga.com/apple-q1",
                "publishedDate": "2024-01-25T09:30:00Z",
                "source": "Benzinga",
                "stocks": [{ "name": "AAPL" }]
            },
            {
                "id": "abc124",
                "title": "Apple Guidance Raised",
                "teaser": "Management raised full-year guidance.",
                "url": "https://example.benzinga.com/apple-guidance",
                "publishedDate": "2024-01-26T10:00:00Z",
                "source": "Benzinga",
                "stocks": [{ "name": "AAPL" }]
            }
        ]"#,
    );
    let news = BenzingaNewsHttpFetcher::default();
    let news_query =
        BenzingaNewsHttpFetcher::transform_query(json!({ "symbol": "aapl", "page_size": 5 }))?;
    let news_rows = news.transform_data(&news_query, news_fixture)?;
    println!("news rows: {}", news_rows.len());
    for item in &news_rows {
        println!(
            "  {} {} stocks={:?}",
            item.published_date, item.title, item.stocks
        );
    }

    // --- Earnings --------------------------------------------------------
    let earnings_fixture = Bytes::from_static(
        br#"{
            "earnings": [
                {
                    "id": "e1",
                    "date": "2024-01-25",
                    "ticker": "AAPL",
                    "eps": "2.18",
                    "epsEstimate": "2.10",
                    "revenue": "119575000000"
                }
            ]
        }"#,
    );
    let earnings = BenzingaEarningsHttpFetcher::default();
    let earnings_query = BenzingaEarningsHttpFetcher::transform_query(json!({
        "symbol": "aapl",
        "date_from": "2024-01-01",
        "date_to": "2024-12-31"
    }))?;
    let earnings_rows = earnings.transform_data(&earnings_query, earnings_fixture)?;
    println!("earnings rows: {}", earnings_rows.len());
    for item in &earnings_rows {
        println!(
            "  {} {} eps={} est={}",
            item.date, item.ticker, item.eps, item.eps_estimate
        );
    }

    Ok(())
}
