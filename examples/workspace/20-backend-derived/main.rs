//! Example 20 — the catalog-derived Workspace data backend.
//!
//! Boots the in-memory daemon's workspace surface, fetches the derived
//! `widgets.json` and the offline equity-historical widget data, and prints a
//! summary. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-20-backend-derived`.

use tdw_workspace_examples::derived;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (document, count) = derived::fetch_widgets_json().await?;
    println!("GET /widgets.json derived {count} widgets from the catalog");
    if let Some(widget) = document.get("equity_price_historical") {
        println!(
            "\nequity_price_historical:\n{}",
            serde_json::to_string_pretty(widget)?
        );
    }

    let data = derived::fetch_equity_historical().await?;
    let rows = data.get("results").and_then(|r| r.as_array());
    println!(
        "\nGET {} -> {} offline rows (fileset fixture)",
        derived::HISTORICAL_TARGET,
        rows.map_or(0, Vec::len)
    );
    if let Some(first) = rows.and_then(|r| r.first()) {
        println!("first row: {}", serde_json::to_string(first)?);
    }
    Ok(())
}
