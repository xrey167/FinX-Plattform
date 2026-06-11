//! Example 10 — hand-written single-widget Workspace data backend.
//!
//! Boots the hand-rolled HTTP server, fetches its two documents over the
//! loopback, prints them, and exits. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-10-backend-minimal`.

use tdw_workspace_examples::{client, minimal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = minimal::serve().await?;
    println!("hand-written backend listening on {}", server.base_url());

    let widgets = client::get(server.addr(), "/widgets.json", "").await?;
    println!("\nGET /widgets.json ({}):", widgets.status);
    println!("{}", serde_json::to_string_pretty(&widgets.json())?);

    let data = client::get(server.addr(), minimal::DATA_ENDPOINT, "").await?;
    println!("\nGET {} ({}):", minimal::DATA_ENDPOINT, data.status);
    println!("{}", serde_json::to_string_pretty(&data.json())?);

    server.shutdown().await;
    Ok(())
}
