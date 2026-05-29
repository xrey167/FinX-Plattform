#![forbid(unsafe_code)]

use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use tdw_app_client::{
    DEFAULT_DAEMON_TCP_ADDR, DaemonClient, DaemonClientConfig, DaemonClientError, DaemonSubmission,
};
use tdw_app_server::{DaemonEndpoint, DaemonTransport};
use tdw_config::{ConfigLayer, ConfigLayerKind, TdwConfig, merge_layers};
use tdw_protocol::{ActorKind, ActorRef, CostHint, EventMsg, Op, OpEnvelope, PlanId, SessionId};

pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub const DEFAULT_STREAMABLE_HTTP_BIND: &str = "127.0.0.1:8788";

const SERVER_NAME: &str = "tdw-mcp";
const SERVER_TITLE: &str = "TDW MCP Server";
const MAX_CANCELLED_REQUESTS: usize = 128;
const STREAMABLE_HTTP_PATH: &str = "/mcp";
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
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

pub struct McpServer {
    initialized: bool,
    client_info: Option<Value>,
    cancelled_requests: Vec<CancelledRequest>,
    daemon: DaemonToolRuntime,
}

impl Default for McpServer {
    fn default() -> Self {
        Self {
            initialized: false,
            client_info: None,
            cancelled_requests: Vec::new(),
            daemon: DaemonToolRuntime::from_env(),
        }
    }
}

impl McpServer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_daemon_config(config: DaemonClientConfig) -> Self {
        Self {
            initialized: false,
            client_info: None,
            cancelled_requests: Vec::new(),
            daemon: DaemonToolRuntime::configured(config),
        }
    }

    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub const fn client_info(&self) -> Option<&Value> {
        self.client_info.as_ref()
    }

    #[must_use]
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
                    "TDW exposes deterministic offline tools plus explicitly daemon-backed query and triage tools over MCP stdio or Streamable HTTP. Requested protocol: {requested}."
                ),
            }),
        )
    }

    fn call_tool(&self, id: Value, params: Value) -> Vec<Value> {
        let Some(params_object) = params.as_object() else {
            return vec![error_message(JsonRpcProblem::new(
                id,
                -32602,
                "tools/call params must be an object",
            ))];
        };
        let Some(name) = string_field(params_object, "name") else {
            return vec![error_message(JsonRpcProblem::new(
                id,
                -32602,
                "tools/call requires string field: name",
            ))];
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
        let result = execute_tool(&self.daemon, name, &arguments);
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
        let Some(params_object) = params.as_object() else {
            return error_message(JsonRpcProblem::new(
                id,
                -32602,
                "resources/read params must be an object",
            ));
        };
        let Some(uri) = string_field(params_object, "uri") else {
            return error_message(JsonRpcProblem::new(
                id,
                -32602,
                "resources/read requires string field: uri",
            ));
        };

        match resource_content(uri) {
            Ok(content) => success_message(id, json!({ "contents": [content] })),
            Err(problem) => error_message(problem.with_id(id).with_data(json!({ "uri": uri }))),
        }
    }

    fn get_prompt(&self, id: Value, params: Value) -> Value {
        let Some(params_object) = params.as_object() else {
            return error_message(JsonRpcProblem::new(
                id,
                -32602,
                "prompts/get params must be an object",
            ));
        };
        let Some(name) = string_field(params_object, "name") else {
            return error_message(JsonRpcProblem::new(
                id,
                -32602,
                "prompts/get requires string field: name",
            ));
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

#[must_use]
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

#[must_use]
pub fn handle_json_rpc_line(line: &str) -> Vec<String> {
    handle_json_rpc_lines([line])
}

/// Fuzz shim: feed arbitrary bytes through the JSON-RPC line handler.
///
/// Must never panic on adversarial input; malformed lines must produce error
/// responses rather than panics. Shared with the nightly cargo-fuzz target.
#[doc(hidden)]
pub fn __fuzz_mcp_jsonrpc(data: &[u8]) {
    let line = String::from_utf8_lossy(data);
    let _ = handle_json_rpc_line(&line);
}

/// Fuzz shim: feed arbitrary bytes as the body of a Streamable HTTP request.
///
/// Must never panic on adversarial input; malformed requests must yield an
/// error response rather than a panic. Shared with the nightly cargo-fuzz
/// target.
#[doc(hidden)]
pub fn __fuzz_mcp_http(data: &[u8]) {
    let mut server = McpServer::new();
    let request =
        StreamableHttpRequest::new("POST", STREAMABLE_HTTP_PATH, Vec::new(), data.to_vec());
    let _ = handle_streamable_http_request_with_config(
        &mut server,
        request,
        &StreamableHttpConfig::new(),
    );
}

#[must_use]
pub fn mcp_tool_catalog() -> Vec<String> {
    tool_descriptors()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

#[derive(Clone, Debug)]
struct DaemonToolRuntime {
    config: Result<DaemonClientConfig, String>,
}

impl DaemonToolRuntime {
    fn from_env() -> Self {
        match daemon_client_config_from_env() {
            Ok(config) => Self::configured(config),
            Err(error) => Self { config: Err(error) },
        }
    }

    fn configured(config: DaemonClientConfig) -> Self {
        Self { config: Ok(config) }
    }

    fn submit(&self, envelope: OpEnvelope) -> Result<DaemonSubmission, String> {
        let config = self.config.as_ref().map_err(Clone::clone)?;
        DaemonClient::new(config.clone())
            .submit_and_wait(envelope)
            .map_err(|error| daemon_client_error_message(config, error))
    }
}

fn daemon_client_error_message(config: &DaemonClientConfig, error: DaemonClientError) -> String {
    format!(
        "{error}; endpoint={}://{}",
        daemon_transport_label(config.endpoint().transport),
        config.endpoint().address
    )
}

fn daemon_client_config_from_env() -> Result<DaemonClientConfig, String> {
    let config = tdw_config_from_env()?;
    daemon_client_config_from_sources(
        config,
        non_empty_env("TDW_MCP_DAEMON_TRANSPORT"),
        non_empty_env("TDW_MCP_DAEMON_ADDR").or_else(|| non_empty_env("TDW_DAEMON_TCP_BIND")),
        non_empty_env("TDW_MCP_DAEMON_TIMEOUT_MS"),
    )
}

fn tdw_config_from_env() -> Result<TdwConfig, String> {
    let mut layers = Vec::new();
    if let Some(path) = non_empty_env("TDW_CONFIG") {
        let contents = std::fs::read_to_string(&path)
            .map_err(|error| format!("TDW_CONFIG read failed for {path}: {error}"))?;
        layers.push(
            ConfigLayer::from_toml(ConfigLayerKind::EnvFile, "TDW_CONFIG", &contents)
                .map_err(|error| error.to_string())?,
        );
    }
    if let Some(contents) = non_empty_env("TDW_CONFIG_CONTENT") {
        layers.push(
            ConfigLayer::from_toml(ConfigLayerKind::InlineEnv, "TDW_CONFIG_CONTENT", &contents)
                .map_err(|error| error.to_string())?,
        );
    }

    if layers.is_empty() {
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Tcp;
        config.daemon.tcp_bind = Some(DEFAULT_DAEMON_TCP_ADDR.to_string());
        return Ok(config);
    }

    merge_layers(&layers).map_err(|error| error.to_string())
}

fn daemon_client_config_from_sources(
    config: TdwConfig,
    transport_override: Option<String>,
    address_override: Option<String>,
    timeout_ms: Option<String>,
) -> Result<DaemonClientConfig, String> {
    let transport = match transport_override {
        Some(value) => parse_daemon_transport(&value)?,
        None => config.daemon.transport,
    };
    let address = match address_override {
        Some(value) => value,
        None => daemon_endpoint_address_from_config(&config, transport),
    };
    let timeout = timeout_ms
        .as_deref()
        .map(parse_timeout_ms)
        .transpose()?
        .unwrap_or_else(|| Duration::from_secs(2));

    let client_config =
        DaemonClientConfig::new(DaemonEndpoint { transport, address }).with_timeout(timeout);
    client_config
        .validate()
        .map_err(|error| format!("invalid daemon client config: {error}"))?;
    Ok(client_config)
}

fn daemon_endpoint_address_from_config(config: &TdwConfig, transport: DaemonTransport) -> String {
    match transport {
        DaemonTransport::Tcp => config
            .daemon
            .tcp_bind
            .clone()
            .unwrap_or_else(|| DEFAULT_DAEMON_TCP_ADDR.to_string()),
        DaemonTransport::Uds => config.daemon.uds_path.clone(),
        DaemonTransport::HttpSse => config.daemon.http_bind.as_deref().map_or_else(
            || "http://127.0.0.1:7879/events".to_string(),
            |bind| {
                if bind.starts_with("http://") || bind.starts_with("https://") {
                    bind.to_string()
                } else {
                    format!("http://{bind}/events")
                }
            },
        ),
    }
}

fn parse_daemon_transport(value: &str) -> Result<DaemonTransport, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tcp" => Ok(DaemonTransport::Tcp),
        "uds" | "unix" => Ok(DaemonTransport::Uds),
        "http" | "http-sse" | "httpsse" => Ok(DaemonTransport::HttpSse),
        other => Err(format!("unknown daemon transport: {other}")),
    }
}

fn parse_timeout_ms(value: &str) -> Result<Duration, String> {
    let millis = value
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid TDW_MCP_DAEMON_TIMEOUT_MS: {error}"))?;
    if millis == 0 {
        return Err("TDW_MCP_DAEMON_TIMEOUT_MS must be greater than zero".to_string());
    }
    Ok(Duration::from_millis(millis))
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamableHttpConfig {
    auth_token: Option<String>,
}

impl StreamableHttpConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        let token = token.into();
        if !token.is_empty() {
            self.auth_token = Some(token);
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamableHttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl StreamableHttpRequest {
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        headers: Vec<(String, String)>,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            headers,
            body: body.into(),
        }
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamableHttpResponse {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct ParsedHttpHead {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl StreamableHttpResponse {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        header_value(&self.headers, name)
    }

    #[must_use]
    pub fn body_text(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }
}

#[must_use]
pub const fn default_streamable_http_bind() -> &'static str {
    DEFAULT_STREAMABLE_HTTP_BIND
}

#[must_use]
pub fn run_streamable_http(bind: &str) -> i32 {
    if !bind_is_loopback(bind) && std::env::var("TDW_MCP_HTTP_TOKEN").is_err() {
        eprintln!(
            "tdw-mcp refusing non-loopback bind {bind}; set TDW_MCP_HTTP_TOKEN to enable authenticated remote binding"
        );
        return 2;
    }

    let listener = match TcpListener::bind(bind) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("tdw-mcp Streamable HTTP bind failed on {bind}: {error}");
            return 1;
        }
    };
    eprintln!("tdw-mcp Streamable HTTP listening on http://{bind}{STREAMABLE_HTTP_PATH}");

    let server = Arc::new(Mutex::new(McpServer::new()));
    let config = Arc::new(streamable_http_config_from_env());
    for accepted in listener.incoming() {
        match accepted {
            Ok(stream) => {
                let server = Arc::clone(&server);
                let config = Arc::clone(&config);
                if let Err(error) = std::thread::Builder::new()
                    .name("tdw-mcp-http".to_string())
                    .spawn(move || {
                        if let Err(error) =
                            handle_streamable_http_connection(stream, &server, &config)
                        {
                            eprintln!("tdw-mcp Streamable HTTP connection error: {error}");
                        }
                    })
                {
                    eprintln!("tdw-mcp Streamable HTTP worker spawn failed: {error}");
                    return 1;
                }
            }
            Err(error) => {
                eprintln!("tdw-mcp Streamable HTTP accept failed: {error}");
                return 1;
            }
        }
    }

    0
}

#[must_use]
pub fn run_streamable_http_smoke() -> i32 {
    let mut server = McpServer::new();
    let initialize = StreamableHttpRequest::new(
        "POST",
        STREAMABLE_HTTP_PATH,
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "application/json".to_string()),
        ],
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{MCP_PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"tdw-smoke","version":"1.0.0"}}}}}}"#
        ),
    );
    let initialized = handle_streamable_http_request(&mut server, initialize);
    if initialized.status != 200 {
        eprintln!(
            "tdw-mcp Streamable HTTP smoke initialize failed: {} {}",
            initialized.status, initialized.reason
        );
        return 1;
    }

    let tool_call = StreamableHttpRequest::new(
        "POST",
        STREAMABLE_HTTP_PATH,
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "text/event-stream".to_string()),
        ],
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tdw.progress.sample","arguments":{"symbol":"AAPL"},"_meta":{"progressToken":"smoke-progress"}}}"#,
    );
    let response = handle_streamable_http_request(&mut server, tool_call);
    let body = response.body_text().unwrap_or("");
    if response.status == 200
        && response.header("content-type") == Some("text/event-stream")
        && body.contains("notifications/progress")
        && body.contains("\"id\":2")
    {
        println!(
            "tdw-mcp streamable-http-smoke status=ok endpoint={STREAMABLE_HTTP_PATH} protocol={MCP_PROTOCOL_VERSION}"
        );
        return 0;
    }

    eprintln!(
        "tdw-mcp Streamable HTTP smoke tool call failed: {} {}",
        response.status, response.reason
    );
    1
}

pub fn handle_streamable_http_request(
    server: &mut McpServer,
    request: StreamableHttpRequest,
) -> StreamableHttpResponse {
    handle_streamable_http_request_with_config(server, request, &StreamableHttpConfig::new())
}

pub fn handle_streamable_http_request_with_config(
    server: &mut McpServer,
    request: StreamableHttpRequest,
    config: &StreamableHttpConfig,
) -> StreamableHttpResponse {
    if request.path != STREAMABLE_HTTP_PATH {
        return text_response(404, "Not Found", "unknown MCP endpoint");
    }

    if let Some(origin) = request.header("origin")
        && !origin_is_allowed(origin)
    {
        return text_response(403, "Forbidden", "forbidden Origin");
    }

    if let Some(protocol_version) = request.header("mcp-protocol-version")
        && protocol_version.trim() != MCP_PROTOCOL_VERSION
    {
        return text_response(400, "Bad Request", "unsupported MCP-Protocol-Version");
    }

    if !request_is_authorized(&request, config) {
        let mut response = text_response(401, "Unauthorized", "missing or invalid bearer token");
        response.headers.push((
            "WWW-Authenticate".to_string(),
            "Bearer realm=\"tdw-mcp\"".to_string(),
        ));
        return response;
    }

    match request.method.as_str() {
        "OPTIONS" => {
            let mut response = empty_response(204, "No Content");
            response
                .headers
                .push(("Allow".to_string(), "GET, POST, OPTIONS".to_string()));
            response.headers.push((
                "Access-Control-Allow-Headers".to_string(),
                "Accept, Authorization, Content-Type, MCP-Protocol-Version".to_string(),
            ));
            response.headers.push((
                "Access-Control-Allow-Methods".to_string(),
                "GET, POST, OPTIONS".to_string(),
            ));
            attach_protocol_headers(&mut response);
            attach_cors_origin(&mut response, &request);
            response
        }
        "GET" if accepts_sse(&request) => {
            let mut response = bytes_response(
                200,
                "OK",
                "text/event-stream",
                b": tdw-mcp stream ready\n\n".to_vec(),
            );
            response
                .headers
                .push(("Cache-Control".to_string(), "no-cache".to_string()));
            attach_protocol_headers(&mut response);
            attach_cors_origin(&mut response, &request);
            response
        }
        "GET" => method_not_allowed(),
        "POST" => handle_streamable_http_post(server, request),
        _ => method_not_allowed(),
    }
}

fn handle_streamable_http_post(
    server: &mut McpServer,
    request: StreamableHttpRequest,
) -> StreamableHttpResponse {
    if request.body.len() > MAX_HTTP_BODY_BYTES {
        return text_response(413, "Payload Too Large", "request body too large");
    }
    if !request
        .header("content-type")
        .is_some_and(content_type_is_json)
    {
        return text_response(415, "Unsupported Media Type", "expected application/json");
    }
    let Ok(body) = std::str::from_utf8(&request.body) else {
        return text_response(400, "Bad Request", "request body must be UTF-8 JSON");
    };

    let messages = server.handle_json_rpc_line(body);
    if messages.is_empty() {
        let mut response = empty_response(202, "Accepted");
        attach_protocol_headers(&mut response);
        attach_cors_origin(&mut response, &request);
        return response;
    }

    let mut response = if accepts_sse(&request) {
        let body = encode_sse_messages(&messages);
        let mut response = bytes_response(200, "OK", "text/event-stream", body.into_bytes());
        response
            .headers
            .push(("Cache-Control".to_string(), "no-cache".to_string()));
        response
    } else {
        bytes_response(
            200,
            "OK",
            "application/json",
            encode_json_messages(&messages).into_bytes(),
        )
    };
    attach_protocol_headers(&mut response);
    attach_cors_origin(&mut response, &request);
    response
}

fn handle_streamable_http_connection(
    mut stream: TcpStream,
    server: &Arc<Mutex<McpServer>>,
    config: &StreamableHttpConfig,
) -> std::io::Result<()> {
    let response = match read_streamable_http_request(&mut stream) {
        Ok(request) => match server.lock() {
            Ok(mut server) => {
                handle_streamable_http_request_with_config(&mut server, request, config)
            }
            Err(_) => text_response(
                500,
                "Internal Server Error",
                "MCP server state lock poisoned",
            ),
        },
        Err(response) => response,
    };
    write_streamable_http_response(&mut stream, &response)
}

fn read_streamable_http_request(
    stream: &mut TcpStream,
) -> Result<StreamableHttpRequest, StreamableHttpResponse> {
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 1024];
    let header_end = loop {
        if buffer.len() > MAX_HTTP_HEADER_BYTES {
            return Err(text_response(
                431,
                "Request Header Fields Too Large",
                "headers too large",
            ));
        }
        let read = stream
            .read(&mut scratch)
            .map_err(|_| text_response(400, "Bad Request", "could not read request"))?;
        if read == 0 {
            return Err(text_response(400, "Bad Request", "empty request"));
        }
        buffer.extend_from_slice(&scratch[..read]);
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
    };

    let header_bytes = &buffer[..header_end];
    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|_| text_response(400, "Bad Request", "headers must be UTF-8"))?;
    let head = parse_http_header_section(header_text)?;
    let content_length = content_length(&head.headers)?;
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(text_response(
            413,
            "Payload Too Large",
            "request body too large",
        ));
    }

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let mut next = vec![0_u8; content_length - body.len()];
        let read = stream
            .read(&mut next)
            .map_err(|_| text_response(400, "Bad Request", "could not read request body"))?;
        if read == 0 {
            return Err(text_response(400, "Bad Request", "incomplete request body"));
        }
        body.extend_from_slice(&next[..read]);
    }
    body.truncate(content_length);

    Ok(StreamableHttpRequest::new(
        head.method,
        head.path,
        head.headers,
        body,
    ))
}

fn write_streamable_http_response(
    stream: &mut TcpStream,
    response: &StreamableHttpResponse,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, response.reason);
    let has_content_length = response
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-length"));
    let has_connection = response
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("connection"));

    for (name, value) in &response.headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    if !has_content_length {
        head.push_str("Content-Length: ");
        head.push_str(&response.body.len().to_string());
        head.push_str("\r\n");
    }
    if !has_connection {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");

    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn parse_http_header_section(header_text: &str) -> Result<ParsedHttpHead, StreamableHttpResponse> {
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| text_response(400, "Bad Request", "missing request line"))?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| text_response(400, "Bad Request", "missing method"))?;
    let path = parts
        .next()
        .ok_or_else(|| text_response(400, "Bad Request", "missing path"))?;
    let version = parts
        .next()
        .ok_or_else(|| text_response(400, "Bad Request", "missing HTTP version"))?;
    if version != "HTTP/1.1" {
        return Err(text_response(
            505,
            "HTTP Version Not Supported",
            "expected HTTP/1.1",
        ));
    }
    if parts.next().is_some() {
        return Err(text_response(400, "Bad Request", "invalid request line"));
    }

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(text_response(400, "Bad Request", "invalid header"));
        };
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    Ok(ParsedHttpHead {
        method: method.to_string(),
        path: path.to_string(),
        headers,
    })
}

fn streamable_http_config_from_env() -> StreamableHttpConfig {
    match std::env::var("TDW_MCP_HTTP_TOKEN") {
        Ok(token) => StreamableHttpConfig::new().with_auth_token(token),
        Err(_) => StreamableHttpConfig::new(),
    }
}

fn request_is_authorized(request: &StreamableHttpRequest, config: &StreamableHttpConfig) -> bool {
    let Some(token) = config.auth_token.as_deref() else {
        return true;
    };
    let Some(value) = request.header("authorization") else {
        return false;
    };
    let mut parts = value.splitn(2, ' ');
    let Some(scheme) = parts.next() else {
        return false;
    };
    let Some(candidate) = parts.next() else {
        return false;
    };
    scheme.eq_ignore_ascii_case("bearer") && candidate == token
}

fn bind_is_loopback(bind: &str) -> bool {
    let host = bind
        .rsplit_once(':')
        .map_or(bind, |(host, _)| host)
        .trim_matches(['[', ']']);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn origin_is_allowed(origin: &str) -> bool {
    let origin = origin.trim().to_ascii_lowercase();
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn content_type_is_json(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

fn accepts_sse(request: &StreamableHttpRequest) -> bool {
    request.header("accept").is_some_and(|accept| {
        accept.split(',').any(|entry| {
            entry
                .split(';')
                .next()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
        })
    })
}

fn content_length(headers: &[(String, String)]) -> Result<usize, StreamableHttpResponse> {
    let Some(value) = header_value(headers, "content-length") else {
        return Ok(0);
    };
    value
        .parse::<usize>()
        .map_err(|_| text_response(400, "Bad Request", "invalid Content-Length"))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn encode_json_messages(messages: &[String]) -> String {
    if messages.len() == 1 {
        return messages[0].clone();
    }
    let values = messages
        .iter()
        .map(|message| {
            serde_json::from_str::<Value>(message).unwrap_or(Value::String(message.clone()))
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values).unwrap_or_else(|error| {
        encode_message(&error_message(JsonRpcProblem::new(
            Value::Null,
            -32603,
            format!("serialize error: {error}"),
        )))
    })
}

fn encode_sse_messages(messages: &[String]) -> String {
    let mut encoded = String::new();
    for message in messages {
        encoded.push_str("event: message\n");
        encoded.push_str("data: ");
        encoded.push_str(message);
        encoded.push_str("\n\n");
    }
    encoded
}

fn method_not_allowed() -> StreamableHttpResponse {
    let mut response = text_response(405, "Method Not Allowed", "method not allowed");
    response
        .headers
        .push(("Allow".to_string(), "GET, POST, OPTIONS".to_string()));
    attach_protocol_headers(&mut response);
    response
}

fn text_response(status: u16, reason: &str, body: &str) -> StreamableHttpResponse {
    bytes_response(
        status,
        reason,
        "text/plain; charset=utf-8",
        body.as_bytes().to_vec(),
    )
}

fn empty_response(status: u16, reason: &str) -> StreamableHttpResponse {
    StreamableHttpResponse {
        status,
        reason: reason.to_string(),
        headers: Vec::new(),
        body: Vec::new(),
    }
}

fn bytes_response(
    status: u16,
    reason: &str,
    content_type: &str,
    body: Vec<u8>,
) -> StreamableHttpResponse {
    let mut response = empty_response(status, reason);
    response
        .headers
        .push(("Content-Type".to_string(), content_type.to_string()));
    response.body = body;
    response
}

fn attach_protocol_headers(response: &mut StreamableHttpResponse) {
    response.headers.push((
        "MCP-Protocol-Version".to_string(),
        MCP_PROTOCOL_VERSION.to_string(),
    ));
}

fn attach_cors_origin(response: &mut StreamableHttpResponse, request: &StreamableHttpRequest) {
    if let Some(origin) = request.header("origin")
        && origin_is_allowed(origin)
    {
        response.headers.push((
            "Access-Control-Allow-Origin".to_string(),
            origin.to_string(),
        ));
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
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
        daemon_tool(
            "tdw.daemon.triage",
            "Daemon Operation Triage",
            "Submit a bounded diagnostic query through the configured TDW daemon and return event-spine evidence for the operation.",
            json!({
                "type": "object",
                "properties": {
                    "op_id": { "type": "string", "description": "Optional external operation id to include in the triage evidence." },
                    "session_id": { "type": "string", "description": "Optional TDW session id, defaults to session-mcp-daemon." },
                    "sequence": { "type": "integer", "minimum": 1, "description": "Optional operation sequence, defaults to 1." }
                },
                "additionalProperties": false
            }),
        ),
        daemon_tool(
            "tdw.daemon.query.submit",
            "Submit Daemon Query",
            "Submit a RunQuery operation through the configured TDW daemon and wait for the terminal event.",
            json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "description": "SQL query to submit to the daemon request path." },
                    "session_id": { "type": "string", "description": "Optional TDW session id, defaults to session-mcp-daemon." },
                    "sequence": { "type": "integer", "minimum": 1, "description": "Optional operation sequence, defaults to 1." },
                    "plan_id": { "type": "string", "description": "Optional plan id attached to the RunQuery op." }
                },
                "required": ["sql"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    tool_with_annotations(name, title, description, input_schema, true, true)
}

fn daemon_tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    tool_with_annotations(name, title, description, input_schema, false, false)
}

fn tool_with_annotations(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    idempotent: bool,
) -> ToolDescriptor {
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
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
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

fn execute_tool(
    daemon: &DaemonToolRuntime,
    name: &str,
    arguments: &Value,
) -> Result<ToolExecution, ToolFailure> {
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
        "tdw.daemon.triage" => execute_daemon_triage(daemon, arguments_object),
        "tdw.daemon.query.submit" => execute_daemon_query_submit(daemon, arguments_object),
        _ => Err(ToolFailure::Protocol(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("unknown tool: {name}"),
        ))),
    }
}

fn execute_daemon_triage(
    daemon: &DaemonToolRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let requested_op_id = optional_argument(arguments, "op_id").map(ToString::to_string);
    let sql = "select 'tdw.daemon.triage' as diagnostic_check";
    let envelope = daemon_run_query_envelope(arguments, sql)?;
    let submission = daemon.submit(envelope).map_err(ToolFailure::Execution)?;
    Ok(structured(daemon_submission_value(
        "tdw.daemon.triage",
        &submission,
        json!({
            "requested_op_id": requested_op_id,
            "diagnostic_sql": sql,
            "checks": [
                "daemon connection accepted the OpEnvelope",
                "event stream emitted a terminal event for the submitted op",
                "service policy and relational dispatch path returned structured evidence"
            ]
        }),
    )))
}

fn execute_daemon_query_submit(
    daemon: &DaemonToolRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let sql = required_argument(arguments, "sql")?;
    validate_daemon_sql(sql)?;
    let envelope = daemon_run_query_envelope(arguments, sql)?;
    let submission = daemon.submit(envelope).map_err(ToolFailure::Execution)?;
    Ok(structured(daemon_submission_value(
        "tdw.daemon.query.submit",
        &submission,
        json!({
            "sql": sql,
        }),
    )))
}

fn daemon_run_query_envelope(
    arguments: &Map<String, Value>,
    sql: &str,
) -> Result<OpEnvelope, ToolFailure> {
    let session_id = optional_argument(arguments, "session_id").unwrap_or("session-mcp-daemon");
    let session_id = SessionId::new(session_id.to_string())
        .map_err(|error| protocol_argument_failure(error.to_string()))?;
    let sequence = optional_u64_argument(arguments, "sequence")?.unwrap_or(1);
    let plan_id = optional_argument(arguments, "plan_id")
        .map(|value| PlanId::new(value.to_string()))
        .transpose()
        .map_err(|error| protocol_argument_failure(error.to_string()))?;

    Ok(OpEnvelope::new(
        session_id,
        sequence,
        ActorRef {
            actor_id: "mcp:tdw-mcp".to_string(),
            kind: ActorKind::Service,
            tenant_id: Some("default".to_string()),
        },
        Op::RunQuery {
            sql: sql.to_string(),
            plan_id,
            cost_hint: Some(CostHint {
                backend: "tdw-daemon".to_string(),
                estimated_bytes_scanned: None,
                estimated_rows_read: None,
            }),
        },
    ))
}

fn daemon_submission_value(tool: &str, submission: &DaemonSubmission, extra: Value) -> Value {
    let terminal_event = submission
        .events
        .last()
        .cloned()
        .unwrap_or_else(|| EventMsg::Failed {
            op_id: tdw_protocol::OpId::generated(),
            error: "missing terminal event".to_string(),
        });
    json!({
        "tool": tool,
        "daemon": {
            "transport": daemon_transport_label(submission.endpoint.transport),
            "address": submission.endpoint.address.clone(),
        },
        "submitted_op_id": submission.op_id,
        "events": submission.events.clone(),
        "terminal_event": terminal_event,
        "extra": extra,
    })
}

fn daemon_transport_label(transport: DaemonTransport) -> &'static str {
    match transport {
        DaemonTransport::Tcp => "tcp",
        DaemonTransport::Uds => "uds",
        DaemonTransport::HttpSse => "http-sse",
    }
}

const fn structured(structured: Value) -> ToolExecution {
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

fn optional_u64_argument(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<u64>, ToolFailure> {
    let Some(value) = arguments.get(name) else {
        return Ok(None);
    };
    match value {
        Value::Number(number) => positive_u64(number.as_u64(), name),
        Value::String(value) => positive_u64(
            value.parse::<u64>().map(Some).map_err(|error| {
                protocol_argument_failure(format!("{name} must be a positive integer: {error}"))
            })?,
            name,
        ),
        _ => Err(protocol_argument_failure(format!(
            "{name} must be a positive integer"
        ))),
    }
}

fn positive_u64(value: Option<u64>, name: &str) -> Result<Option<u64>, ToolFailure> {
    match value {
        Some(value) if value > 0 => Ok(Some(value)),
        _ => Err(protocol_argument_failure(format!(
            "{name} must be a positive integer"
        ))),
    }
}

fn validate_daemon_sql(sql: &str) -> Result<(), ToolFailure> {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return Err(protocol_argument_failure(
            "sql must not be empty".to_string(),
        ));
    }
    if trimmed.len() > 4096 {
        return Err(protocol_argument_failure(
            "sql must not exceed 4096 bytes".to_string(),
        ));
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(protocol_argument_failure(
            "sql must not contain control characters".to_string(),
        ));
    }
    Ok(())
}

fn protocol_argument_failure(message: String) -> ToolFailure {
    ToolFailure::Protocol(JsonRpcProblem::new(Value::Null, -32602, message))
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
    fn daemon_config_uses_config_and_env_style_overrides() {
        let mut config = TdwConfig::default();
        config.daemon.tcp_bind = Some("127.0.0.1:9001".to_string());

        let from_config =
            daemon_client_config_from_sources(config, None, None, Some("75".to_string()))
                .unwrap_or_else(|error| panic!("config override should resolve: {error}"));
        assert_eq!(from_config.endpoint().address, "127.0.0.1:9001");
        assert_eq!(from_config.timeout(), Duration::from_millis(75));

        let from_env_style = daemon_client_config_from_sources(
            TdwConfig::default(),
            Some("tcp".to_string()),
            Some("127.0.0.1:9002".to_string()),
            None,
        )
        .unwrap_or_else(|error| panic!("env-style override should resolve: {error}"));
        assert_eq!(from_env_style.endpoint().transport, DaemonTransport::Tcp);
        assert_eq!(from_env_style.endpoint().address, "127.0.0.1:9002");
    }

    #[test]
    fn daemon_backed_tool_fails_closed_when_daemon_unavailable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap_or_else(|error| panic!("reserve local port: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("reserved listener address: {error}"));
        drop(listener);

        let mut server = McpServer::with_daemon_config(
            DaemonClientConfig::tcp(addr.to_string()).with_timeout(Duration::from_millis(100)),
        );
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"tdw.daemon.query.submit","arguments":{"sql":"select 1"}}}"#,
            )[0],
        );

        assert_eq!(response["id"], 12);
        assert_eq!(response["result"]["isError"], true);
        let error_text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("daemon error text");
        assert!(
            error_text.contains("daemon unavailable")
                || error_text.contains("daemon timed out during connect"),
            "unexpected daemon error text: {error_text}"
        );
        assert!(error_text.contains(&format!("endpoint=tcp://{addr}")));
    }

    #[test]
    fn daemon_query_submit_roundtrips_against_in_process_tcp_daemon() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("tokio runtime: {error}"));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap_or_else(|error| panic!("bind in-process daemon: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("daemon local addr: {error}"));
        let state = runtime.block_on(tdw_service_api::AppState::in_memory_for_tests());
        let (handle, events_rx, mut service_loop) =
            tdw_app_server::service_channel(state.clone(), state);
        let cancel = tdw_app_server::CancellationToken::new();

        let loop_cancel = cancel.clone();
        let loop_task = runtime.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    () = loop_cancel.cancelled() => break,
                    maybe = service_loop.run_once() => {
                        if maybe.is_none() {
                            break;
                        }
                    }
                }
            }
        });
        let tcp_cancel = cancel.clone();
        let tcp_task = runtime.spawn(async move {
            tdw_app_server::serve_tcp(listener, handle, events_rx, tcp_cancel).await
        });

        let mut server = McpServer::with_daemon_config(
            DaemonClientConfig::tcp(addr.to_string()).with_timeout(Duration::from_secs(2)),
        );
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"tdw.daemon.query.submit","arguments":{"sql":"select 1","session_id":"session-mcp-test"}}}"#,
            )[0],
        );

        cancel.cancel();
        runtime.block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(1), loop_task).await;
            let _ = tokio::time::timeout(Duration::from_secs(1), tcp_task).await;
        });

        assert_eq!(response["id"], 13);
        assert_eq!(response["result"]["isError"], false);
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["tool"], "tdw.daemon.query.submit");
        assert_eq!(structured["daemon"]["transport"], "tcp");
        assert_eq!(structured["extra"]["sql"], "select 1");
        assert!(
            structured["events"]
                .as_array()
                .is_some_and(|events| events.iter().any(|event| event["type"] == "completed")),
        );
    }

    #[test]
    fn daemon_query_submit_roundtrips_against_in_process_http_sse_daemon() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|error| panic!("tokio runtime: {error}"));
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap_or_else(|error| panic!("bind in-process HTTP/SSE daemon: {error}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|error| panic!("daemon local addr: {error}"));
        let state = runtime.block_on(tdw_service_api::AppState::in_memory_for_tests());
        let (handle, events_rx, mut service_loop) =
            tdw_app_server::service_channel(state.clone(), state);
        let cancel = tdw_app_server::CancellationToken::new();

        let loop_cancel = cancel.clone();
        let loop_task = runtime.spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    () = loop_cancel.cancelled() => break,
                    maybe = service_loop.run_once() => {
                        if maybe.is_none() {
                            break;
                        }
                    }
                }
            }
        });
        let http_cancel = cancel.clone();
        let http_task = runtime.spawn(async move {
            tdw_app_server::serve_http(listener, handle, events_rx, http_cancel).await
        });

        let mut server = McpServer::with_daemon_config(
            DaemonClientConfig::new(DaemonEndpoint {
                transport: DaemonTransport::HttpSse,
                address: format!("http://{addr}/events"),
            })
            .with_timeout(Duration::from_secs(2)),
        );
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"tdw.daemon.query.submit","arguments":{"sql":"select 1","session_id":"session-mcp-http-test"}}}"#,
            )[0],
        );

        cancel.cancel();
        runtime.block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(1), loop_task).await;
            let _ = tokio::time::timeout(Duration::from_secs(1), http_task).await;
        });

        assert_eq!(response["id"], 14);
        assert_eq!(response["result"]["isError"], false);
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["tool"], "tdw.daemon.query.submit");
        assert_eq!(structured["daemon"]["transport"], "http-sse");
        assert_eq!(structured["extra"]["sql"], "select 1");
        assert!(
            structured["events"]
                .as_array()
                .is_some_and(|events| events.iter().any(|event| event["type"] == "completed")),
        );
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
    fn streamable_http_post_initialize_returns_json_response() {
        let mut server = McpServer::new();
        let response = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Origin".to_string(), "http://localhost:3000".to_string()),
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Accept".to_string(), "application/json".to_string()),
                    (
                        "MCP-Protocol-Version".to_string(),
                        MCP_PROTOCOL_VERSION.to_string(),
                    ),
                ],
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"http-test","version":"1.0.0"}}}"#,
            ),
        );

        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("application/json"));
        assert_eq!(
            response.header("mcp-protocol-version"),
            Some(MCP_PROTOCOL_VERSION)
        );
        assert_eq!(
            response.header("access-control-allow-origin"),
            Some("http://localhost:3000")
        );
        let body = response
            .body_text()
            .unwrap_or_else(|| panic!("HTTP response body should be text"));
        let decoded = decode(body);
        assert_eq!(decoded["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(server.is_initialized());
    }

    #[test]
    fn streamable_http_notifications_return_accepted_without_body() {
        let mut server = McpServer::new();
        let response = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![("Content-Type".to_string(), "application/json".to_string())],
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            ),
        );

        assert_eq!(response.status, 202);
        assert!(response.body.is_empty());
        assert_eq!(
            response.header("mcp-protocol-version"),
            Some(MCP_PROTOCOL_VERSION)
        );
        assert!(server.is_initialized());
    }

    #[test]
    fn streamable_http_rejects_bad_origin_protocol_and_auth() {
        let mut server = McpServer::new();
        let bad_origin = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Origin".to_string(), "https://example.com".to_string()),
                    ("Content-Type".to_string(), "application/json".to_string()),
                ],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
        );
        assert_eq!(bad_origin.status, 403);

        let bad_protocol = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("MCP-Protocol-Version".to_string(), "2024-11-05".to_string()),
                ],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
        );
        assert_eq!(bad_protocol.status, 400);

        let config = StreamableHttpConfig::new().with_auth_token("secret");
        let unauthorized = handle_streamable_http_request_with_config(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![("Content-Type".to_string(), "application/json".to_string())],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
            &config,
        );
        assert_eq!(unauthorized.status, 401);
        assert_eq!(
            unauthorized.header("www-authenticate"),
            Some("Bearer realm=\"tdw-mcp\"")
        );

        let authorized = handle_streamable_http_request_with_config(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Authorization".to_string(), "Bearer secret".to_string()),
                ],
                r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
            ),
            &config,
        );
        assert_eq!(authorized.status, 200);
    }

    #[test]
    fn streamable_http_sse_streams_progress_before_response() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Accept".to_string(), "text/event-stream".to_string()),
                ],
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tdw.progress.sample","arguments":{"symbol":"aapl"},"_meta":{"progressToken":"progress-http"}}}"#,
            ),
        );

        assert_eq!(response.status, 200);
        assert_eq!(response.header("content-type"), Some("text/event-stream"));
        let body = response
            .body_text()
            .unwrap_or_else(|| panic!("SSE response body should be text"));
        assert!(body.contains("event: message"));
        assert!(body.contains("notifications/progress"));
        assert!(body.contains("progress-http"));
        assert!(body.contains("\"id\":4"));
    }

    #[test]
    fn streamable_http_json_mode_preserves_multi_message_tool_output() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Accept".to_string(), "application/json".to_string()),
                ],
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tdw.progress.sample","arguments":{"symbol":"aapl"},"_meta":{"progressToken":"progress-json"}}}"#,
            ),
        );

        assert_eq!(response.status, 200);
        let body = response
            .body_text()
            .unwrap_or_else(|| panic!("JSON response body should be text"));
        let decoded: Value = serde_json::from_str(body)
            .unwrap_or_else(|error| panic!("HTTP response should be JSON: {error}; {body}"));
        let messages = decoded
            .as_array()
            .unwrap_or_else(|| panic!("progress response should preserve all JSON-RPC messages"));
        assert!(messages.len() >= 2);
        assert_eq!(messages[0]["method"], "notifications/progress");
        assert_eq!(
            messages
                .last()
                .unwrap_or_else(|| panic!("final response should be present"))["id"],
            4
        );
    }

    #[test]
    fn streamable_http_get_and_method_boundaries_are_explicit() {
        let mut server = McpServer::new();

        let sse_ready = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "GET",
                "/mcp",
                vec![("Accept".to_string(), "text/event-stream".to_string())],
                "",
            ),
        );
        assert_eq!(sse_ready.status, 200);
        assert_eq!(sse_ready.header("content-type"), Some("text/event-stream"));
        assert!(
            sse_ready
                .body_text()
                .is_some_and(|body| body.contains("stream ready"))
        );

        let no_sse = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new("GET", "/mcp", Vec::new(), ""),
        );
        assert_eq!(no_sse.status, 405);
        assert_eq!(no_sse.header("allow"), Some("GET, POST, OPTIONS"));

        let wrong_path = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new("POST", "/other", Vec::new(), ""),
        );
        assert_eq!(wrong_path.status, 404);

        let wrong_media = handle_streamable_http_request(
            &mut server,
            StreamableHttpRequest::new(
                "POST",
                "/mcp",
                vec![("Content-Type".to_string(), "text/plain".to_string())],
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            ),
        );
        assert_eq!(wrong_media.status, 415);
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
