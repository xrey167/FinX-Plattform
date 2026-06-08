//! Offline Seeking Alpha example: feed inline RapidAPI-shaped fixtures through
//! the real `transform_data` path. No network access and no RapidAPI key
//! required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-seeking-alpha --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_seeking_alpha::{
    SeekingAlphaArticlesHttpFetcher, SeekingAlphaArticlesQuery, SeekingAlphaRatingsHttpFetcher,
    SeekingAlphaRatingsQuery,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Articles: GET /analysis/v2/list shape -------------------------------
    let articles_fetcher = SeekingAlphaArticlesHttpFetcher::default();
    let articles_query = SeekingAlphaArticlesQuery::new("aapl", 5)?;
    println!("articles ticker = {}", articles_query.ticker);

    let articles_fixture = Bytes::from_static(
        br#"{
            "data": [
                {
                    "id": "abc123",
                    "type": "article",
                    "attributes": {
                        "title": "Apple Q1 2024 Earnings: Strong Beat",
                        "publishOn": "2024-01-25T20:30:00Z",
                        "isLockedPro": false,
                        "commentCount": 45
                    }
                }
            ]
        }"#,
    );
    let articles = articles_fetcher.transform_data(&articles_query, articles_fixture)?;
    for a in &articles {
        println!(
            "article {}: {} ({} comments)",
            a.id, a.title, a.comment_count
        );
    }

    // --- Ratings: GET /symbols/v1/summary shape ------------------------------
    let ratings_fetcher = SeekingAlphaRatingsHttpFetcher::default();
    let ratings_query = SeekingAlphaRatingsQuery::new("aapl")?;

    let ratings_fixture = Bytes::from_static(
        br#"{
            "data": [
                {
                    "id": "AAPL",
                    "type": "ticker",
                    "attributes": {
                        "quant_rating": 4.12,
                        "authors_rating": 3.85,
                        "sell_side_rating": 4.21,
                        "quant_rating_change": "up"
                    }
                }
            ]
        }"#,
    );
    let ratings = ratings_fetcher.transform_data(&ratings_query, ratings_fixture)?;
    for r in &ratings {
        println!(
            "{} quant={} authors={} sell-side={}",
            r.ticker, r.quant_rating, r.authors_rating, r.sell_side_rating
        );
    }

    Ok(())
}
