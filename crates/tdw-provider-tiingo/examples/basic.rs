//! Offline Tiingo example: feed inline Tiingo-shaped fixtures through the real
//! `transform_data` path. No network access and no API token required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-tiingo --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_tiingo::{
    TiingoHistoricalQuery, TiingoHttpHistoricalFetcher, TiingoHttpNewsFetcher, TiingoNewsQuery,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Daily historical prices -> MarketDataBar ----------------------------
    let hist_fetcher = TiingoHttpHistoricalFetcher::default();
    let hist_query = TiingoHistoricalQuery::new("aapl")?;
    println!("normalised symbol = {}", hist_query.symbol);

    let prices_fixture = Bytes::from(
        serde_json::json!([
            {
                "date": "2024-01-02T00:00:00.000Z",
                "open": 187.15, "high": 188.44, "low": 183.89,
                "close": 185.64, "volume": 82_488_700.0_f64
            }
        ])
        .to_string()
        .into_bytes(),
    );
    let bars = hist_fetcher.transform_data(&hist_query, prices_fixture)?;
    for bar in &bars {
        println!("bar {} close={} venue={}", bar.ts, bar.close, bar.venue);
    }

    // --- News feed -> TiingoNewsArticle --------------------------------------
    let news_fetcher = TiingoHttpNewsFetcher::default();
    let news_query = TiingoNewsQuery::new(&["AAPL", "MSFT"])?;
    println!("news tickers = {:?}", news_query.tickers);

    let news_fixture = Bytes::from(
        serde_json::json!([
            {
                "id": 42_u64,
                "title": "Apple announces results",
                "publishedDate": "2024-01-25T20:30:00Z",
                "url": "https://example.com/article",
                "source": "example.com"
            }
        ])
        .to_string()
        .into_bytes(),
    );
    let articles = news_fetcher.transform_data(&news_query, news_fixture)?;
    for article in &articles {
        println!("news #{}: {}", article.id, article.title);
    }

    Ok(())
}
