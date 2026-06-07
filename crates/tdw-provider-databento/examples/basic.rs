//! Offline walkthrough of the Databento fetchers.
//!
//! Mirrors the crate's cassette tests: it constructs each fetcher, validates a
//! query with `transform_query`, and decodes an inline response envelope via
//! `transform_data`. No network access and no API key are required.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-databento --features http --example basic
//! ```

use bytes::Bytes;
use serde_json::json;
use tdw_core::Fetcher;
use tdw_provider_databento::DatabentoHttpTimeseriesFetcher;
use tdw_provider_databento::http_fetcher::DatabentoMetadataFetcher;

fn main() -> tdw_core::Result<()> {
    // --- Timeseries OHLCV ------------------------------------------------
    let ts = DatabentoHttpTimeseriesFetcher::default();
    let ts_query = DatabentoHttpTimeseriesFetcher::transform_query(json!({
        "dataset": "GLBX.MDP3",
        "symbols": ["ESH5"],
        "start": "2024-01-01",
        "end": "2024-01-31"
    }))?;
    let ts_fixture = Bytes::from(
        json!({
            "records": [
                { "ts_event": 1_704_153_600_000_000_000_i64, "open": 4800.0, "high": 4825.0, "low": 4790.0, "close": 4815.0, "volume": 125000.0 }
            ]
        })
        .to_string()
        .into_bytes(),
    );
    let bars = ts.transform_data(&ts_query, ts_fixture)?;
    println!("timeseries bars: {}", bars.len());
    for bar in &bars {
        println!(
            "  {} {} ({}) close={}",
            bar.ts, bar.symbol, bar.venue, bar.close
        );
    }

    // --- Dataset metadata ------------------------------------------------
    let meta = DatabentoMetadataFetcher::default();
    let meta_query = DatabentoMetadataFetcher::transform_query(json!({}))?;
    let meta_fixture = Bytes::from(
        json!({ "result": ["GLBX.MDP3", "XNAS.ITCH", "DBEQ.BASIC"] })
            .to_string()
            .into_bytes(),
    );
    let datasets = meta.transform_data(&meta_query, meta_fixture)?;
    println!("datasets: {}", datasets.len());
    for d in &datasets {
        println!("  {}", d.id);
    }

    Ok(())
}
