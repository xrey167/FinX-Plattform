//! Example 40 — the minimal copilot: `agents.json` + a streamed answer.
//!
//! Boots the offline agent bridge, prints its `agents.json`, asks a no-widget
//! question, and prints the SSE event order. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-40-agent-echo`.

use tdw_workspace_examples::agent_echo;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = agent_echo::fetch_agents_json().await?;
    println!("GET /agents.json:");
    println!("{}", serde_json::to_string_pretty(&manifest)?);

    let response = agent_echo::ask("What is a P/E ratio?").await?;
    println!("\nPOST /v1/query ({}):", response.status);
    println!("SSE events: {:?}", response.sse_event_names());
    Ok(())
}
