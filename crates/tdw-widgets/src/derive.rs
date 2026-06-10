//! Derivation engine: project a [`CatalogEntry`] into a [`WidgetConfig`].
//!
//! The typed endpoint catalog stays the single source of truth; this module is
//! a pure projection of one catalog route into the `OpenBB` Workspace widget
//! shape. It reads the route's `params_schema` and `model` schemas as `JSON`
//! (the same `schemars`-emitted `Value` shape `xtask`'s `OpenAPI` generator
//! consumes), then maps:
//!
//! - JSON-Schema params (`start_date`/`end_date` -> `date`, integer/number ->
//!   `number`, boolean -> `boolean`, `enum` -> dropdown, string fallback ->
//!   `text`), plus a synthesized `symbol` `ticker` param for symbol-bearing
//!   routes (the standard params schema carries date/interval/limit but not the
//!   provider-read `symbol`).
//! - model schema properties -> `columnsDefs` (`field` = property, `headerName`
//!   = Title Case, `cellDataType` from the JSON type, `formatterFn` heuristics
//!   on field names).
//!
//! `type` is `"chart"` when `entry.chartable`, else `"table"`. The `endpoint`
//! is `"/widget-data/" + route`, `category` is the first route segment, `id` is
//! the route with `'/'` -> `'_'`, and every widget binds the `tdw-mcp`
//! `tdw.provider.fetch` tool.

use schemars::Schema;
use serde_json::Value;
use tdw_endpoint_catalog::CatalogEntry;

use crate::config::{
    ChartConfig, ColumnDef, GridData, McpTool, ParamDef, ParamOption, TableConfig, WidgetConfig,
    WidgetData,
};

/// JSON key under which the daemon's `ResultEnvelope` carries the row array.
///
/// The catalog REST / widget-data path returns the `ResultEnvelope` directly,
/// whose rows live under `results` (see the daemon's `ResultEnvelope` and the
/// REST e2e fixture). The derived widgets therefore set `dataKey = "results"`
/// so Workspace reads rows from the same key the backend emits.
pub const DATA_KEY: &str = "results";

/// MCP server name every derived widget binds its fetch tool to.
const MCP_SERVER: &str = "tdw-mcp";
/// MCP tool id every derived widget binds (the generic catalog fetch tool).
const MCP_TOOL_ID: &str = "tdw.provider.fetch";

/// Derive the `OpenBB` Workspace [`WidgetConfig`] for a single catalog route.
///
/// Pure and deterministic: the same `entry` always yields the same widget, so
/// the assembled `widgets.json` is reproducible (the golden-snapshot invariant).
#[must_use]
pub fn derive_widget(entry: &CatalogEntry) -> WidgetConfig {
    let route = entry.route;
    let model = schema_value(entry.model);
    let params_schema = schema_value(entry.params_schema);

    let widget_type = if entry.chartable { "chart" } else { "table" };
    let id = route.replace('/', "_");
    let category = route.split('/').next().unwrap_or(route).to_string();
    let (name, description) =
        curated_label(route).unwrap_or_else(|| (default_name(route), entry.doc.to_string()));

    WidgetConfig {
        id,
        name,
        description,
        category,
        widget_type: widget_type.to_string(),
        endpoint: format!("/widget-data/{route}"),
        grid_data: grid_for(widget_type),
        params: derive_params(route, &model, &params_schema),
        data: derive_data(widget_type, &model),
        refetch_interval: None,
        stale_time: Some(60_000),
        mcp_tool: Some(McpTool {
            mcp_server: MCP_SERVER.to_string(),
            tool_id: MCP_TOOL_ID.to_string(),
        }),
    }
}

/// Serialize a `schemars` schema thunk into its JSON `Value` form (an object
/// with `properties`/`type`/...), mirroring the `OpenAPI` generator.
fn schema_value(thunk: fn() -> Schema) -> Value {
    serde_json::to_value(thunk()).unwrap_or(Value::Null)
}

/// Default grid placement per widget type (charts are taller than tables).
fn grid_for(widget_type: &str) -> GridData {
    if widget_type == "chart" {
        GridData {
            w: 20,
            h: 12,
            min_w: Some(10),
            min_h: Some(6),
        }
    } else {
        GridData {
            w: 20,
            h: 9,
            min_w: Some(8),
            min_h: Some(4),
        }
    }
}

/// Build the `data` block: a chart config for chartable routes, else a table
/// config with `columnsDefs` derived from the model schema.
fn derive_data(widget_type: &str, model: &Value) -> WidgetData {
    if widget_type == "chart" {
        WidgetData {
            data_key: Some(DATA_KEY.to_string()),
            table: None,
            chart: Some(ChartConfig {
                chart_type: Some("candlestick".to_string()),
            }),
        }
    } else {
        WidgetData {
            data_key: Some(DATA_KEY.to_string()),
            table: Some(TableConfig {
                show_all: Some(true),
                columns_defs: columns_from_model(model),
            }),
            chart: None,
        }
    }
}

/// Whether a route's standard records are keyed by a `symbol` field, which
/// means the widget needs a `ticker` param the user edits.
fn model_has_symbol(model: &Value) -> bool {
    model
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|props| props.contains_key("symbol"))
}

/// Build the widget's editable params: a synthesized `symbol` ticker param for
/// symbol-bearing routes, then one param per standard-params property.
fn derive_params(route: &str, model: &Value, params_schema: &Value) -> Vec<ParamDef> {
    let mut params = Vec::new();
    if model_has_symbol(model) {
        params.push(symbol_param(route));
    }
    if let Some(properties) = params_schema.get("properties").and_then(Value::as_object) {
        // Sorted for deterministic output (BTreeMap-free: collect + sort keys).
        let mut names: Vec<&String> = properties.keys().collect();
        names.sort();
        for name in names {
            params.push(param_from_property(name, &properties[name]));
        }
    }
    params
}

/// The synthesized `symbol` ticker parameter, defaulting to `AAPL` so the
/// widget renders offline against the `fileset` fixture on first load.
fn symbol_param(route: &str) -> ParamDef {
    let default = if route.starts_with("crypto/") {
        "BTCUSD"
    } else {
        "AAPL"
    };
    ParamDef {
        param_name: "symbol".to_string(),
        param_type: "ticker".to_string(),
        value: Some(Value::String(default.to_string())),
        label: Some("Symbol".to_string()),
        description: Some("Ticker symbol to query.".to_string()),
        options: Vec::new(),
        multi_select: Some(false),
    }
}

/// Map one JSON-Schema property into a [`ParamDef`].
fn param_from_property(name: &str, schema: &Value) -> ParamDef {
    let param_type = param_type_for(name, schema);
    let options = enum_options(schema);
    let label = title_case(name);
    let description = schema
        .get("description")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    ParamDef {
        param_name: name.to_string(),
        param_type: param_type.to_string(),
        value: None,
        label: Some(label),
        description,
        options,
        multi_select: None,
    }
}

/// Decide the Workspace param control type for a JSON-Schema property.
fn param_type_for(name: &str, schema: &Value) -> &'static str {
    if name.contains("date") {
        return "date";
    }
    match json_type(schema) {
        Some("integer" | "number") => "number",
        Some("boolean") => "boolean",
        _ => "text",
    }
}

/// Extract dropdown `options` from a schema's `enum` list, if present.
fn enum_options(schema: &Value) -> Vec<ParamOption> {
    schema
        .get("enum")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .map(|value| ParamOption {
                    label: title_case(&value),
                    value,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Build `columnsDefs` from a model schema's top-level `properties`, in sorted
/// (deterministic) order.
fn columns_from_model(model: &Value) -> Vec<ColumnDef> {
    let Some(properties) = model.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut names: Vec<&String> = properties.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| column_from_property(name, &properties[name]))
        .collect()
}

/// Map one model property into a [`ColumnDef`].
fn column_from_property(name: &str, schema: &Value) -> ColumnDef {
    ColumnDef {
        field: name.to_string(),
        header_name: title_case(name),
        cell_data_type: Some(cell_data_type_for(schema).to_string()),
        formatter_fn: formatter_for(name),
        render_fn: None,
        width: None,
        pinned: None,
        hide: None,
    }
}

/// Choose a `cellDataType` from a property's JSON type.
fn cell_data_type_for(schema: &Value) -> &'static str {
    match json_type(schema) {
        Some("integer" | "number") => "number",
        Some("boolean") => "boolean",
        _ => "text",
    }
}

/// Heuristic `formatterFn` from a field name: percentage-ish fields format as
/// `"percent"`, count/volume-ish fields as `"int"`, everything else unset.
fn formatter_for(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower.contains("percent")
        || lower.contains("pct")
        || lower.ends_with("_change")
        || lower.contains("yield")
        || lower.contains("ratio")
    {
        Some("percent".to_string())
    } else if lower.contains("volume") || lower.contains("count") || lower.contains("shares") {
        Some("int".to_string())
    } else {
        None
    }
}

/// The JSON-Schema `type` of a property, tolerating schemars' `["string",
/// "null"]` array form (first non-null wins).
fn json_type(schema: &Value) -> Option<&str> {
    match schema.get("type") {
        Some(Value::String(single)) => Some(single.as_str()),
        Some(Value::Array(types)) => types
            .iter()
            .filter_map(Value::as_str)
            .find(|type_name| *type_name != "null"),
        _ => None,
    }
}

/// Title-case a `snake_case` identifier (`market_cap` -> `Market Cap`).
fn title_case(name: &str) -> String {
    name.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Default widget name from a route: the last segment, Title Cased.
fn default_name(route: &str) -> String {
    let last = route.rsplit('/').next().unwrap_or(route);
    title_case(last)
}

/// Optional curated `(name, description)` override for a handful of marquee
/// routes. A small `const`-style table — deliberately not a file-loading
/// system — gives the default app's widgets friendlier labels for v1.
fn curated_label(route: &str) -> Option<(String, String)> {
    let label = match route {
        "equity/price/historical" => (
            "Equity Historical Prices",
            "End-of-day OHLCV price history for an equity symbol.",
        ),
        "equity/price/quote" => (
            "Equity Quote",
            "Latest last-price quote snapshot for an equity symbol.",
        ),
        "fixedincome/government/yield_curve" => (
            "US Treasury Yield Curve",
            "Treasury par yields across maturities.",
        ),
        "economy/cpi" => ("Consumer Price Index", "FRED Consumer Price Index series."),
        _ => return None,
    };
    Some((label.0.to_string(), label.1.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_endpoint_catalog::{EndpointKind, lookup};

    #[test]
    fn equity_historical_is_a_chart_with_symbol_ticker_param() {
        let entry = lookup("equity/price/historical").expect("route present");
        let widget = derive_widget(&entry);
        assert_eq!(widget.id, "equity_price_historical");
        assert_eq!(widget.widget_type, "chart");
        assert_eq!(widget.category, "equity");
        assert_eq!(widget.endpoint, "/widget-data/equity/price/historical");
        assert_eq!(widget.data.data_key.as_deref(), Some(DATA_KEY));
        assert!(widget.data.chart.is_some(), "chartable route -> chart data");
        let symbol = widget
            .params
            .iter()
            .find(|param| param.param_name == "symbol")
            .expect("symbol ticker param synthesized");
        assert_eq!(symbol.param_type, "ticker");
        assert_eq!(symbol.value, Some(Value::String("AAPL".to_string())));
    }

    #[test]
    fn non_chartable_route_is_a_table_with_columns() {
        let entry = lookup("equity/profile").expect("route present");
        let widget = derive_widget(&entry);
        assert_eq!(widget.widget_type, "table");
        let table = widget.data.table.as_ref().expect("table data present");
        assert!(
            !table.columns_defs.is_empty(),
            "columnsDefs derived from the model schema"
        );
        // Columns are sorted deterministically by field name.
        let fields: Vec<&str> = table
            .columns_defs
            .iter()
            .map(|column| column.field.as_str())
            .collect();
        let mut sorted = fields.clone();
        sorted.sort_unstable();
        assert_eq!(fields, sorted, "columns are in deterministic field order");
    }

    #[test]
    fn date_params_map_to_date_control() {
        let entry = lookup("equity/price/historical").expect("route present");
        let widget = derive_widget(&entry);
        let start = widget
            .params
            .iter()
            .find(|param| param.param_name == "start_date")
            .expect("start_date param present from StandardParams");
        assert_eq!(start.param_type, "date");
    }

    #[test]
    fn formatter_heuristics_apply_to_volume_and_percent_fields() {
        assert_eq!(formatter_for("volume").as_deref(), Some("int"));
        assert_eq!(formatter_for("shares_outstanding").as_deref(), Some("int"));
        assert_eq!(formatter_for("dividend_yield").as_deref(), Some("percent"));
        assert_eq!(formatter_for("close"), None);
    }

    #[test]
    fn cell_data_type_reflects_json_type() {
        let number = serde_json::json!({ "type": "number" });
        assert_eq!(cell_data_type_for(&number), "number");
        let nullable = serde_json::json!({ "type": ["integer", "null"] });
        assert_eq!(cell_data_type_for(&nullable), "number");
        let text = serde_json::json!({ "type": "string" });
        assert_eq!(cell_data_type_for(&text), "text");
    }

    #[test]
    fn title_case_humanizes_snake_case() {
        assert_eq!(title_case("market_cap"), "Market Cap");
        assert_eq!(title_case("symbol"), "Symbol");
    }

    #[test]
    fn every_widget_binds_the_mcp_fetch_tool() {
        let entry = lookup("equity/price/historical").expect("route present");
        let widget = derive_widget(&entry);
        let tool = widget.mcp_tool.as_ref().expect("mcp_tool bound");
        assert_eq!(tool.mcp_server, "tdw-mcp");
        assert_eq!(tool.tool_id, "tdw.provider.fetch");
    }

    #[test]
    fn crypto_symbol_default_differs_from_equity() {
        let entry = lookup("crypto/price/historical").expect("route present");
        if entry.kind == EndpointKind::Fetch {
            let widget = derive_widget(&entry);
            let symbol = widget
                .params
                .iter()
                .find(|param| param.param_name == "symbol")
                .expect("crypto symbol param");
            assert_eq!(symbol.value, Some(Value::String("BTCUSD".to_string())));
        }
    }
}
