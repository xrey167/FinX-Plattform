//! Example 70 — table + chart SSE artifacts from widget data.
//!
//! Builds the table + chart artifacts from a set of OHLCV bars, prints the SSE
//! transcript, and the reusable Plotly candlestick figure-spec. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-70-agent-charts-tables`.

use tdw_workspace_examples::charts_tables;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "SSE artifacts transcript:\n{}",
        charts_tables::sse_transcript()
    );
    println!(
        "Plotly candlestick figure-spec:\n{}",
        serde_json::to_string_pretty(&charts_tables::plotly_candlestick())?
    );
    Ok(())
}
