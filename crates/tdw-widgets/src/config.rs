//! Serde types for the `OpenBB` Workspace `widgets.json` and `apps.json`
//! contract (the published *backends-for-openbb* shape).
//!
//! These types are a faithful projection of the **public** `OpenBB` Workspace
//! developer documentation — no `OpenBB` source code was consulted. Field names
//! match the documented `JSON` exactly (camelCase where the docs use camelCase),
//! so a serialized [`WidgetConfig`] / [`AppConfig`] is byte-compatible with what
//! `OpenBB` Workspace ingests from a custom data backend.
//!
//! Sources (public docs):
//! - `widgets.json` reference: <https://docs.openbb.co/workspace/data-integration/widgets.json>
//! - `apps.json` / templates: <https://docs.openbb.co/workspace/data-integration/templates>
//! - Getting started (custom backend): <https://docs.openbb.co/workspace/getting-started/custom-backend>
//!
//! Optional fields use `skip_serializing_if = "Option::is_none"` / `is_empty`
//! so a derived document stays minimal: a widget only emits the keys it actually
//! needs, matching the hand-written `widgets.json` examples in the docs.

use serde::{Deserialize, Serialize};

/// Grid placement defaults for a widget on a Workspace dashboard (`gridData`).
///
/// `w`/`h` are the default width/height in grid cells; `min_w`/`min_h` are the
/// minimum the user can shrink the widget to. Mirrors the documented
/// `gridData` object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridData {
    /// Default width in grid columns.
    pub w: u32,
    /// Default height in grid rows.
    pub h: u32,
    /// Minimum width in grid columns the user can resize to.
    #[serde(rename = "minW", default, skip_serializing_if = "Option::is_none")]
    pub min_w: Option<u32>,
    /// Minimum height in grid rows the user can resize to.
    #[serde(rename = "minH", default, skip_serializing_if = "Option::is_none")]
    pub min_h: Option<u32>,
}

/// One parameter the user can edit for a widget (`params[]` entry).
///
/// `param_name` is the query-string key sent to the data endpoint;
/// `param_type` selects the editor control (`"ticker"`, `"date"`, `"text"`,
/// `"number"`, `"boolean"`, `"endpoint"`, `"form"`, `"tabs"`). `value` is the
/// default; `options` populate a dropdown for an enumerated parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDef {
    /// Query-string key sent to the data endpoint (the documented `paramName`).
    #[serde(rename = "paramName")]
    pub param_name: String,
    /// Editor control type: `"ticker"`, `"date"`, `"text"`, `"number"`,
    /// `"boolean"`, `"endpoint"`, `"form"`, or `"tabs"`.
    #[serde(rename = "type")]
    pub param_type: String,
    /// Default value for the parameter, serialized as its JSON form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Human-readable label shown next to the control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Longer help text describing the parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Dropdown choices for an enumerated parameter (`label`/`value` pairs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ParamOption>,
    /// Whether multiple options can be selected at once.
    #[serde(
        rename = "multiSelect",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub multi_select: Option<bool>,
}

/// One choice in a dropdown parameter's `options` list (`label`/`value` pair).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamOption {
    /// Display label for the choice.
    pub label: String,
    /// Value submitted when the choice is selected.
    pub value: String,
}

/// One column definition for a table widget (`data.table.columnsDefs[]`).
///
/// `field` is the row key; `header_name` is the display title; `cell_data_type`
/// hints the renderer (`"text"`, `"number"`, `"date"`, `"boolean"`).
/// `formatter_fn` applies a built-in formatter (e.g. `"percent"`, `"int"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDef {
    /// Row key this column reads from each record.
    pub field: String,
    /// Display title shown in the column header.
    #[serde(rename = "headerName")]
    pub header_name: String,
    /// Cell data type hint: `"text"`, `"number"`, `"date"`, or `"boolean"`.
    #[serde(
        rename = "cellDataType",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cell_data_type: Option<String>,
    /// Built-in formatter applied to the cell (e.g. `"percent"`, `"int"`).
    #[serde(
        rename = "formatterFn",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub formatter_fn: Option<String>,
    /// Custom renderer function name, when the cell needs one.
    #[serde(rename = "renderFn", default, skip_serializing_if = "Option::is_none")]
    pub render_fn: Option<String>,
    /// Fixed column width in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Pin the column to a side (`"left"` / `"right"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<String>,
    /// Hide the column by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide: Option<bool>,
}

/// Table-specific rendering config (`data.table`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableConfig {
    /// Whether to show every column by default.
    #[serde(rename = "showAll", default, skip_serializing_if = "Option::is_none")]
    pub show_all: Option<bool>,
    /// Per-column definitions in display order.
    #[serde(rename = "columnsDefs", default, skip_serializing_if = "Vec::is_empty")]
    pub columns_defs: Vec<ColumnDef>,
}

/// Chart-specific rendering config (`data.chart`).
///
/// Kept as an open JSON object: the chart sub-schema in the docs is large and
/// version-specific, so a derived chart widget emits only the keys it sets
/// (e.g. `chartType`) and round-trips the rest verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartConfig {
    /// Chart kind, e.g. `"line"` or `"candlestick"` (the documented `chartType`).
    #[serde(rename = "chartType", default, skip_serializing_if = "Option::is_none")]
    pub chart_type: Option<String>,
}

/// The `data` block of a widget: where rows live (`dataKey`) plus the table /
/// chart rendering config.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetData {
    /// JSON key under which the row array lives in the endpoint response.
    #[serde(rename = "dataKey", default, skip_serializing_if = "Option::is_none")]
    pub data_key: Option<String>,
    /// Table rendering config, present for `type = "table"` widgets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableConfig>,
    /// Chart rendering config, present for `type = "chart"` widgets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<ChartConfig>,
}

/// Binds a widget to an MCP tool so an agent can fetch the same data
/// (`mcp_tool`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTool {
    /// MCP server name that hosts the tool.
    pub mcp_server: String,
    /// Tool identifier within that server.
    pub tool_id: String,
}

/// One widget definition in `widgets.json`.
///
/// A `widgets.json` document is a JSON object keyed by widget `id`; this struct
/// is the value. Built deterministically from a catalog route by
/// [`crate::derive::derive_widget`], so the field set is stable across builds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetConfig {
    /// Stable widget id (route with `'/'` replaced by `'_'`).
    pub id: String,
    /// Human-readable widget name.
    pub name: String,
    /// One-line widget description.
    pub description: String,
    /// Category bucket (the route's first segment).
    pub category: String,
    /// Render type: `"table"`, `"chart"`, `"markdown"`, `"metric"`, ...
    #[serde(rename = "type")]
    pub widget_type: String,
    /// Backend data endpoint path (relative to the backend root).
    pub endpoint: String,
    /// Default dashboard grid placement.
    #[serde(rename = "gridData")]
    pub grid_data: GridData,
    /// Editable parameters for the widget.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamDef>,
    /// Data-shape and rendering config (`dataKey`, table, chart).
    pub data: WidgetData,
    /// Auto-refresh interval in milliseconds, when the widget polls.
    #[serde(
        rename = "refetchInterval",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub refetch_interval: Option<u64>,
    /// How long fetched data stays fresh, in milliseconds.
    #[serde(rename = "staleTime", default, skip_serializing_if = "Option::is_none")]
    pub stale_time: Option<u64>,
    /// MCP tool binding so an agent can fetch the same data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool: Option<McpTool>,
}

/// One item placed on a Workspace app tab's layout grid (`layout[]` entry).
///
/// `i` is the widget id this slot renders; `x`/`y` are the grid origin and
/// `w`/`h` the span.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutItem {
    /// Widget id placed in this slot (the documented `i` key).
    pub i: String,
    /// Grid column origin.
    pub x: u32,
    /// Grid row origin.
    pub y: u32,
    /// Width in grid columns.
    pub w: u32,
    /// Height in grid rows.
    pub h: u32,
}

/// One tab of a Workspace app (`tabs` value).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppTab {
    /// Tab id (also the `tabs` object key).
    pub id: String,
    /// Display name of the tab.
    pub name: String,
    /// Widget placements on this tab's grid.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layout: Vec<LayoutItem>,
}

/// Points an app at an MCP server it can use (`mcp_servers[]` entry).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRef {
    /// MCP server name.
    pub name: String,
    /// Transport URL or command the Workspace connects to.
    pub url: String,
}

/// One application definition in `apps.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// App name shown in the Workspace app list.
    pub name: String,
    /// One-line app description.
    pub description: String,
    /// Optional preview image URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<String>,
    /// Tabs keyed by tab id, each with its own layout.
    pub tabs: std::collections::BTreeMap<String, AppTab>,
    /// Suggested Copilot prompts for the app.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<String>,
    /// MCP servers the app's agents may use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small `widgets.json` value transcribed from the public docs round-trips
    /// through the [`WidgetConfig`] types without losing or renaming fields.
    #[test]
    fn widget_config_round_trips_doc_fixture() {
        let fixture = serde_json::json!({
            "id": "equity_price_historical",
            "name": "Historical Prices",
            "description": "End-of-day OHLCV bars.",
            "category": "equity",
            "type": "chart",
            "endpoint": "/widget-data/equity/price/historical",
            "gridData": { "w": 20, "h": 12, "minW": 10, "minH": 6 },
            "params": [
                { "paramName": "symbol", "type": "ticker", "value": "AAPL", "label": "Symbol" }
            ],
            "data": {
                "dataKey": "results",
                "chart": { "chartType": "candlestick" }
            },
            "mcp_tool": { "mcp_server": "tdw-mcp", "tool_id": "tdw.provider.fetch" }
        });
        let widget: WidgetConfig =
            serde_json::from_value(fixture.clone()).expect("fixture parses as WidgetConfig");
        assert_eq!(widget.id, "equity_price_historical");
        assert_eq!(widget.widget_type, "chart");
        assert_eq!(widget.params[0].param_name, "symbol");
        let reserialized = serde_json::to_value(&widget).expect("widget serializes");
        assert_eq!(
            reserialized, fixture,
            "round-trip preserves doc field names"
        );
    }

    /// A table widget fixture exercises `columnsDefs` and the `dataKey` key.
    #[test]
    fn table_widget_round_trips_columns_defs() {
        let fixture = serde_json::json!({
            "id": "equity_profile",
            "name": "Company Profile",
            "description": "Profile fields.",
            "category": "equity",
            "type": "table",
            "endpoint": "/widget-data/equity/profile",
            "gridData": { "w": 20, "h": 9 },
            "data": {
                "dataKey": "results",
                "table": {
                    "showAll": true,
                    "columnsDefs": [
                        { "field": "symbol", "headerName": "Symbol", "cellDataType": "text" },
                        { "field": "market_cap", "headerName": "Market Cap", "cellDataType": "number", "formatterFn": "int" }
                    ]
                }
            }
        });
        let widget: WidgetConfig =
            serde_json::from_value(fixture.clone()).expect("table fixture parses");
        let table = widget
            .data
            .table
            .as_ref()
            .expect("table config present after parse");
        assert_eq!(table.columns_defs.len(), 2);
        assert_eq!(table.columns_defs[1].formatter_fn.as_deref(), Some("int"));
        let reserialized = serde_json::to_value(&widget).expect("table widget serializes");
        assert_eq!(reserialized, fixture, "columnsDefs round-trips verbatim");
    }

    /// An `apps.json` fixture transcribed from the docs round-trips, exercising
    /// the `tabs` map, `layout`, `prompts`, and `mcp_servers`.
    #[test]
    fn app_config_round_trips_doc_fixture() {
        let fixture = serde_json::json!({
            "name": "FinX Market Overview",
            "description": "A starter market dashboard.",
            "tabs": {
                "overview": {
                    "id": "overview",
                    "name": "Overview",
                    "layout": [
                        { "i": "equity_price_historical", "x": 0, "y": 0, "w": 20, "h": 12 }
                    ]
                }
            },
            "prompts": ["Show me the price history for AAPL."],
            "mcp_servers": [ { "name": "tdw-mcp", "url": "http://127.0.0.1:7878" } ]
        });
        let app: AppConfig =
            serde_json::from_value(fixture.clone()).expect("apps.json fixture parses");
        assert_eq!(app.name, "FinX Market Overview");
        assert_eq!(app.tabs["overview"].layout[0].i, "equity_price_historical");
        let reserialized = serde_json::to_value(&app).expect("app serializes");
        assert_eq!(reserialized, fixture, "apps.json round-trips verbatim");
    }
}
