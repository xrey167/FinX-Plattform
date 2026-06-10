//! Assemble the full `widgets.json` document and the curated default app.
//!
//! [`catalog_widgets`] projects every `Fetch` route in
//! [`tdw_endpoint_catalog::catalog`] into a [`WidgetConfig`] via
//! [`crate::derive::derive_widget`], in the catalog's deterministic order.
//! `Compute` routes are excluded for v1 (they carry no provider fetcher) — a
//! documented follow-up.
//!
//! [`widgets_json`] renders the `id -> WidgetConfig` map `OpenBB` Workspace
//! ingests; [`default_app`] / [`apps_json`] supply one curated starter
//! dashboard.

use std::collections::BTreeMap;

use serde_json::Value;
use tdw_endpoint_catalog::{EndpointKind, catalog};

use crate::config::{AppConfig, AppTab, LayoutItem, McpServerRef, WidgetConfig};
use crate::derive::derive_widget;

/// Default MCP server URL the starter app points agents at (the loopback daemon
/// bind documented for the workspace backend).
const DEFAULT_MCP_URL: &str = "http://127.0.0.1:7878";

/// Derive a [`WidgetConfig`] for every `Fetch` route in the catalog, in the
/// catalog's deterministic declaration order.
///
/// `Compute` routes are skipped (no provider fetcher backs them yet), matching
/// the `OpenAPI` generator's `Fetch`-only projection. The order is stable, so
/// the assembled document is reproducible for the golden snapshot.
#[must_use]
pub fn catalog_widgets() -> Vec<WidgetConfig> {
    catalog()
        .iter()
        .filter(|entry| entry.kind == EndpointKind::Fetch)
        .map(derive_widget)
        .collect()
}

/// Render the full `widgets.json` document: a JSON object keyed by widget `id`.
///
/// Keys are inserted into a [`BTreeMap`], so the document serializes in sorted
/// `id` order regardless of catalog order — deterministic and diff-friendly.
#[must_use]
pub fn widgets_json() -> Value {
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    for widget in catalog_widgets() {
        let id = widget.id.clone();
        if let Ok(value) = serde_json::to_value(&widget) {
            map.insert(id, value);
        }
    }
    serde_json::to_value(map).unwrap_or(Value::Null)
}

/// The curated default Workspace app (`FinX` Market Overview): a few marquee
/// widgets laid out on one tab, plus starter prompts and the `tdw-mcp` server.
#[must_use]
pub fn default_app() -> AppConfig {
    let layout = vec![
        layout_item("equity_price_historical", 0, 0, 20, 12),
        layout_item("equity_price_quote", 20, 0, 20, 12),
        layout_item("fixedincome_government_yield_curve", 0, 12, 20, 12),
        layout_item("economy_cpi", 20, 12, 20, 12),
    ];
    let mut tabs = BTreeMap::new();
    tabs.insert(
        "overview".to_string(),
        AppTab {
            id: "overview".to_string(),
            name: "Overview".to_string(),
            layout,
        },
    );
    AppConfig {
        name: "FinX Market Overview".to_string(),
        description: "A starter market dashboard: equity history and quote, the US Treasury yield curve, and CPI.".to_string(),
        img: None,
        tabs,
        prompts: vec![
            "Show the price history for AAPL.".to_string(),
            "What is the latest quote for MSFT?".to_string(),
            "Summarize the current US Treasury yield curve.".to_string(),
        ],
        mcp_servers: vec![McpServerRef {
            name: "tdw-mcp".to_string(),
            url: DEFAULT_MCP_URL.to_string(),
        }],
    }
}

/// Render the `apps.json` document: a JSON object keyed by app name, holding the
/// curated default app.
#[must_use]
pub fn apps_json() -> Value {
    let app = default_app();
    let name = app.name.clone();
    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    if let Ok(value) = serde_json::to_value(&app) {
        map.insert(name, value);
    }
    serde_json::to_value(map).unwrap_or(Value::Null)
}

/// Build one [`LayoutItem`] for the default app's grid.
fn layout_item(id: &str, x: u32, y: u32, w: u32, h: u32) -> LayoutItem {
    LayoutItem {
        i: id.to_string(),
        x,
        y,
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_widgets_cover_every_fetch_route() {
        let fetch_routes = catalog()
            .iter()
            .filter(|entry| entry.kind == EndpointKind::Fetch)
            .count();
        let widgets = catalog_widgets();
        assert_eq!(
            widgets.len(),
            fetch_routes,
            "one widget per Fetch route, Compute routes excluded"
        );
        assert!(fetch_routes > 0, "the catalog has Fetch routes to project");
    }

    #[test]
    fn widgets_json_contains_the_equity_historical_widget() {
        let document = widgets_json();
        let widget = &document["equity_price_historical"];
        assert!(widget.is_object(), "equity historical widget present");
        assert_eq!(widget["type"], "chart");
        assert_eq!(widget["endpoint"], "/widget-data/equity/price/historical");
        assert_eq!(widget["data"]["dataKey"], "results");
    }

    #[test]
    fn widget_ids_are_unique() {
        let widgets = catalog_widgets();
        let mut ids: Vec<&str> = widgets.iter().map(|widget| widget.id.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "every derived widget id is unique");
    }

    #[test]
    fn derivation_is_deterministic() {
        assert_eq!(widgets_json(), widgets_json());
        assert_eq!(apps_json(), apps_json());
    }

    #[test]
    fn default_app_references_catalog_widget_ids() {
        let widgets = catalog_widgets();
        let known: std::collections::BTreeSet<&str> =
            widgets.iter().map(|widget| widget.id.as_str()).collect();
        let app = default_app();
        for item in &app.tabs["overview"].layout {
            assert!(
                known.contains(item.i.as_str()),
                "app layout references a real widget id: {}",
                item.i
            );
        }
        assert_eq!(app.mcp_servers[0].name, "tdw-mcp");
    }
}
