//! Example 30 — `apps.json`: the curated `FinX` Market Overview app plus a
//! custom app definition.
//!
//! An app is a saved dashboard: named tabs, each a grid of widget placements,
//! plus starter prompts and the MCP servers its copilot may use. This example
//! serves the curated default app the backend ships ([`tdw_widgets::apps_json`])
//! and also *builds a second, custom app by hand* — a two-tab "Rates & Macro"
//! board — to show how to compose one from the derived widget ids.
//!
//! [`custom_app`] returns the hand-built [`AppConfig`]; [`apps_document`] merges
//! it with the curated default into one `apps.json` object so a backend could
//! serve both. [`fetch_curated_apps_json`] boots the real backend and fetches
//! the document it actually serves.

use std::collections::BTreeMap;

use serde_json::Value;
use tdw_app_server::WorkspaceConfig;
use tdw_widgets::{
    AppConfig, AppTab, LayoutItem, McpServerRef, apps_json as curated_apps_json, default_app,
};

use crate::client;
use crate::server::{RunningServer, start_workspace};

/// Build a custom two-tab "Rates & Macro" app over derived widget ids.
///
/// Tab one lays out the Treasury yield curve next to CPI; tab two pairs an
/// equity quote with its price history — showing how a parameter-sharing board
/// (every widget keyed by the same `symbol`) is composed. Widget ids are the
/// derived `widgets.json` keys (route with `'/'` replaced by `'_'`).
#[must_use]
pub fn custom_app() -> AppConfig {
    let mut tabs = BTreeMap::new();
    tabs.insert(
        "macro".to_string(),
        AppTab {
            id: "macro".to_string(),
            name: "Rates & Macro".to_string(),
            layout: vec![
                layout("fixedincome_government_yield_curve", 0, 0, 20, 12),
                layout("economy_cpi", 20, 0, 20, 12),
            ],
        },
    );
    tabs.insert(
        "equity".to_string(),
        AppTab {
            id: "equity".to_string(),
            name: "Equity".to_string(),
            layout: vec![
                layout("equity_price_quote", 0, 0, 20, 9),
                layout("equity_price_historical", 0, 9, 40, 12),
            ],
        },
    );
    AppConfig {
        name: "Rates & Macro Board".to_string(),
        description: "A custom two-tab board: the Treasury curve and CPI, plus an equity quote and its history.".to_string(),
        img: None,
        tabs,
        prompts: vec![
            "Summarize the current US Treasury yield curve.".to_string(),
            "Is CPI trending up or down?".to_string(),
        ],
        mcp_servers: vec![McpServerRef {
            name: "tdw-mcp".to_string(),
            url: "http://127.0.0.1:7878".to_string(),
        }],
    }
}

/// Merge the curated default app with the custom app into one `apps.json`
/// object keyed by app name — what a backend serving both would return.
#[must_use]
pub fn apps_document() -> Value {
    let mut map = BTreeMap::new();
    let curated = default_app();
    map.insert(
        curated.name.clone(),
        serde_json::to_value(&curated).unwrap_or(Value::Null),
    );
    let custom = custom_app();
    map.insert(
        custom.name.clone(),
        serde_json::to_value(&custom).unwrap_or(Value::Null),
    );
    serde_json::to_value(map).unwrap_or(Value::Null)
}

/// The curated `apps.json` document the backend ships (the single default app).
#[must_use]
pub fn curated_document() -> Value {
    curated_apps_json()
}

/// Boot the workspace backend and fetch the `apps.json` it actually serves.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_curated_apps_json() -> std::io::Result<Value> {
    let server = start().await?;
    let response = client::get(server.addr(), "/apps.json", "").await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Boot the workspace backend (default config) on an ephemeral port.
///
/// # Errors
///
/// Returns any error binding the listener.
pub async fn start() -> std::io::Result<RunningServer> {
    start_workspace(WorkspaceConfig::default()).await
}

/// Build one grid placement for a custom app tab.
fn layout(id: &str, x: u32, y: u32, w: u32, h: u32) -> LayoutItem {
    LayoutItem {
        i: id.to_string(),
        x,
        y,
        w,
        h,
    }
}
