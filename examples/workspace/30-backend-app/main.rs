//! Example 30 — `apps.json`: the curated default app plus a custom app.
//!
//! Prints the curated `apps.json` the backend serves and the hand-built custom
//! "Rates & Macro Board" app, then the merged document. Run with:
//! `cargo run -p tdw-workspace-examples --bin ws-30-backend-app`.

use tdw_workspace_examples::app;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let curated = app::fetch_curated_apps_json().await?;
    println!("GET /apps.json (curated default app):");
    println!("{}", serde_json::to_string_pretty(&curated)?);

    println!("\nCustom app definition (Rates & Macro Board):");
    println!("{}", serde_json::to_string_pretty(&app::custom_app())?);

    println!("\nMerged apps.json (both apps, what a backend serving both returns):");
    let names: Vec<String> = app::apps_document()
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();
    println!("apps: {names:?}");
    Ok(())
}
