//! Example 50 — reasoning steps + citations to a dashboard widget.
//!
//! Drives the pure copilot sequencer over the offline stub on a grounded
//! conversation and prints the resulting SSE transcript. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-50-agent-reasoning-citations`.

use tdw_workspace_examples::reasoning_citations;

fn main() {
    let events = reasoning_citations::answered_turn();
    println!("copilot turn emitted {} events:", events.len());
    for event in &events {
        println!("  - {}", event.event_name());
    }
    println!(
        "\nSSE transcript:\n{}",
        reasoning_citations::sse_transcript()
    );
}
