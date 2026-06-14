//! The Workspace control-plane MCP server: a minimal stdio JSON-RPC loop that
//! dispatches the `tdw.workspace.*` tools against a mutable [`WorkspaceState`].
//!
//! The serve loop, [`ToolDescriptor`] shape, and JSON-RPC envelope mirror the
//! data MCP (`tdw-mcp`) pattern — `initialize` / `ping` / `tools/list` /
//! `tools/call` over newline-delimited JSON-RPC 2.0 on stdio — kept self-contained
//! here (plain `std`, no shared server crate) so the control plane stays a small
//! pure-Rust packet.

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::state::{Placement, WorkspaceError, WorkspaceState};

/// MCP protocol revision this server negotiates.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const SERVER_NAME: &str = "tdw-workspace-mcp";
const SERVER_TITLE: &str = "TDW Workspace Control-Plane MCP Server";

/// A single MCP tool advertised in `tools/list`, matching the data MCP's shape.
#[derive(Clone, Debug, Serialize)]
struct ToolDescriptor {
    name: String,
    title: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
    annotations: Value,
}

/// A parsed inbound JSON-RPC request/notification.
#[derive(Clone, Debug)]
struct Inbound {
    id: Option<Value>,
    method: String,
    params: Value,
    is_notification: bool,
}

/// A JSON-RPC protocol-level problem (mapped to an `error` response).
#[derive(Clone, Debug)]
struct Problem {
    id: Value,
    code: i64,
    message: String,
}

impl Problem {
    fn new(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            id,
            code,
            message: message.into(),
        }
    }
}

/// The control-plane MCP server: owns the mutable workspace document and the
/// JSON-RPC initialization state.
#[derive(Debug, Default)]
pub struct WorkspaceServer {
    state: WorkspaceState,
    initialized: bool,
}

impl WorkspaceServer {
    /// A new server over an empty default workspace document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: WorkspaceState::new(
                "TDW Workspace",
                "An agent-controlled Workspace dashboard built over the widget catalog.",
            ),
            initialized: false,
        }
    }

    /// Borrow the underlying workspace document (for tests / embedding).
    #[must_use]
    pub const fn state(&self) -> &WorkspaceState {
        &self.state
    }

    /// Handle one JSON-RPC line, returning zero or more encoded response lines.
    ///
    /// Notifications (no `id`) produce no response line. Mirrors the data MCP's
    /// line handler so a client can drive either server identically.
    #[must_use]
    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let messages = match parse_inbound(line) {
            Ok(inbound) if inbound.is_notification => {
                self.handle_notification(&inbound);
                Vec::new()
            }
            Ok(inbound) => self.handle_request(&inbound),
            Err(problem) => vec![error_message(&problem)],
        };
        messages.iter().map(encode_message).collect()
    }

    fn handle_notification(&mut self, inbound: &Inbound) {
        if inbound.method == "notifications/initialized" {
            self.initialized = true;
        }
    }

    fn handle_request(&mut self, inbound: &Inbound) -> Vec<Value> {
        let id = inbound.id.clone().unwrap_or(Value::Null);
        if !self.initialized && !matches!(inbound.method.as_str(), "initialize" | "ping") {
            return vec![error_message(&Problem::new(
                id,
                -32002,
                "server is not initialized",
            ))];
        }
        match inbound.method.as_str() {
            "initialize" => vec![self.initialize(&id, &inbound.params)],
            "ping" => vec![success_message(&id, &json!({}))],
            "tools/list" => vec![success_message(
                &id,
                &json!({ "tools": tool_descriptors() }),
            )],
            "tools/call" => vec![self.call_tool(&id, &inbound.params)],
            _ => vec![error_message(&Problem::new(id, -32601, "method not found"))],
        }
    }

    fn initialize(&mut self, id: &Value, params: &Value) -> Value {
        self.initialized = true;
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(MCP_PROTOCOL_VERSION);
        success_message(
            id,
            &json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "title": SERVER_TITLE,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": format!(
                    "The Workspace control plane: create and arrange dashboards and widgets over the widget catalog, then read back the apps.json-shaped layout. Requested protocol: {requested}."
                ),
            }),
        )
    }

    fn call_tool(&mut self, id: &Value, params: &Value) -> Value {
        let Some(params_object) = params.as_object() else {
            return error_message(&Problem::new(
                id.clone(),
                -32602,
                "tools/call params must be an object",
            ));
        };
        let Some(name) = params_object.get("name").and_then(Value::as_str) else {
            return error_message(&Problem::new(
                id.clone(),
                -32602,
                "tools/call requires string field: name",
            ));
        };
        let arguments = params_object
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let Some(arguments) = arguments.as_object() else {
            return error_message(&Problem::new(
                id.clone(),
                -32602,
                "tools/call arguments must be an object",
            ));
        };
        match self.execute(name, arguments) {
            Ok(structured) => success_message(id, &tool_result(&structured)),
            Err(ToolFailure::Protocol(code, message)) => {
                error_message(&Problem::new(id.clone(), code, message))
            }
            Err(ToolFailure::Execution(message)) => {
                success_message(id, &tool_error_result(&message))
            }
        }
    }

    /// Dispatch one `tdw.workspace.*` tool by name.
    fn execute(&mut self, name: &str, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        match name {
            "tdw.workspace.dashboard.list" => Ok(self.dashboard_list()),
            "tdw.workspace.dashboard.create" => self.dashboard_create(args),
            "tdw.workspace.dashboard.delete" => self.dashboard_delete(args),
            "tdw.workspace.dashboard.rename" => self.dashboard_rename(args),
            "tdw.workspace.widget.add" => self.widget_add(args),
            "tdw.workspace.widget.move" => self.widget_move(args),
            "tdw.workspace.widget.resize" => self.widget_resize(args),
            "tdw.workspace.widget.remove" => self.widget_remove(args),
            "tdw.workspace.widget.schema" => Self::widget_schema(args),
            "tdw.workspace.layout.get" => Ok(self.layout_get()),
            "tdw.workspace.navigate" => self.navigate(args),
            _ => Err(ToolFailure::Protocol(
                -32601,
                format!("unknown tool: {name}"),
            )),
        }
    }

    fn dashboard_list(&self) -> Value {
        let dashboards: Vec<Value> = self
            .state
            .dashboard_ids()
            .into_iter()
            .filter_map(|id| {
                let dashboard = self.state.dashboard(&id)?;
                Some(json!({
                    "id": dashboard.id,
                    "name": dashboard.name,
                    "widgetCount": dashboard.widgets.len(),
                }))
            })
            .collect();
        json!({
            "dashboards": dashboards,
            "active": self.state.active_dashboard(),
        })
    }

    fn dashboard_create(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let id = required_str(args, "id")?;
        let name = optional_str(args, "name").unwrap_or(id);
        self.state
            .create_dashboard(id, name)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "created": id, "active": self.state.active_dashboard() }))
    }

    fn dashboard_delete(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let id = required_str(args, "id")?;
        self.state
            .delete_dashboard(id)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "deleted": id, "active": self.state.active_dashboard() }))
    }

    fn dashboard_rename(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let id = required_str(args, "id")?;
        let name = required_str(args, "name")?;
        self.state
            .rename_dashboard(id, name)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "renamed": id, "name": name }))
    }

    fn widget_add(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let dashboard = required_str(args, "dashboard")?;
        let widget_id = required_str(args, "widget_id")?;
        let placement = required_placement(args)?;
        let instance = self
            .state
            .add_widget(dashboard, widget_id, placement)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "instance_id": instance, "widget_id": widget_id, "dashboard": dashboard }))
    }

    fn widget_move(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let dashboard = required_str(args, "dashboard")?;
        let instance = required_str(args, "instance_id")?;
        let x = required_u32(args, "x")?;
        let y = required_u32(args, "y")?;
        self.state
            .move_widget(dashboard, instance, x, y)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "moved": instance, "x": x, "y": y }))
    }

    fn widget_resize(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let dashboard = required_str(args, "dashboard")?;
        let instance = required_str(args, "instance_id")?;
        let w = required_u32(args, "w")?;
        let h = required_u32(args, "h")?;
        self.state
            .resize_widget(dashboard, instance, w, h)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "resized": instance, "w": w, "h": h }))
    }

    fn widget_remove(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let dashboard = required_str(args, "dashboard")?;
        let instance = required_str(args, "instance_id")?;
        self.state
            .remove_widget(dashboard, instance)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "removed": instance, "dashboard": dashboard }))
    }

    /// Inspect a catalog widget's params schema, delegating to the widget catalog
    /// (the same catalog every widget id and citation resolves back to).
    fn widget_schema(args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let widget_id = required_str(args, "widget_id")?;
        let widget = tdw_widgets::catalog_widgets()
            .into_iter()
            .find(|w| w.id == widget_id)
            .ok_or_else(|| ToolFailure::Execution(format!("unknown widget id: {widget_id}")))?;
        // The slash-route the widget id projects from; its catalog entry carries
        // the canonical request params JSON schema.
        let route = widget_id.replace('_', "/");
        let params_schema = tdw_endpoint_catalog::lookup(&route)
            .map(|entry| serde_json::to_value((entry.params_schema)()).unwrap_or(Value::Null));
        let params = serde_json::to_value(&widget.params).unwrap_or(Value::Null);
        Ok(json!({
            "id": widget.id,
            "name": widget.name,
            "category": widget.category,
            "endpoint": widget.endpoint,
            "params": params,
            "paramsSchema": params_schema,
        }))
    }

    fn layout_get(&self) -> Value {
        json!({
            "apps": self.state.to_apps_json(),
            "active": self.state.active_dashboard(),
        })
    }

    fn navigate(&mut self, args: &Map<String, Value>) -> Result<Value, ToolFailure> {
        let id = required_str(args, "dashboard")?;
        self.state
            .navigate(id)
            .map_err(|e| ToolFailure::from_workspace(&e))?;
        Ok(json!({ "active": id }))
    }
}

/// A tool execution outcome that is not a structured success.
enum ToolFailure {
    /// A JSON-RPC protocol error (bad arguments) — surfaces as an `error`.
    Protocol(i64, String),
    /// A domain failure (unknown widget, overlap, …) — surfaces as an `isError`
    /// tool result so the agent can read and recover from it.
    Execution(String),
}

impl ToolFailure {
    /// Map a [`WorkspaceError`] to an execution failure (a readable tool error,
    /// not a protocol error — the agent should see and correct it).
    fn from_workspace(error: &WorkspaceError) -> Self {
        Self::Execution(error.to_string())
    }
}

/// The full advertised `tdw.workspace.*` control-plane tool surface.
fn tool_descriptors() -> Vec<ToolDescriptor> {
    let mut descriptors = dashboard_tool_descriptors();
    descriptors.extend(widget_tool_descriptors());
    descriptors
}

/// The dashboard lifecycle tools (`tdw.workspace.dashboard.*`) plus navigate.
fn dashboard_tool_descriptors() -> Vec<ToolDescriptor> {
    let dashboard_id = json!({ "type": "string", "description": "Dashboard id." });
    vec![
        tool(
            "tdw.workspace.dashboard.list",
            "List Dashboards",
            "List the Workspace dashboards (id, name, widget count) and the active dashboard.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.workspace.dashboard.create",
            "Create Dashboard",
            "Create a new empty dashboard. The first dashboard created becomes the active one.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Unique dashboard id." },
                    "name": { "type": "string", "description": "Display name (defaults to the id)." }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.dashboard.delete",
            "Delete Dashboard",
            "Delete a dashboard by id. If it was active, the active selection moves to the first remaining dashboard.",
            json!({
                "type": "object",
                "properties": { "id": dashboard_id },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.dashboard.rename",
            "Rename Dashboard",
            "Rename a dashboard's display name (its id is stable).",
            json!({
                "type": "object",
                "properties": {
                    "id": dashboard_id,
                    "name": { "type": "string", "description": "New display name." }
                },
                "required": ["id", "name"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// The widget placement tools (`tdw.workspace.widget.*`), the layout reader, and
/// the active-dashboard navigate tool.
fn widget_tool_descriptors() -> Vec<ToolDescriptor> {
    let dashboard_id = json!({ "type": "string", "description": "Dashboard id." });
    vec![
        tool(
            "tdw.workspace.widget.add",
            "Add Widget",
            "Place a catalog widget on a dashboard at a grid position/size. The widget id must exist in the widget catalog (call tdw.workspace.widget.schema), and the placement must fit the 40-column grid without overlapping another widget.",
            json!({
                "type": "object",
                "properties": {
                    "dashboard": dashboard_id,
                    "widget_id": { "type": "string", "description": "Catalog widget id, e.g. equity_price_historical." },
                    "x": { "type": "integer", "minimum": 0, "description": "Column origin (0-based)." },
                    "y": { "type": "integer", "minimum": 0, "description": "Row origin (0-based)." },
                    "w": { "type": "integer", "minimum": 1, "description": "Width in columns." },
                    "h": { "type": "integer", "minimum": 1, "description": "Height in rows." }
                },
                "required": ["dashboard", "widget_id", "x", "y", "w", "h"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.widget.move",
            "Move Widget",
            "Move a placed widget instance to a new grid origin, keeping its size. Rejected if the destination is out of bounds or overlaps another widget.",
            json!({
                "type": "object",
                "properties": {
                    "dashboard": dashboard_id,
                    "instance_id": { "type": "string", "description": "Placed widget instance id (from tdw.workspace.widget.add)." },
                    "x": { "type": "integer", "minimum": 0 },
                    "y": { "type": "integer", "minimum": 0 }
                },
                "required": ["dashboard", "instance_id", "x", "y"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.widget.resize",
            "Resize Widget",
            "Resize a placed widget instance, keeping its origin. Rejected if it would leave the grid or overlap another widget.",
            json!({
                "type": "object",
                "properties": {
                    "dashboard": dashboard_id,
                    "instance_id": { "type": "string" },
                    "w": { "type": "integer", "minimum": 1 },
                    "h": { "type": "integer", "minimum": 1 }
                },
                "required": ["dashboard", "instance_id", "w", "h"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.widget.remove",
            "Remove Widget",
            "Remove a placed widget instance from a dashboard.",
            json!({
                "type": "object",
                "properties": {
                    "dashboard": dashboard_id,
                    "instance_id": { "type": "string" }
                },
                "required": ["dashboard", "instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.widget.schema",
            "Inspect Widget Schema",
            "Return a catalog widget's params and request params JSON schema (delegates to the widget catalog).",
            json!({
                "type": "object",
                "properties": {
                    "widget_id": { "type": "string", "description": "Catalog widget id." }
                },
                "required": ["widget_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.workspace.layout.get",
            "Get Layout",
            "Return the current layout as an apps.json-shaped Workspace app document.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.workspace.navigate",
            "Navigate",
            "Select the active dashboard. Rejected if the dashboard id is unknown.",
            json!({
                "type": "object",
                "properties": { "dashboard": dashboard_id },
                "required": ["dashboard"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        input_schema,
        annotations: json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false,
        }),
    }
}

// --- JSON-RPC envelope helpers (mirrors the data MCP's wire shape) -----------

fn parse_inbound(line: &str) -> Result<Inbound, Problem> {
    let value: Value =
        serde_json::from_str(line).map_err(|_| Problem::new(Value::Null, -32700, "parse error"))?;
    let object = value
        .as_object()
        .ok_or_else(|| Problem::new(Value::Null, -32600, "invalid request"))?;
    let id = object.get("id").cloned();
    let id_for_error = match id.as_ref() {
        Some(value) if is_valid_id(value) => value.clone(),
        Some(_) | None => Value::Null,
    };
    if id.as_ref().is_some_and(|value| !is_valid_id(value)) {
        return Err(Problem::new(Value::Null, -32600, "invalid request id"));
    }
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Problem::new(id_for_error, -32600, "invalid request"));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| Problem::new(id_for_error, -32600, "invalid request"))?;
    Ok(Inbound {
        id,
        method: method.to_string(),
        params: object.get("params").cloned().unwrap_or(Value::Null),
        is_notification: !object.contains_key("id"),
    })
}

fn is_valid_id(value: &Value) -> bool {
    value.is_string() || value.is_number() || value.is_null()
}

fn success_message(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_message(problem: &Problem) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": problem.id,
        "error": { "code": problem.code, "message": problem.message },
    })
}

fn encode_message(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32603,"message":"serialize error: {error}"}}}}"#
        )
    })
}

fn tool_result(structured: &Value) -> Value {
    json!({
        "content": [ { "type": "text", "text": pretty_json(structured) } ],
        "structuredContent": structured,
        "isError": false,
    })
}

fn tool_error_result(message: &str) -> Value {
    json!({
        "content": [ { "type": "text", "text": message } ],
        "isError": true,
    })
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|_| "{\"error\":\"could not serialize structured content\"}".to_string())
}

// --- Argument extraction -----------------------------------------------------

fn required_str<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ToolFailure> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::Protocol(-32602, format!("missing string argument: {key}")))
}

fn optional_str<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn required_u32(args: &Map<String, Value>, key: &str) -> Result<u32, ToolFailure> {
    let raw = args
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolFailure::Protocol(-32602, format!("missing integer argument: {key}")))?;
    u32::try_from(raw)
        .map_err(|_| ToolFailure::Protocol(-32602, format!("argument out of range: {key}")))
}

fn required_placement(args: &Map<String, Value>) -> Result<Placement, ToolFailure> {
    Ok(Placement {
        x: required_u32(args, "x")?,
        y: required_u32(args, "y")?,
        w: required_u32(args, "w")?,
        h: required_u32(args, "h")?,
    })
}

/// Run the blocking stdio JSON-RPC loop.
///
/// Reads newline-delimited requests from stdin, dispatches each against a fresh
/// [`WorkspaceServer`], and writes each response line to stdout. Mirrors the data
/// MCP's `run_stdio_json_rpc`. Returns a process exit code.
#[must_use]
pub fn run_stdio_json_rpc() -> i32 {
    use std::io::BufRead;

    let stdin = std::io::stdin();
    let mut server = WorkspaceServer::new();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => {
                for message in server.handle_line(&line) {
                    println!("{message}");
                }
            }
            Err(error) => {
                eprintln!("tdw-workspace-mcp JSON-RPC read error: {error}");
                return 1;
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_real_widget_id() -> String {
        tdw_widgets::catalog_widgets()
            .first()
            .expect("catalog non-empty")
            .id
            .clone()
    }

    /// Drive `initialize` then return an initialized server.
    fn initialized() -> WorkspaceServer {
        let mut server = WorkspaceServer::new();
        let out = server.handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(out.len(), 1);
        server
    }

    fn call(server: &mut WorkspaceServer, id: i64, name: &str, arguments: &Value) -> Value {
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let out = server.handle_line(&request.to_string());
        assert_eq!(out.len(), 1, "tools/call returns one response");
        serde_json::from_str(&out[0]).expect("response is JSON")
    }

    #[test]
    fn initialize_negotiates_protocol() {
        let mut server = WorkspaceServer::new();
        let out = server.handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        let response: Value = serde_json::from_str(&out[0]).expect("json");
        assert_eq!(
            response["result"]["protocolVersion"],
            json!(MCP_PROTOCOL_VERSION)
        );
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "tdw-workspace-mcp"
        );
    }

    #[test]
    fn requires_initialize_before_other_methods() {
        let mut server = WorkspaceServer::new();
        let out = server.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
        let response: Value = serde_json::from_str(&out[0]).expect("json");
        assert_eq!(response["error"]["code"], json!(-32002));
    }

    #[test]
    fn tools_list_advertises_the_control_surface() {
        let mut server = initialized();
        let out = server.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let response: Value = serde_json::from_str(&out[0]).expect("json");
        let names: Vec<&str> = response["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        for expected in [
            "tdw.workspace.dashboard.list",
            "tdw.workspace.dashboard.create",
            "tdw.workspace.dashboard.delete",
            "tdw.workspace.dashboard.rename",
            "tdw.workspace.widget.add",
            "tdw.workspace.widget.move",
            "tdw.workspace.widget.resize",
            "tdw.workspace.widget.remove",
            "tdw.workspace.widget.schema",
            "tdw.workspace.layout.get",
            "tdw.workspace.navigate",
        ] {
            assert!(names.contains(&expected), "missing tool: {expected}");
        }
    }

    #[test]
    fn add_unknown_widget_is_a_tool_error_not_a_protocol_error() {
        let mut server = initialized();
        call(
            &mut server,
            2,
            "tdw.workspace.dashboard.create",
            &json!({ "id": "main" }),
        );
        let response = call(
            &mut server,
            3,
            "tdw.workspace.widget.add",
            &json!({ "dashboard": "main", "widget_id": "no_such_widget", "x": 0, "y": 0, "w": 10, "h": 10 }),
        );
        assert_eq!(response["result"]["isError"], json!(true));
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("unknown widget")
        );
    }

    #[test]
    fn end_to_end_crud_and_layout() {
        let mut server = initialized();
        let widget = a_real_widget_id();
        call(
            &mut server,
            2,
            "tdw.workspace.dashboard.create",
            &json!({ "id": "overview", "name": "Overview" }),
        );
        let add = call(
            &mut server,
            3,
            "tdw.workspace.widget.add",
            &json!({ "dashboard": "overview", "widget_id": widget, "x": 0, "y": 0, "w": 20, "h": 12 }),
        );
        let instance = add["result"]["structuredContent"]["instance_id"]
            .as_str()
            .expect("instance id")
            .to_string();

        // Overlapping add is a tool error.
        let overlap = call(
            &mut server,
            4,
            "tdw.workspace.widget.add",
            &json!({ "dashboard": "overview", "widget_id": widget, "x": 10, "y": 6, "w": 20, "h": 12 }),
        );
        assert_eq!(overlap["result"]["isError"], json!(true));

        // Move to a free spot succeeds.
        let moved = call(
            &mut server,
            5,
            "tdw.workspace.widget.move",
            &json!({ "dashboard": "overview", "instance_id": instance, "x": 20, "y": 0 }),
        );
        assert_eq!(moved["result"]["isError"], json!(false));

        // layout.get returns the apps.json shape with the widget placed.
        let layout = call(&mut server, 6, "tdw.workspace.layout.get", &json!({}));
        let apps = &layout["result"]["structuredContent"]["apps"];
        let item = &apps["TDW Workspace"]["tabs"]["overview"]["layout"][0];
        assert_eq!(item["i"], json!(widget));
        assert_eq!(item["x"], json!(20));

        // navigate to unknown dashboard is a tool error.
        let nav = call(
            &mut server,
            7,
            "tdw.workspace.navigate",
            &json!({ "dashboard": "ghost" }),
        );
        assert_eq!(nav["result"]["isError"], json!(true));

        // remove + delete close the lifecycle.
        let removed = call(
            &mut server,
            8,
            "tdw.workspace.widget.remove",
            &json!({ "dashboard": "overview", "instance_id": instance }),
        );
        assert_eq!(removed["result"]["isError"], json!(false));
    }

    #[test]
    fn widget_schema_delegates_to_catalog() {
        let mut server = initialized();
        let widget = a_real_widget_id();
        let response = call(
            &mut server,
            2,
            "tdw.workspace.widget.schema",
            &json!({ "widget_id": widget }),
        );
        assert_eq!(response["result"]["isError"], json!(false));
        assert_eq!(response["result"]["structuredContent"]["id"], json!(widget));
    }

    #[test]
    fn unknown_tool_is_a_protocol_error() {
        let mut server = initialized();
        let response = call(&mut server, 2, "tdw.workspace.not.a.tool", &json!({}));
        assert_eq!(response["error"]["code"], json!(-32601));
    }
}
