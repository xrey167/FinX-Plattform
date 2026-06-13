//! The curated knowledge graph-visualization widget (K-M6).
//!
//! Unlike the catalog-derived widgets ([`crate::derive`]), this widget is NOT a
//! projection of an endpoint-catalog `Fetch` route: the knowledge graph is a
//! read API on the daemon's `KnowledgeRuntime`, not a provider-backed data
//! route. So the widget is authored here as a small `const`-style builder and
//! merged into the assembled `widgets.json` by [`crate::catalog::widgets_json`],
//! the same way the curated default app is authored in [`crate::catalog`].
//!
//! It points at the dedicated, read-only data endpoint
//! `/widget-data/knowledge/graph`, which the Workspace bridge serves out-of-band
//! of the catalog dispatch (see `tdw_app_server::KnowledgeGraphHandler`). The
//! endpoint resolves an *ego-graph* — a root node plus its bounded neighborhood
//! — or a taxonomy slice, and renders it as Markdown the Workspace displays.
//!
//! # Doc provenance
//!
//! The widget shape (the `markdown` render type, the `dataKey`, the param
//! controls) is a faithful projection of the **public** `OpenBB` Workspace
//! `widgets.json` reference — no `OpenBB` source code was consulted. A
//! `markdown` widget reads a Markdown string from its `dataKey` and renders it;
//! see <https://docs.openbb.co/workspace/data-integration/widgets.json>.

use serde_json::Value;

use crate::config::{GridData, McpTool, ParamDef, WidgetConfig, WidgetData};

/// Stable widget id for the knowledge graph-visualization widget.
pub const GRAPH_WIDGET_ID: &str = "knowledge_graph";

/// The read-only data endpoint the widget fetches from. Served by the Workspace
/// bridge's optional knowledge graph handler, NOT the catalog dispatch.
pub const GRAPH_WIDGET_ENDPOINT: &str = "/widget-data/knowledge/graph";

/// The JSON key the rendered Markdown lives under in the endpoint response
/// (the `dataKey` the widget declares — matches the daemon's envelope shape).
pub const GRAPH_DATA_KEY: &str = "results";

/// The default ego-graph depth (hop budget) the widget requests.
///
/// Kept small so a first render on a hub node stays bounded; the user can raise
/// it via the `depth` control up to the engine's `MAX_HOPS` ceiling.
pub const GRAPH_DEFAULT_DEPTH: u64 = 1;

/// Build the curated [`WidgetConfig`] for the knowledge graph-visualization
/// widget (K-M6).
///
/// Read-only: the widget only fetches; it never writes the graph. The `root`
/// param is the entity/tag node id to anchor the ego-graph on (e.g.
/// `instrument:AAPL`); `depth` bounds the hop budget; `as_of` is the optional
/// leakage-safe point-in-time (RFC 3339) the daemon applies to the temporal
/// filter.
#[must_use]
pub fn graph_widget() -> WidgetConfig {
    WidgetConfig {
        id: GRAPH_WIDGET_ID.to_string(),
        name: "Knowledge Graph".to_string(),
        description: "Read-only ego-graph and taxonomy view of the knowledge graph: a root \
                      entity/tag plus its bounded neighborhood, rendered from the live \
                      knowledge read API."
            .to_string(),
        category: "knowledge".to_string(),
        widget_type: "markdown".to_string(),
        endpoint: GRAPH_WIDGET_ENDPOINT.to_string(),
        grid_data: GridData {
            w: 20,
            h: 12,
            min_w: Some(8),
            min_h: Some(6),
        },
        params: vec![
            ParamDef {
                param_name: "root".to_string(),
                param_type: "text".to_string(),
                value: None,
                label: Some("Root node".to_string()),
                description: Some(
                    "Entity or tag node id to anchor the ego-graph on (e.g. instrument:AAPL). \
                     Leave empty for an honest empty graph."
                        .to_string(),
                ),
                options: Vec::new(),
                multi_select: Some(false),
            },
            ParamDef {
                param_name: "depth".to_string(),
                param_type: "number".to_string(),
                value: Some(Value::from(GRAPH_DEFAULT_DEPTH)),
                label: Some("Depth".to_string()),
                description: Some(
                    "Hop budget for the ego-graph (bounded by the engine's MAX_HOPS ceiling)."
                        .to_string(),
                ),
                options: Vec::new(),
                multi_select: Some(false),
            },
            ParamDef {
                param_name: "as_of".to_string(),
                param_type: "date".to_string(),
                value: None,
                label: Some("As of".to_string()),
                description: Some(
                    "Leakage-safe point-in-time (RFC 3339): only facts valid at or before \
                     this instant are shown. Empty applies no temporal filter."
                        .to_string(),
                ),
                options: Vec::new(),
                multi_select: Some(false),
            },
        ],
        data: WidgetData {
            data_key: Some(GRAPH_DATA_KEY.to_string()),
            table: None,
            chart: None,
        },
        refetch_interval: None,
        stale_time: Some(60_000),
        mcp_tool: Some(McpTool {
            // Agents reach the same read-only neighborhood via the kg traverse
            // tool on the tdw-mcp server.
            mcp_server: "tdw-mcp".to_string(),
            tool_id: "tdw.kg.traverse".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_widget_is_read_only_markdown_with_bounded_params() {
        let widget = graph_widget();
        assert_eq!(widget.id, GRAPH_WIDGET_ID);
        assert_eq!(widget.widget_type, "markdown");
        assert_eq!(widget.category, "knowledge");
        assert_eq!(widget.endpoint, GRAPH_WIDGET_ENDPOINT);
        assert_eq!(widget.data.data_key.as_deref(), Some(GRAPH_DATA_KEY));
        // No chart/table render config — a markdown widget renders the string.
        assert!(widget.data.chart.is_none());
        assert!(widget.data.table.is_none());

        // The three documented params are present.
        let names: Vec<&str> = widget
            .params
            .iter()
            .map(|param| param.param_name.as_str())
            .collect();
        assert_eq!(names, ["root", "depth", "as_of"]);
        let depth = widget
            .params
            .iter()
            .find(|param| param.param_name == "depth")
            .expect("depth param");
        assert_eq!(depth.value, Some(Value::from(GRAPH_DEFAULT_DEPTH)));
        let as_of = widget
            .params
            .iter()
            .find(|param| param.param_name == "as_of")
            .expect("as_of param");
        assert_eq!(as_of.param_type, "date");
    }

    #[test]
    fn graph_widget_binds_the_kg_traverse_tool() {
        let tool = graph_widget().mcp_tool.expect("mcp_tool bound");
        assert_eq!(tool.mcp_server, "tdw-mcp");
        assert_eq!(tool.tool_id, "tdw.kg.traverse");
    }
}
