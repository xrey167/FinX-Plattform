//! Example 60 — the stateless two-request `get_widget_data` round trip.
//!
//! Scripts both legs against the offline agent bridge and prints the SSE event
//! order for each. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-60-agent-widget-data`.

use tdw_workspace_examples::widget_data;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let round_trip = widget_data::run().await?;
    println!(
        "leg 1 ({}): events {:?}",
        round_trip.leg1.status,
        round_trip.leg1.sse_event_names()
    );
    println!(
        "leg 2 ({}): events {:?}",
        round_trip.leg2.status,
        round_trip.leg2.sse_event_names()
    );
    println!("\nleg 2 answer carried the folded rows back into context:");
    println!("{}", round_trip.leg2.sse_data());
    Ok(())
}
