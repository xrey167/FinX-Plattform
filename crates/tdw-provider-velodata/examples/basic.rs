//! Offline Velodata example: feed an inline Velo-shaped fixture through the
//! real `transform_data` path. No network access and no API key required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-velodata --example basic --features http
//! ```

use bytes::Bytes;
use tdw_core::Fetcher;
use tdw_provider_velodata::{VelodataFundingQuery, VelodataHttpFundingFetcher};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = VelodataHttpFundingFetcher::default();
    let query = VelodataFundingQuery::new("Binance", "BTCUSDT", 100)?;
    println!("exchange={} symbol={}", query.exchange, query.symbol);

    // Velo `/funding/rates` returns a JSON array of samples.
    let fixture = Bytes::from(
        serde_json::json!([
            {
                "timestamp": 1_700_000_000_000_i64,
                "exchange": "binance",
                "symbol": "BTCUSDT",
                "fundingRate": 0.0001,
                "fundingRateAnnualized": 0.1095
            },
            {
                "timestamp": 1_700_000_028_800_000_i64,
                "exchange": "binance",
                "symbol": "BTCUSDT",
                "fundingRate": 0.00012,
                "fundingRateAnnualized": 0.1314
            }
        ])
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, fixture)?;
    println!("funding samples: {}", rows.len());
    for r in &rows {
        println!(
            "  ts={} rate={} annualized={}",
            r.timestamp, r.funding_rate, r.funding_rate_annualized
        );
    }

    Ok(())
}
