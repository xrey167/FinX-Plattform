//! Offline walkthrough of the BLS time-series fetcher.
//!
//! Mirrors the crate's cassette test: it constructs the fetcher, validates a
//! query with `transform_query`, and decodes an inline BLS response envelope via
//! `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-bls --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_bls::BlsHttpTimeSeriesFetcher;

fn main() -> tdw_core::Result<()> {
    let fetcher = BlsHttpTimeSeriesFetcher::default();

    let query = BlsHttpTimeSeriesFetcher::transform_query(json!({
        "series_ids": ["CUUR0000SA0"],
        "start_year": 2024,
        "end_year": 2024
    }))?;

    // Recorded BLS v2 response shape.
    let fixture = Bytes::from(
        json!({
            "status": "REQUEST_SUCCEEDED",
            "responseTime": 123,
            "Results": {
                "series": [{
                    "seriesID": "CUUR0000SA0",
                    "data": [
                        { "year": "2024", "period": "M01", "periodName": "January", "value": "308.417", "footnotes": [{}] }
                    ]
                }]
            }
        })
        .to_string()
        .into_bytes(),
    );

    let rows = fetcher.transform_data(&query, fixture)?;
    println!("decoded {} data points", rows.len());
    for point in &rows {
        println!(
            "  {} {}/{} ({}) = {}",
            point.series_id, point.year, point.period, point.period_name, point.value
        );
    }

    Ok(())
}
