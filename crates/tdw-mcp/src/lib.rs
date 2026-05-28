#![forbid(unsafe_code)]

use std::io::BufRead;

use serde::Serialize;
use serde_json::{Map, Value, json};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

const SERVER_NAME: &str = "tdw-mcp";
const SERVER_TITLE: &str = "TDW MCP Server";
const MAX_CANCELLED_REQUESTS: usize = 128;
const MCP_BOUNDARY_DOC: &str =
    include_str!("../../../docs/quality/mcp-worker-product-boundaries.md");
const TEST_TAXONOMY_DOC: &str =
    include_str!("../../../docs/quality/daemon-hardening-test-taxonomy.md");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelledRequest {
    pub request_id: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
struct JsonRpcInbound {
    id: Option<Value>,
    method: String,
    params: Value,
    is_notification: bool,
}

#[derive(Clone, Debug)]
struct JsonRpcProblem {
    id: Value,
    code: i64,
    message: String,
    data: Option<Value>,
}

impl JsonRpcProblem {
    fn new(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            id,
            code,
            message: message.into(),
            data: None,
        }
    }

    fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    fn with_id(mut self, id: Value) -> Self {
        self.id = id;
        self
    }
}

#[derive(Default)]
pub struct McpServer {
    initialized: bool,
    client_info: Option<Value>,
    cancelled_requests: Vec<CancelledRequest>,
}

impl McpServer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn client_info(&self) -> Option<&Value> {
        self.client_info.as_ref()
    }

    pub fn cancelled_requests(&self) -> &[CancelledRequest] {
        &self.cancelled_requests
    }

    pub fn handle_json_rpc_line(&mut self, line: &str) -> Vec<String> {
        let messages = match parse_inbound(line) {
            Ok(inbound) if inbound.is_notification => self.handle_notification(inbound),
            Ok(inbound) => self.handle_request(inbound),
            Err(problem) => vec![error_message(problem)],
        };

        messages.iter().map(encode_message).collect()
    }

    fn handle_notification(&mut self, inbound: JsonRpcInbound) -> Vec<Value> {
        match inbound.method.as_str() {
            "notifications/initialized" => {
                self.initialized = true;
            }
            "notifications/cancelled" => {
                if let Some(cancelled) = cancelled_request_from_params(&inbound.params) {
                    self.record_cancelled_request(cancelled);
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn record_cancelled_request(&mut self, cancelled: CancelledRequest) {
        if let Some(existing) = self
            .cancelled_requests
            .iter_mut()
            .find(|request| request.request_id == cancelled.request_id)
        {
            *existing = cancelled;
            return;
        }

        if self.cancelled_requests.len() == MAX_CANCELLED_REQUESTS {
            self.cancelled_requests.remove(0);
        }
        self.cancelled_requests.push(cancelled);
    }

    fn handle_request(&mut self, inbound: JsonRpcInbound) -> Vec<Value> {
        let id = inbound.id.clone().unwrap_or(Value::Null);
        if !self.initialized && !matches!(inbound.method.as_str(), "initialize" | "ping") {
            return vec![error_message(JsonRpcProblem::new(
                id,
                -32002,
                "server is not initialized",
            ))];
        }

        match inbound.method.as_str() {
            "initialize" => vec![self.initialize(id, inbound.params)],
            "ping" => vec![success_message(id, json!({}))],
            "tools/list" => vec![success_message(id, json!({ "tools": tool_descriptors() }))],
            "tools/call" => self.call_tool(id, inbound.params),
            "resources/list" => vec![success_message(
                id,
                json!({ "resources": resource_descriptors() }),
            )],
            "resources/read" => vec![self.read_resource(id, inbound.params)],
            "prompts/list" => {
                vec![success_message(
                    id,
                    json!({ "prompts": prompt_descriptors() }),
                )]
            }
            "prompts/get" => vec![self.get_prompt(id, inbound.params)],
            _ => vec![error_message(JsonRpcProblem::new(
                id,
                -32601,
                "method not found",
            ))],
        }
    }

    fn initialize(&mut self, id: Value, params: Value) -> Value {
        if let Some(client_info) = params.get("clientInfo") {
            self.client_info = Some(client_info.clone());
        }
        self.initialized = true;

        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(MCP_PROTOCOL_VERSION);

        success_message(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": server_capabilities(),
                "serverInfo": {
                    "name": SERVER_NAME,
                    "title": SERVER_TITLE,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": format!(
                    "TDW exposes read-only warehouse discovery, deterministic provider samples, safe resources, and finance workflow prompts over MCP stdio. Requested protocol: {requested}."
                ),
            }),
        )
    }

    fn call_tool(&self, id: Value, params: Value) -> Vec<Value> {
        let params_object = match params.as_object() {
            Some(params_object) => params_object,
            None => {
                return vec![error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "tools/call params must be an object",
                ))];
            }
        };
        let name = match string_field(params_object, "name") {
            Some(name) => name,
            None => {
                return vec![error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "tools/call requires string field: name",
                ))];
            }
        };
        let arguments = params_object
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return vec![error_message(JsonRpcProblem::new(
                id,
                -32602,
                "tools/call arguments must be an object",
            ))];
        }

        let progress_token = progress_token(&params);
        let result = execute_tool(name, &arguments);
        match result {
            Ok(ToolExecution {
                structured,
                progress_events,
            }) => {
                let mut messages = progress_notifications(progress_token, &progress_events);
                messages.push(success_message(id, tool_result(structured)));
                messages
            }
            Err(ToolFailure::Protocol(problem)) => vec![error_message(
                problem.with_id(id).with_data(json!({ "tool": name })),
            )],
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, tool_error_result(&message))]
            }
        }
    }

    fn read_resource(&self, id: Value, params: Value) -> Value {
        let params_object = match params.as_object() {
            Some(params_object) => params_object,
            None => {
                return error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "resources/read params must be an object",
                ));
            }
        };
        let uri = match string_field(params_object, "uri") {
            Some(uri) => uri,
            None => {
                return error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "resources/read requires string field: uri",
                ));
            }
        };

        match resource_content(uri) {
            Ok(content) => success_message(id, json!({ "contents": [content] })),
            Err(problem) => error_message(problem.with_id(id).with_data(json!({ "uri": uri }))),
        }
    }

    fn get_prompt(&self, id: Value, params: Value) -> Value {
        let params_object = match params.as_object() {
            Some(params_object) => params_object,
            None => {
                return error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "prompts/get params must be an object",
                ));
            }
        };
        let name = match string_field(params_object, "name") {
            Some(name) => name,
            None => {
                return error_message(JsonRpcProblem::new(
                    id,
                    -32602,
                    "prompts/get requires string field: name",
                ));
            }
        };
        let arguments = params_object
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return error_message(JsonRpcProblem::new(
                id,
                -32602,
                "prompts/get arguments must be an object",
            ));
        }

        match prompt_content(name, &arguments) {
            Ok(content) => success_message(id, content),
            Err(problem) => error_message(problem.with_id(id).with_data(json!({ "prompt": name }))),
        }
    }
}

pub fn run_stdio_json_rpc() -> i32 {
    let stdin = std::io::stdin();
    let mut server = McpServer::new();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => {
                for message in server.handle_json_rpc_line(&line) {
                    println!("{message}");
                }
            }
            Err(error) => {
                eprintln!("tdw-mcp JSON-RPC read error: {error}");
                return 1;
            }
        }
    }
    0
}

pub fn handle_json_rpc_lines<I, S>(lines: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut server = McpServer::new();
    let mut messages = Vec::new();
    for line in lines {
        messages.extend(server.handle_json_rpc_line(line.as_ref()));
    }
    messages
}

pub fn handle_json_rpc_line(line: &str) -> Vec<String> {
    handle_json_rpc_lines([line])
}

pub fn mcp_tool_catalog() -> Vec<String> {
    tool_descriptors()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

fn parse_inbound(line: &str) -> Result<JsonRpcInbound, JsonRpcProblem> {
    let value: Value = serde_json::from_str(line)
        .map_err(|_| JsonRpcProblem::new(Value::Null, -32700, "parse error"))?;
    let object = value
        .as_object()
        .ok_or_else(|| JsonRpcProblem::new(Value::Null, -32600, "invalid request"))?;
    let id = object.get("id").cloned();
    let id_for_error = match id.as_ref() {
        Some(value) if is_valid_id(value) => value.clone(),
        Some(_) => Value::Null,
        None => Value::Null,
    };
    if id.as_ref().is_some_and(|value| !is_valid_id(value)) {
        return Err(JsonRpcProblem::new(
            Value::Null,
            -32600,
            "invalid request id",
        ));
    }
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(JsonRpcProblem::new(id_for_error, -32600, "invalid request"));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| JsonRpcProblem::new(id_for_error, -32600, "invalid request"))?;

    Ok(JsonRpcInbound {
        id,
        method: method.to_string(),
        params: object.get("params").cloned().unwrap_or(Value::Null),
        is_notification: !object.contains_key("id"),
    })
}

fn is_valid_id(value: &Value) -> bool {
    value.is_string() || value.is_number() || value.is_null()
}

fn success_message(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn error_message(problem: JsonRpcProblem) -> Value {
    let mut error = json!({
        "code": problem.code,
        "message": problem.message,
    });
    if let Some(data) = problem.data {
        error["data"] = data;
    }
    json!({
        "jsonrpc": "2.0",
        "id": problem.id,
        "error": error,
    })
}

fn notification_message(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

fn encode_message(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32603,"message":"serialize error: {error}"}}}}"#
        )
    })
}

fn server_capabilities() -> Value {
    json!({
        "tools": { "listChanged": false },
        "resources": { "listChanged": false },
        "prompts": { "listChanged": false },
    })
}

#[derive(Clone, Debug, Serialize)]
struct ToolDescriptor {
    name: String,
    title: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
    #[serde(rename = "outputSchema", skip_serializing_if = "Option::is_none")]
    output_schema: Option<Value>,
    annotations: Value,
}

fn tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        tool(
            "tdw.providers.list",
            "List TDW Providers",
            "List registered TDW providers and endpoint kinds.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.equity.historical",
            "Fetch Equity Historical",
            "Fetch deterministic equity historical data through the TDW provider registry.",
            json!({
                "type": "object",
                "properties": {
                    "provider": { "type": "string", "description": "Provider id, defaults to fileset." },
                    "symbol": { "type": "string", "description": "Ticker symbol, for example AAPL." }
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.progress.sample",
            "Emit Progress Sample",
            "Run the deterministic streaming fetch sample and emit MCP progress notifications when a progress token is supplied.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Ticker symbol, for example AAPL." }
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.agent.sample",
            "Agent Surface Evidence",
            "Return deterministic agent schema, workflow, eval, and slash-command evidence.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.extensibility.sample",
            "Extensibility Evidence",
            "Return deterministic tool registry, sandbox, MCP tool, and ACP evidence.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.event_spine.sample",
            "Event Spine Evidence",
            "Return deterministic actor, hook, bus, outbox, CDC, and replay evidence.",
            json!({
                "type": "object",
                "properties": {
                    "entrypoint": { "type": "string", "description": "Entrypoint label, defaults to mcp." }
                },
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.kg_tag.sample",
            "Knowledge Graph And Tag Evidence",
            "Return deterministic KG, resolver, tag-rule, live bus, and feature-store evidence.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.client_event.sample",
            "Client Event Evidence",
            "Return deterministic app-client, app-server, exec, TUI, and replay evidence.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]
}

fn tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        input_schema,
        output_schema: Some(json!({
            "type": "object",
            "properties": {
                "content": { "type": "array" },
                "structuredContent": { "type": "object" },
                "isError": { "type": "boolean" }
            },
            "required": ["content", "isError"]
        })),
        annotations: json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        }),
    }
}

struct ToolExecution {
    structured: Value,
    progress_events: Vec<String>,
}

enum ToolFailure {
    Protocol(JsonRpcProblem),
    Execution(String),
}

fn execute_tool(name: &str, arguments: &Value) -> Result<ToolExecution, ToolFailure> {
    let arguments_object = arguments.as_object().ok_or_else(|| {
        ToolFailure::Protocol(JsonRpcProblem::new(
            Value::Null,
            -32602,
            "tool arguments must be an object",
        ))
    })?;
    match name {
        "tdw.providers.list" => {
            let providers = tdw_service_api::list_providers()
                .map_err(|error| ToolFailure::Execution(error.to_string()))?;
            Ok(structured(json!({ "providers": providers })))
        }
        "tdw.equity.historical" => {
            let symbol = required_argument(arguments_object, "symbol")?;
            let provider = optional_argument(arguments_object, "provider").unwrap_or("fileset");
            let response = tdw_service_api::endpoint_response(provider, symbol)
                .map_err(|error| ToolFailure::Execution(error.to_string()))?;
            Ok(structured(response))
        }
        "tdw.progress.sample" => {
            let symbol = required_argument(arguments_object, "symbol")?;
            let events = tdw_service_api::mcp_progress_sample(symbol)
                .map_err(|error| ToolFailure::Execution(error.to_string()))?;
            Ok(ToolExecution {
                structured: json!({ "events": events }),
                progress_events: events,
            })
        }
        "tdw.agent.sample" => tdw_service_api::agent_tool_sample()
            .map(structured)
            .map_err(|error| ToolFailure::Execution(error.to_string())),
        "tdw.extensibility.sample" => tdw_service_api::extensibility_sample()
            .map(structured)
            .map_err(|error| ToolFailure::Execution(error.to_string())),
        "tdw.event_spine.sample" => {
            let entrypoint = optional_argument(arguments_object, "entrypoint").unwrap_or("mcp");
            tdw_service_api::event_spine_sample(entrypoint)
                .map(structured)
                .map_err(|error| ToolFailure::Execution(error.to_string()))
        }
        "tdw.kg_tag.sample" => tdw_service_api::kg_tag_sample()
            .map(structured)
            .map_err(|error| ToolFailure::Execution(error.to_string())),
        "tdw.client_event.sample" => tdw_service_api::client_event_sample()
            .map(structured)
            .map_err(|error| ToolFailure::Execution(error.to_string())),
        _ => Err(ToolFailure::Protocol(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("unknown tool: {name}"),
        ))),
    }
}

fn structured(structured: Value) -> ToolExecution {
    ToolExecution {
        structured,
        progress_events: Vec::new(),
    }
}

fn tool_result(structured: Value) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": pretty_json(&structured),
            }
        ],
        "structuredContent": structured,
        "isError": false,
    })
}

fn tool_error_result(message: &str) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": message,
            }
        ],
        "isError": true,
    })
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|error| {
        format!("{{\"error\":\"could not serialize structured content\",\"detail\":\"{error}\"}}")
    })
}

fn progress_token(params: &Value) -> Option<Value> {
    let token = params
        .get("_meta")
        .and_then(|meta| meta.get("progressToken"))?;
    if token.is_string() || token.is_number() {
        Some(token.clone())
    } else {
        None
    }
}

fn progress_notifications(progress_token: Option<Value>, events: &[String]) -> Vec<Value> {
    let Some(progress_token) = progress_token else {
        return Vec::new();
    };
    let mut notifications = Vec::new();
    let mut last_progress = -1.0_f64;
    let mut last_stage: Option<String> = None;
    for event in events {
        if let Some((stage, fraction)) = parse_progress_event(event) {
            let stage_changed = last_stage.as_deref() != Some(stage.as_str());
            if stage_changed {
                last_progress = -1.0;
            }
            if stage_changed || fraction > last_progress {
                last_progress = fraction;
                last_stage = Some(stage.clone());
                notifications.push(notification_message(
                    "notifications/progress",
                    json!({
                        "progressToken": progress_token,
                        "progress": fraction,
                        "total": 1.0,
                        "message": stage,
                    }),
                ));
            }
        } else if event.starts_with("done:")
            && (last_progress < 1.0 || last_stage.as_deref() != Some("complete"))
        {
            last_progress = 1.0;
            last_stage = Some("complete".to_string());
            notifications.push(notification_message(
                "notifications/progress",
                json!({
                    "progressToken": progress_token,
                    "progress": 1.0,
                    "total": 1.0,
                    "message": "complete",
                }),
            ));
        }
    }
    notifications
}

fn parse_progress_event(event: &str) -> Option<(String, f64)> {
    let mut parts = event.split(':');
    if parts.next()? != "progress" {
        return None;
    }
    let stage = parts.next()?.to_string();
    let fraction = parts.next()?.parse::<f64>().ok()?;
    if !fraction.is_finite() {
        return None;
    }
    Some((stage, fraction))
}

fn cancelled_request_from_params(params: &Value) -> Option<CancelledRequest> {
    let request_id = params.get("requestId")?;
    let request_id = id_to_string(request_id)?;
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Some(CancelledRequest { request_id, reason })
}

fn id_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Null => Some("null".to_string()),
        _ => None,
    }
}

fn required_argument<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, ToolFailure> {
    optional_argument(arguments, name).ok_or_else(|| {
        ToolFailure::Protocol(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("missing required argument: {name}"),
        ))
    })
}

fn optional_argument<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

#[derive(Clone, Debug, Serialize)]
struct ResourceDescriptor {
    uri: String,
    name: String,
    title: String,
    description: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

fn resource_descriptors() -> Vec<ResourceDescriptor> {
    vec![
        resource(
            "tdw://quality/mcp-worker-product-boundaries",
            "mcp-worker-product-boundaries",
            "MCP And Worker Product Boundaries",
            "Current shipped and remaining MCP/worker boundary status.",
            "text/markdown",
        ),
        resource(
            "tdw://quality/daemon-hardening-test-taxonomy",
            "daemon-hardening-test-taxonomy",
            "Daemon Hardening Test Taxonomy",
            "Always-on, real-backend, live-network, and final gate taxonomy.",
            "text/markdown",
        ),
        resource(
            "tdw://service/protocol-config-sample",
            "protocol-config-sample",
            "Protocol And Config Sample",
            "Deterministic protocol/config evidence from tdw-service-api.",
            "application/json",
        ),
        resource(
            "tdw://mcp/capabilities",
            "mcp-capabilities",
            "MCP Server Capabilities",
            "Runtime MCP protocol version, capabilities, tools, prompts, and resources.",
            "application/json",
        ),
    ]
}

fn resource(
    uri: &str,
    name: &str,
    title: &str,
    description: &str,
    mime_type: &str,
) -> ResourceDescriptor {
    ResourceDescriptor {
        uri: uri.to_string(),
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        mime_type: mime_type.to_string(),
    }
}

fn resource_content(uri: &str) -> Result<Value, JsonRpcProblem> {
    match uri {
        "tdw://quality/mcp-worker-product-boundaries" => Ok(resource_text(
            uri,
            "text/markdown",
            MCP_BOUNDARY_DOC.to_string(),
        )),
        "tdw://quality/daemon-hardening-test-taxonomy" => Ok(resource_text(
            uri,
            "text/markdown",
            TEST_TAXONOMY_DOC.to_string(),
        )),
        "tdw://service/protocol-config-sample" => {
            let sample = tdw_service_api::protocol_config_sample().map_err(|error| {
                JsonRpcProblem::new(
                    Value::Null,
                    -32603,
                    format!("protocol config resource failed: {error}"),
                )
            })?;
            Ok(resource_text(uri, "application/json", pretty_json(&sample)))
        }
        "tdw://mcp/capabilities" => {
            let capabilities = json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": {
                    "name": SERVER_NAME,
                    "title": SERVER_TITLE,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": server_capabilities(),
                "tools": mcp_tool_catalog(),
                "resources": resource_descriptors(),
                "prompts": prompt_descriptors(),
            });
            Ok(resource_text(
                uri,
                "application/json",
                pretty_json(&capabilities),
            ))
        }
        _ => Err(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("unknown resource: {uri}"),
        )),
    }
}

fn resource_text(uri: &str, mime_type: &str, text: String) -> Value {
    json!({
        "uri": uri,
        "mimeType": mime_type,
        "text": text,
    })
}

#[derive(Clone, Debug, Serialize)]
struct PromptDescriptor {
    name: String,
    title: String,
    description: String,
    arguments: Vec<PromptArgument>,
}

#[derive(Clone, Debug, Serialize)]
struct PromptArgument {
    name: String,
    description: String,
    required: bool,
}

fn prompt_descriptors() -> Vec<PromptDescriptor> {
    vec![
        prompt(
            "tdw.equity.research",
            "Equity Research Workflow",
            "Guide a TDW-backed equity research workflow.",
            vec![
                argument("symbol", "Ticker symbol, for example AAPL.", true),
                argument("provider", "Provider id, defaults to fileset.", false),
                argument("horizon", "Research horizon, for example 1d or 30d.", false),
            ],
        ),
        prompt(
            "tdw.daemon.triage",
            "Daemon Operation Triage",
            "Guide diagnosis of a TDW daemon operation using event-spine evidence.",
            vec![argument(
                "op_id",
                "Optional operation id to focus the triage.",
                false,
            )],
        ),
        prompt(
            "tdw.ingest.plan",
            "Provider Ingest Plan",
            "Guide a safe provider ingest plan through TDW registry and policy boundaries.",
            vec![
                argument("provider", "Provider id, defaults to fileset.", false),
                argument(
                    "endpoint",
                    "Endpoint id, defaults to equity_historical.",
                    false,
                ),
                argument("symbol", "Ticker symbol, for example AAPL.", true),
            ],
        ),
    ]
}

fn prompt(
    name: &str,
    title: &str,
    description: &str,
    arguments: Vec<PromptArgument>,
) -> PromptDescriptor {
    PromptDescriptor {
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        arguments,
    }
}

fn argument(name: &str, description: &str, required: bool) -> PromptArgument {
    PromptArgument {
        name: name.to_string(),
        description: description.to_string(),
        required,
    }
}

fn prompt_content(name: &str, arguments: &Value) -> Result<Value, JsonRpcProblem> {
    let empty_arguments = Map::new();
    let arguments_object = arguments.as_object().unwrap_or(&empty_arguments);
    match name {
        "tdw.equity.research" => {
            let symbol = required_prompt_arg(arguments_object, "symbol")?;
            let provider = optional_argument(arguments_object, "provider").unwrap_or("fileset");
            let horizon = optional_argument(arguments_object, "horizon").unwrap_or("1d");
            Ok(prompt_messages(
                "TDW equity research workflow",
                format!(
                    "Use TDW MCP tools to research {symbol} with provider {provider} over horizon {horizon}. Start with tdw.providers.list, fetch tdw.equity.historical, call tdw.kg_tag.sample for context, then summarize data quality, rows observed, warehouse follow-ups, and risk notes."
                ),
            ))
        }
        "tdw.daemon.triage" => {
            let op_id = optional_argument(arguments_object, "op_id").unwrap_or("the target op");
            Ok(prompt_messages(
                "TDW daemon operation triage",
                format!(
                    "Triage {op_id} through the TDW daemon boundary. Check started/completed/failed event order, outbox relay status, session cost entries, rollout frames, and policy evidence before proposing a fix."
                ),
            ))
        }
        "tdw.ingest.plan" => {
            let symbol = required_prompt_arg(arguments_object, "symbol")?;
            let provider = optional_argument(arguments_object, "provider").unwrap_or("fileset");
            let endpoint =
                optional_argument(arguments_object, "endpoint").unwrap_or("equity_historical");
            Ok(prompt_messages(
                "TDW provider ingest plan",
                format!(
                    "Plan a safe ingest for provider {provider}, endpoint {endpoint}, symbol {symbol}. Validate provider registration, policy role, idempotency, expected event-spine writes, storage target, and skipped live-network requirements."
                ),
            ))
        }
        _ => Err(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("unknown prompt: {name}"),
        )),
    }
}

fn required_prompt_arg<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, JsonRpcProblem> {
    optional_argument(arguments, name).ok_or_else(|| {
        JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("missing required prompt argument: {name}"),
        )
    })
}

fn prompt_messages(description: &str, text: String) -> Value {
    json!({
        "description": description,
        "messages": [
            {
                "role": "user",
                "content": {
                    "type": "text",
                    "text": text,
                },
            }
        ],
    })
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(message: &str) -> Value {
        serde_json::from_str(message)
            .unwrap_or_else(|error| panic!("response should be json: {error}; {message}"))
    }

    fn initialize(server: &mut McpServer) -> Value {
        let messages = server.handle_json_rpc_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}}"#,
        );
        assert_eq!(messages.len(), 1);
        decode(&messages[0])
    }

    #[test]
    fn initialize_negotiates_capabilities_and_server_info() {
        let mut server = McpServer::new();
        let response = initialize(&mut server);

        assert!(server.is_initialized());
        assert_eq!(
            server.client_info().and_then(|info| info.get("name")),
            Some(&json!("test-client"))
        );
        assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_object());
        assert!(response["result"]["capabilities"]["prompts"].is_object());
    }

    #[test]
    fn initialized_and_cancelled_notifications_are_fire_and_forget() {
        let mut server = McpServer::new();

        assert!(
            server
                .handle_json_rpc_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_empty()
        );
        assert!(server.is_initialized());
        assert!(
            server
                .handle_json_rpc_line(
                    r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"call-1","reason":"user stopped it"}}"#,
                )
                .is_empty()
        );
        assert_eq!(
            server.cancelled_requests(),
            &[CancelledRequest {
                request_id: "call-1".to_string(),
                reason: Some("user stopped it".to_string()),
            }]
        );
    }

    #[test]
    fn cancelled_requests_are_bounded_and_deduplicated() {
        let mut server = McpServer::new();

        for index in 0..(MAX_CANCELLED_REQUESTS + 2) {
            let message = format!(
                r#"{{"jsonrpc":"2.0","method":"notifications/cancelled","params":{{"requestId":"call-{index}"}}}}"#
            );
            assert!(server.handle_json_rpc_line(&message).is_empty());
        }
        assert_eq!(server.cancelled_requests().len(), MAX_CANCELLED_REQUESTS);
        assert_eq!(server.cancelled_requests()[0].request_id, "call-2");

        assert!(
            server
                .handle_json_rpc_line(
                    r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"call-129","reason":"new reason"}}"#,
                )
                .is_empty()
        );
        assert_eq!(server.cancelled_requests().len(), MAX_CANCELLED_REQUESTS);
        assert_eq!(
            server.cancelled_requests()[MAX_CANCELLED_REQUESTS - 1].reason,
            Some("new reason".to_string())
        );
    }

    #[test]
    fn rejects_operation_before_initialize_but_allows_ping() {
        let mut server = McpServer::new();

        let ping =
            decode(&server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)[0]);
        assert!(ping["result"].is_object());

        let tools = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        assert_eq!(tools["error"]["code"], -32002);
    }

    #[test]
    fn tools_list_returns_spec_shaped_descriptors() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));
        assert!(tools.iter().any(|tool| {
            tool["name"] == "tdw.equity.historical" && tool["inputSchema"].is_object()
        }));
        assert_eq!(mcp_tool_catalog().len(), tools.len());
    }

    #[test]
    fn tools_call_fetches_equity_historical_structured_content() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.equity.historical","arguments":{"provider":"fileset","symbol":"aapl"}}}"#,
            )[0],
        );
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["structuredContent"]["provider"],
            "fileset"
        );
        assert_eq!(
            response["result"]["structuredContent"]["rows"][0]["symbol"],
            "AAPL"
        );
        assert_eq!(response["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn progress_tool_emits_notifications_before_response() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let messages = server.handle_json_rpc_line(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tdw.progress.sample","arguments":{"symbol":"aapl"},"_meta":{"progressToken":"progress-1"}}}"#,
        );
        assert!(messages.len() >= 2);
        let first = decode(&messages[0]);
        let last = decode(
            messages
                .last()
                .unwrap_or_else(|| panic!("response should be present")),
        );
        assert_eq!(first["method"], "notifications/progress");
        assert_eq!(first["params"]["progressToken"], "progress-1");
        assert_eq!(last["id"], 4);
        assert_eq!(
            last["result"]["structuredContent"]["events"][0],
            "progress:fetch:0.0"
        );
    }

    #[test]
    fn progress_notifications_allow_new_stage_reset() {
        let events = vec![
            "progress:fetch:0.9".to_string(),
            "progress:parse:0.1".to_string(),
            "done:fileset:2".to_string(),
        ];

        let messages = progress_notifications(Some(json!("progress-2")), &events);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["params"]["message"], "fetch");
        assert_eq!(messages[0]["params"]["progress"], 0.9);
        assert_eq!(messages[1]["params"]["message"], "parse");
        assert_eq!(messages[1]["params"]["progress"], 0.1);
        assert_eq!(messages[2]["params"]["message"], "complete");
        assert_eq!(messages[2]["params"]["progress"], 1.0);
    }

    #[test]
    fn resources_list_and_read_safe_static_resources() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let listed = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":5,"method":"resources/list"}"#)
                [0],
        );
        assert!(
            listed["result"]["resources"]
                .as_array()
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item["uri"] == "tdw://quality/mcp-worker-product-boundaries")
                })
        );

        let read = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":6,"method":"resources/read","params":{"uri":"tdw://quality/mcp-worker-product-boundaries"}}"#,
            )[0],
        );
        assert_eq!(read["result"]["contents"][0]["mimeType"], "text/markdown");
        assert!(
            read["result"]["contents"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("MCP"))
        );
    }

    #[test]
    fn prompts_list_and_get_finance_prompt() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let listed = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":7,"method":"prompts/list"}"#)[0],
        );
        assert!(listed["result"]["prompts"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["name"] == "tdw.equity.research")
        }));

        let prompt = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":8,"method":"prompts/get","params":{"name":"tdw.equity.research","arguments":{"symbol":"MSFT","provider":"fileset","horizon":"30d"}}}"#,
            )[0],
        );
        let text = prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("prompt text should be string"));
        assert!(text.contains("MSFT"));
        assert!(text.contains("tdw.equity.historical"));
    }

    #[test]
    fn reports_parse_and_unknown_method_errors() {
        let malformed_messages = handle_json_rpc_line("{");
        assert_eq!(malformed_messages.len(), 1);
        let malformed = decode(&malformed_messages[0]);
        assert_eq!(malformed["error"]["code"], -32700);

        let mut server = McpServer::new();
        initialize(&mut server);
        let unknown = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":"x","method":"unknown"}"#)[0],
        );
        assert_eq!(unknown["id"], "x");
        assert_eq!(unknown["error"]["code"], -32601);
    }

    #[test]
    fn session_helper_preserves_state_and_all_messages() {
        let messages = handle_json_rpc_lines([
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tdw.progress.sample","arguments":{"symbol":"aapl"},"_meta":{"progressToken":"progress-3"}}}"#,
        ]);

        assert!(messages.len() >= 3);
        assert!(messages.iter().any(|message| {
            let decoded = decode(message);
            decoded["method"] == "notifications/progress"
                && decoded["params"]["progressToken"] == "progress-3"
        }));
        let last = decode(
            messages
                .last()
                .unwrap_or_else(|| panic!("response should be present")),
        );
        assert_eq!(last["id"], 4);
        assert_eq!(last["result"]["isError"], false);
    }

    #[test]
    fn unknown_tool_is_protocol_error_and_execution_error_is_tool_result() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let unknown = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"missing","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(unknown["id"], 9);
        assert_eq!(unknown["error"]["code"], -32602);

        let execution = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"tdw.equity.historical","arguments":{"provider":"missing","symbol":"AAPL"}}}"#,
            )[0],
        );
        assert_eq!(execution["result"]["isError"], true);
        assert!(
            execution["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("unknown provider"))
        );
    }

    #[test]
    fn invalid_prompt_arguments_keep_request_id() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let invalid = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":11,"method":"prompts/get","params":{"name":"tdw.equity.research","arguments":"bad"}}"#,
            )[0],
        );
        assert_eq!(invalid["id"], 11);
        assert_eq!(invalid["error"]["code"], -32602);
    }
}
