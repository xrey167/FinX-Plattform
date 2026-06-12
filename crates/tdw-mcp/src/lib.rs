#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use tdw_agent::Registry;
use tdw_app_client::{
    DEFAULT_DAEMON_TCP_ADDR, DaemonClient, DaemonClientConfig, DaemonClientError, DaemonSubmission,
};
use tdw_app_server::{DaemonEndpoint, DaemonTransport};
use tdw_config::{ConfigLayer, ConfigLayerKind, TdwConfig, merge_layers};
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_protocol::{ActorKind, ActorRef, CostHint, EventMsg, Op, OpEnvelope, PlanId, SessionId};

pub(crate) mod knowledge_explain_tools;
pub(crate) mod knowledge_feedback_tools;
pub(crate) mod knowledge_ingest_tools;
pub(crate) mod knowledge_tools;
pub(crate) mod knowledge_write_tools;
pub mod ops;

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

/// Cheap, cloneable per-method request counters for the MCP `/metrics` surface.
///
/// Keyed by JSON-RPC method name (`initialize`, `tools/list`, `tools/call`, …)
/// so the exposition shows one labelled sample per method actually seen. Backed
/// by a `Mutex<BTreeMap>` (deterministic key order for stable output); the
/// hot-path cost is one short lock per request.
#[derive(Clone, Default)]
pub struct McpMetrics {
    by_method: Arc<Mutex<std::collections::BTreeMap<String, u64>>>,
}

impl McpMetrics {
    /// A fresh, empty metrics handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one request for `method`.
    pub fn record(&self, method: &str) {
        if let Ok(mut map) = self.by_method.lock() {
            *map.entry(method.to_string()).or_insert(0) += 1;
        }
    }

    /// Render request counts as a Prometheus `tdw_mcp_requests_total{method=...}`
    /// counter family.
    #[must_use]
    pub fn render(&self) -> String {
        use tdw_app_server::ops::{Metric, render_prometheus};
        let snapshot = self
            .by_method
            .lock()
            .map(|map| map.clone())
            .unwrap_or_default();
        let metrics: Vec<Metric> = snapshot
            .iter()
            .map(|(method, count)| {
                #[allow(clippy::cast_precision_loss)]
                Metric::counter(
                    "tdw_mcp_requests_total",
                    "MCP JSON-RPC requests by method",
                    *count as f64,
                )
                .with_label("method", method)
            })
            .collect();
        if metrics.is_empty() {
            // Emit the family header even with no samples so scrapers see the
            // series exists.
            return "# HELP tdw_mcp_requests_total MCP JSON-RPC requests by method\n\
                    # TYPE tdw_mcp_requests_total counter\n"
                .to_string();
        }
        render_prometheus(&metrics)
    }
}

pub struct McpServer {
    initialized: bool,
    client_info: Option<Value>,
    cancelled_requests: Vec<CancelledRequest>,
    daemon: DaemonToolRuntime,
    /// Per-method request counters surfaced by the ops `/metrics` endpoint.
    metrics: McpMetrics,
    /// Optional `tdw-agent` registry whose `tool` resources are appended to the hardcoded
    /// `tools/list` catalog. `None` keeps only the built-in tools.
    registry: Option<Registry>,
    /// Cached projection of the attached registry's `tool` resources, computed ONCE at
    /// attach time in [`McpServer::set_registry`]. Already deduped against built-in names
    /// (built-ins win) with `notExecutable` baked in, so the hot paths (`tools/list`,
    /// `tools/call`) consult this vec instead of re-projecting the whole registry per
    /// request. Empty when no registry is attached.
    registry_descriptors: Vec<ToolDescriptor>,
    /// Executes bound registry tools (resolves each tool's `implementation` binding). Used
    /// in `call_tool` for listed registry tools before the built-in `execute_tool` path.
    executor: tdw_tool_exec::ToolExecutor,
    /// Whether the `tdw.*.sample` evidence/demo tools appear in `tools/list`.
    /// Defaults from `TDW_MCP_SAMPLE_TOOLS=1` so a real agent's catalog leads
    /// with data tools; hidden tools remain callable via `tools/call` so the
    /// packaged smokes keep working.
    expose_sample_tools: bool,
    /// Optional knowledge runtime (hybrid retriever + graph/tag engines + version
    /// info). When attached, the `tdw.kg.*` / `tdw.tags.query` read tools
    /// (knowledge-system B8) are appended to `tools/list` and dispatched in
    /// `call_tool`; `None` keeps the knowledge surface off entirely.
    knowledge: Option<Arc<tdw_knowledge::runtime::KnowledgeRuntime>>,
    /// Optional retrieval feedback store (knowledge-system B10). When attached
    /// AND a knowledge runtime is present, the `tdw.kg.feedback` tool is exposed.
    /// `None` keeps the feedback surface off entirely.
    feedback_store: Option<Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>>,
    /// Optional daemon-hosted knowledge indexer (knowledge-system K-E3). When
    /// attached AND a knowledge runtime is present, the `tdw.kg.ingest` tool is
    /// exposed. The indexer is the write surface for the public ingestion path;
    /// its `Arc<tokio::sync::Mutex<KnowledgeIndexer>>` is shared with the
    /// daemon's `Backend` so the manifest and in-process state persist across
    /// MCP calls on the same process.
    indexer: Option<Arc<tokio::sync::Mutex<KnowledgeIndexer>>>,
}

impl Default for McpServer {
    fn default() -> Self {
        Self {
            initialized: false,
            client_info: None,
            cancelled_requests: Vec::new(),
            daemon: DaemonToolRuntime::from_env(),
            metrics: McpMetrics::new(),
            registry: None,
            registry_descriptors: Vec::new(),
            executor: tdw_tool_exec::ToolExecutor::new(),
            expose_sample_tools: sample_tools_enabled(),
            knowledge: None,
            feedback_store: None,
            indexer: None,
        }
    }
}

impl McpServer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_daemon_config(config: DaemonClientConfig) -> Self {
        Self {
            initialized: false,
            client_info: None,
            cancelled_requests: Vec::new(),
            daemon: DaemonToolRuntime::configured(config),
            metrics: McpMetrics::new(),
            registry: None,
            registry_descriptors: Vec::new(),
            executor: tdw_tool_exec::ToolExecutor::new(),
            expose_sample_tools: sample_tools_enabled(),
            knowledge: None,
            feedback_store: None,
            indexer: None,
        }
    }

    /// Override whether the `*.sample` demo tools are listed in `tools/list`
    /// (default: only when `TDW_MCP_SAMPLE_TOOLS=1`). Hidden tools remain
    /// callable via `tools/call`. Consumes and returns `self` for builder use.
    #[must_use]
    pub const fn with_sample_tools(mut self, expose: bool) -> Self {
        self.expose_sample_tools = expose;
        self
    }

    /// A cloneable handle to this server's per-method request metrics, for the
    /// ops `/metrics` endpoint.
    #[must_use]
    pub fn metrics(&self) -> McpMetrics {
        self.metrics.clone()
    }

    /// Attach a `tdw-agent` [`Registry`]; its `tool` resources are exposed via `tools/list`
    /// in addition to the built-in tools. Consumes and returns `self` for builder use.
    #[must_use]
    pub fn with_registry(mut self, registry: Registry) -> Self {
        self.set_registry(registry);
        self
    }

    /// Replace the registry-tool [`tdw_tool_exec::ToolExecutor`] (e.g. to supply an explicit
    /// [`tdw_tool_exec::CommandPolicy`] instead of the env-derived default). Consumes and
    /// returns `self` for builder use.
    #[must_use]
    pub fn with_executor(mut self, executor: tdw_tool_exec::ToolExecutor) -> Self {
        self.executor = executor;
        self
    }

    /// Attach a [`tdw_knowledge::runtime::KnowledgeRuntime`]; its `tdw.kg.*` and
    /// `tdw.tags.query` read tools (knowledge-system B8) are exposed via
    /// `tools/list` in addition to the built-in tools and dispatched in
    /// `tools/call`. `None` keeps the knowledge surface off. Consumes and
    /// returns `self` for builder use.
    #[must_use]
    pub fn with_knowledge(
        mut self,
        runtime: Arc<tdw_knowledge::runtime::KnowledgeRuntime>,
    ) -> Self {
        self.knowledge = Some(runtime);
        self
    }

    /// Attach a [`tdw_agent_store::RetrievalFeedbackStore`] (knowledge-system B10).
    /// When attached AND a knowledge runtime is present, `tdw.kg.feedback` appears in
    /// `tools/list` and is dispatched in `tools/call`. Consumes and returns `self` for
    /// builder use.
    #[must_use]
    pub fn with_feedback_store(
        mut self,
        store: Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>,
    ) -> Self {
        self.feedback_store = Some(store);
        self
    }

    /// Set (or replace) the attached feedback store in place — the same seam
    /// that [`AgentBackend::set_proposals`] uses for the write queue in B9.
    pub fn set_feedback_store(
        &mut self,
        store: Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>,
    ) {
        self.feedback_store = Some(store);
    }

    /// Attach a daemon-hosted [`KnowledgeIndexer`] (knowledge-system K-E3).
    ///
    /// When attached AND a knowledge runtime is present, the `tdw.kg.ingest`
    /// write tool is exposed via `tools/list` and dispatched in `tools/call`.
    /// `None` keeps the ingestion surface off entirely. Consumes and returns
    /// `self` for builder use.
    ///
    /// The indexer handle is an `Arc<tokio::sync::Mutex<KnowledgeIndexer>>`
    /// shared with the daemon's `Backend`, so manifest state and in-process
    /// tag assignments persist across MCP calls in the same process.
    #[must_use]
    pub fn with_indexer(mut self, indexer: Arc<tokio::sync::Mutex<KnowledgeIndexer>>) -> Self {
        self.indexer = Some(indexer);
        self
    }

    /// Set (or replace) the attached `tdw-agent` [`Registry`] whose `tool` resources are
    /// exposed via `tools/list`.
    ///
    /// This is the single place the registry-tool projection is computed: the registry is
    /// projected ONCE here (deduped against built-in names, built-ins win, with
    /// `notExecutable` baked in) and cached in `registry_descriptors`. The hot paths
    /// (`tools/list`, `tools/call`) then consult the cache instead of re-projecting per
    /// request. Calling this again refreshes the cache for the new registry.
    pub fn set_registry(&mut self, registry: Registry) {
        let builtin_names: std::collections::HashSet<String> = tool_descriptors()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        self.registry_descriptors = registry_tool_descriptors(&registry)
            .into_iter()
            .filter(|tool| !builtin_names.contains(&tool.name))
            .collect();
        self.registry = Some(registry);
    }

    /// Set (or replace) the daemon client configuration the daemon-backed tools
    /// (e.g. `tdw.daemon.query.submit`) use to reach the TDW daemon. Mirrors
    /// [`Self::set_registry`] as an in-place setter, so an already-composed server
    /// (registry + executor attached) can be pointed at a late-bound daemon
    /// address without rebuilding it.
    pub fn set_daemon_config(&mut self, config: DaemonClientConfig) {
        self.daemon = DaemonToolRuntime::configured(config);
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

    /// The full `tools/list` catalog: the built-in [`tool_descriptors`] plus, when a
    /// registry is attached, the descriptors projected from its `tool` resources.
    ///
    /// Built-ins always win on name collisions: a registry tool whose `name` equals a
    /// built-in name is skipped so the catalog never emits duplicate descriptors and
    /// `tools/call` keeps dispatching to the built-in.
    fn all_tool_descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors = tool_descriptors();
        if !self.expose_sample_tools {
            // The `*.sample` evidence tools are demo surface: hidden from agents
            // by default (still callable) so the catalog leads with the real
            // data and daemon tools.
            descriptors.retain(|tool| !tool.name.ends_with(".sample"));
        }
        // The knowledge read tools (knowledge-system B8) are appended ONLY when a
        // runtime is attached — exactly like the registry descriptors, conditional on
        // the optional handle.
        if self.knowledge.is_some() {
            descriptors.extend(knowledge_tools::descriptors());
        }
        // The knowledge WRITE tools (knowledge-system B9) are appended ONLY when the
        // runtime ALSO has the proposal queue + adaptivity resolver attached — the
        // gate's inputs. A read-only knowledge runtime never exposes the write surface.
        if self.knowledge_writes_available() {
            descriptors.extend(knowledge_write_tools::descriptors());
        }
        // The knowledge FEEDBACK tool (knowledge-system B10) is appended ONLY when the
        // runtime is present AND a feedback store handle is attached. Absent either → off.
        if self.knowledge_feedback_available() {
            descriptors.push(knowledge_feedback_tools::descriptor());
        }
        // The knowledge EXPLAIN tools (knowledge-system K-X1): tdw.kg.why and
        // tdw.kg.diff are read-only and require only the graph engine. They are
        // appended whenever a knowledge runtime with a graph engine is attached —
        // the same gate as the B8 entity/traverse/path tools.
        if self
            .knowledge
            .as_ref()
            .is_some_and(|rt| rt.graph().is_some())
        {
            descriptors.extend(knowledge_explain_tools::descriptors());
        }
        // The knowledge INGEST tool (knowledge-system K-E3) is appended ONLY when a
        // knowledge runtime is present AND a hosted indexer handle is attached. The
        // indexer is the public write surface: content-hash-idempotent batch ingest
        // through the full B5 pipeline (rules, lexical, durable graph).
        if self.knowledge_ingest_available() {
            descriptors.push(knowledge_ingest_tools::descriptor());
        }
        // `registry_descriptors` is already deduped against built-in names at attach time
        // (`set_registry`), so a plain concatenation preserves the built-in-wins ordering
        // and never emits duplicate descriptors. Empty when no registry is attached.
        descriptors.extend(self.registry_descriptors.iter().cloned());
        descriptors
    }

    pub fn handle_json_rpc_line(&mut self, line: &str) -> Vec<String> {
        let messages = match parse_inbound(line) {
            Ok(inbound) if inbound.is_notification => {
                self.metrics.record(&inbound.method);
                self.handle_notification(&inbound)
            }
            Ok(inbound) => {
                self.metrics.record(&inbound.method);
                self.handle_request(&inbound)
            }
            Err(problem) => vec![error_message(problem)],
        };

        messages.iter().map(encode_message).collect()
    }

    fn handle_notification(&mut self, inbound: &JsonRpcInbound) -> Vec<Value> {
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

    fn handle_request(&mut self, inbound: &JsonRpcInbound) -> Vec<Value> {
        let id = inbound.id.clone().unwrap_or(Value::Null);
        if !self.initialized && !matches!(inbound.method.as_str(), "initialize" | "ping") {
            return vec![error_message(JsonRpcProblem::new(
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
                &json!({ "tools": self.all_tool_descriptors() }),
            )],
            "tools/call" => self.call_tool(&id, &inbound.params),
            "resources/list" => vec![success_message(
                &id,
                &json!({ "resources": resource_descriptors() }),
            )],
            "resources/read" => vec![Self::read_resource(&id, &inbound.params)],
            "prompts/list" => {
                vec![success_message(
                    &id,
                    &json!({ "prompts": prompt_descriptors() }),
                )]
            }
            "prompts/get" => vec![Self::get_prompt(&id, &inbound.params)],
            _ => vec![error_message(JsonRpcProblem::new(
                id,
                -32601,
                "method not found",
            ))],
        }
    }

    fn initialize(&mut self, id: &Value, params: &Value) -> Value {
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
            &json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": server_capabilities(),
                "serverInfo": {
                    "name": SERVER_NAME,
                    "title": SERVER_TITLE,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": format!(
                    "TDW exposes deterministic offline tools plus explicitly daemon-backed query and triage tools over MCP stdio or Streamable HTTP. {data_mode} Requested protocol: {requested}.",
                    data_mode = DATA_MODE_DISCLOSURE,
                ),
            }),
        )
    }

    fn call_tool(&self, id: &Value, params: &Value) -> Vec<Value> {
        let Some(params_object) = params.as_object() else {
            return vec![error_message(JsonRpcProblem::new(
                id.clone(),
                -32602,
                "tools/call params must be an object",
            ))];
        };
        let Some(name) = string_field(params_object, "name") else {
            return vec![error_message(JsonRpcProblem::new(
                id.clone(),
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
                id.clone(),
                -32602,
                "tools/call arguments must be an object",
            ))];
        }

        let progress_token = progress_token(params);

        if let Some(messages) = self.dispatch_knowledge_tool(id, name, &arguments) {
            return messages;
        }

        if let Some(messages) = self.dispatch_knowledge_write_tool(id, name, &arguments) {
            return messages;
        }

        if let Some(messages) = self.dispatch_knowledge_feedback_tool(id, name, &arguments) {
            return messages;
        }

        if let Some(messages) = self.dispatch_knowledge_explain_tool(id, name, &arguments) {
            return messages;
        }

        if let Some(messages) = self.dispatch_knowledge_ingest_tool(id, name, &arguments) {
            return messages;
        }

        if let Some(messages) = self.dispatch_registry_tool(id, name, &arguments) {
            return messages;
        }

        let result = execute_tool(&self.daemon, name, &arguments);
        match result {
            Ok(ToolExecution {
                structured,
                progress_events,
            }) => {
                let mut messages = progress_notifications(progress_token, &progress_events);
                messages.push(success_message(id, &tool_result(&structured)));
                messages
            }
            Err(ToolFailure::Protocol(problem)) => {
                // A name that is unknown to `execute_tool` but IS a listed registry tool
                // (and not a built-in) is stubbed-but-listed, not truly unknown. Distinguish
                // it with -32601 (method not found) instead of the generic -32602.
                //
                // EXECUTION GAP (intentionally deferred): registry tools are *listed*
                // truthfully via `tools/list` but are NOT executable here. Running one
                // requires an execution backend keyed off the tool's `implementation`/origin
                // — e.g. a proxy to a sub-MCP server, a bound Rust fn, or an HTTP endpoint —
                // and no such backend exists in this crate. Until one is wired up we return a
                // precise, non-misleading not-implemented error rather than pretending to
                // dispatch. This keeps the surface a "functional packet": tools are listed
                // honestly and calls fail honestly. See `registry_tool_descriptors`.
                if problem.code == -32602 && self.is_listed_registry_tool(name) {
                    return vec![error_message(
                        JsonRpcProblem::new(
                            id.clone(),
                            -32601,
                            format!("registry tool not yet executable: {name}"),
                        )
                        .with_data(json!({ "tool": name })),
                    )];
                }
                vec![error_message(
                    problem
                        .with_id(id.clone())
                        .with_data(json!({ "tool": name })),
                )]
            }
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, &tool_error_result(&message))]
            }
        }
    }

    /// Dispatch a knowledge read tool (`tdw.kg.*` / `tdw.tags.query`, knowledge-system B8).
    ///
    /// Returns `Some(messages)` when `name` is a knowledge tool — the resolved response
    /// whether the runtime is attached (route to [`knowledge_tools::execute`]) or not
    /// (a tool error, never a protocol error, per the plan). Returns `None` when `name`
    /// is not a knowledge tool so the caller falls through to the registry and built-in
    /// dispatch paths.
    fn dispatch_knowledge_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if !knowledge_tools::owns(name) {
            return None;
        }
        let Some(runtime) = self.knowledge.as_ref() else {
            // Attached = false + a knowledge name is a tool error, not a protocol error.
            return Some(vec![success_message(
                id,
                &tool_error_result("knowledge runtime not attached"),
            )]);
        };
        // `call_tool` already validated `arguments` is an object.
        let arguments_object = arguments.as_object().cloned().unwrap_or_default();
        let messages = match knowledge_tools::execute(runtime, name, &arguments_object) {
            Ok(ToolExecution { structured, .. }) => {
                vec![success_message(id, &tool_result(&structured))]
            }
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, &tool_error_result(&message))]
            }
            Err(ToolFailure::Protocol(problem)) => vec![error_message(
                problem
                    .with_id(id.clone())
                    .with_data(json!({ "tool": name })),
            )],
        };
        Some(messages)
    }

    /// True when the attached knowledge runtime exposes the gated WRITE surface
    /// (knowledge-system B9): it has a proposal queue, an adaptivity resolver,
    /// AND a bound agent identity. A read-only knowledge runtime returns `false`.
    fn knowledge_writes_available(&self) -> bool {
        self.knowledge.as_ref().is_some_and(|runtime| {
            runtime.proposals().is_some()
                && runtime.adaptivity_resolver().is_some()
                && runtime.bound_agent_id().is_some()
        })
    }

    /// Dispatch a knowledge write tool (`tdw.tags.define` / `tdw.tags.assign` /
    /// `tdw.kg.annotate` / `tdw.kg.proposals`, knowledge-system B9).
    ///
    /// Returns `Some(messages)` when `name` is a write tool — a tool error (never
    /// a protocol error) when the write surface is unavailable (no runtime, or no
    /// proposal queue + resolver), otherwise the [`knowledge_write_tools::execute`]
    /// result. Returns `None` when `name` is not a write tool so the caller falls
    /// through to the read/registry/built-in dispatch paths.
    fn dispatch_knowledge_write_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if !knowledge_write_tools::owns(name) {
            return None;
        }
        if !self.knowledge_writes_available() {
            // No runtime, or a read-only runtime: a write name is a tool error.
            return Some(vec![success_message(
                id,
                &tool_error_result("knowledge write surface not attached"),
            )]);
        }
        let runtime = self.knowledge.as_ref()?;
        let arguments_object = arguments.as_object().cloned().unwrap_or_default();
        let messages = match knowledge_write_tools::execute(runtime, name, &arguments_object) {
            Ok(ToolExecution { structured, .. }) => {
                vec![success_message(id, &tool_result(&structured))]
            }
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, &tool_error_result(&message))]
            }
            Err(ToolFailure::Protocol(problem)) => vec![error_message(
                problem
                    .with_id(id.clone())
                    .with_data(json!({ "tool": name })),
            )],
        };
        Some(messages)
    }

    /// True when the feedback surface is available (knowledge-system B10): a
    /// knowledge runtime is attached AND a feedback store handle is attached.
    const fn knowledge_feedback_available(&self) -> bool {
        self.knowledge.is_some() && self.feedback_store.is_some()
    }

    /// True when the ingest surface is available (knowledge-system K-E3): a
    /// knowledge runtime is attached AND a hosted indexer handle is attached.
    const fn knowledge_ingest_available(&self) -> bool {
        self.knowledge.is_some() && self.indexer.is_some()
    }

    /// Dispatch the knowledge ingest tool (`tdw.kg.ingest`, knowledge-system K-E3).
    ///
    /// Returns `Some(messages)` when `name` is `tdw.kg.ingest` — a tool error
    /// (never a protocol error) when the ingest surface is unavailable (no
    /// runtime or no indexer), otherwise the
    /// [`knowledge_ingest_tools::execute`] result. Returns `None` when `name`
    /// is not the ingest tool.
    fn dispatch_knowledge_ingest_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if !knowledge_ingest_tools::owns(name) {
            return None;
        }
        let Some(indexer) = self.indexer.as_ref() else {
            return Some(vec![success_message(
                id,
                &tool_error_result("knowledge ingest surface not attached"),
            )]);
        };
        let arguments_object = arguments.as_object().cloned().unwrap_or_default();
        let messages = match knowledge_ingest_tools::execute(indexer, &arguments_object) {
            Ok(ToolExecution { structured, .. }) => {
                vec![success_message(id, &tool_result(&structured))]
            }
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, &tool_error_result(&message))]
            }
            Err(ToolFailure::Protocol(problem)) => vec![error_message(
                problem
                    .with_id(id.clone())
                    .with_data(json!({ "tool": name })),
            )],
        };
        Some(messages)
    }

    /// Dispatch the knowledge feedback tool (`tdw.kg.feedback`, knowledge-system B10).
    ///
    /// Returns `Some(messages)` when `name` is `tdw.kg.feedback` — a tool error
    /// (never a protocol error) when the feedback surface is unavailable (no runtime
    /// or no store), otherwise the [`knowledge_feedback_tools::execute`] result.
    /// Returns `None` when `name` is not the feedback tool.
    fn dispatch_knowledge_feedback_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if !knowledge_feedback_tools::owns(name) {
            return None;
        }
        let (Some(runtime), Some(store)) = (self.knowledge.as_ref(), self.feedback_store.as_ref())
        else {
            return Some(vec![success_message(
                id,
                &tool_error_result("knowledge feedback surface not attached"),
            )]);
        };
        let arguments_object = arguments.as_object().cloned().unwrap_or_default();
        let now = chrono::Utc::now().to_rfc3339();
        let messages =
            match knowledge_feedback_tools::execute(runtime, store, &arguments_object, &now) {
                Ok(crate::ToolExecution { structured, .. }) => {
                    vec![success_message(id, &tool_result(&structured))]
                }
                Err(ToolFailure::Execution(message)) => {
                    vec![success_message(id, &tool_error_result(&message))]
                }
                Err(ToolFailure::Protocol(problem)) => vec![error_message(
                    problem
                        .with_id(id.clone())
                        .with_data(json!({ "tool": name })),
                )],
            };
        Some(messages)
    }

    /// Dispatch a knowledge explain tool (`tdw.kg.why` / `tdw.kg.diff`,
    /// knowledge-system K-X1).
    ///
    /// Returns `Some(messages)` when `name` is an explain tool — a tool error
    /// (never a protocol error) when the graph engine is unavailable, otherwise
    /// the [`knowledge_explain_tools::execute`] result. Returns `None` when
    /// `name` is not an explain tool so the caller falls through to the registry
    /// and built-in dispatch paths.
    fn dispatch_knowledge_explain_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if !knowledge_explain_tools::owns(name) {
            return None;
        }
        let Some(runtime) = self.knowledge.as_ref() else {
            return Some(vec![success_message(
                id,
                &tool_error_result("knowledge runtime not attached"),
            )]);
        };
        let arguments_object = arguments.as_object().cloned().unwrap_or_default();
        let messages = match knowledge_explain_tools::execute(runtime, name, &arguments_object) {
            Ok(ToolExecution { structured, .. }) => {
                vec![success_message(id, &tool_result(&structured))]
            }
            Err(ToolFailure::Execution(message)) => {
                vec![success_message(id, &tool_error_result(&message))]
            }
            Err(ToolFailure::Protocol(problem)) => vec![error_message(
                problem
                    .with_id(id.clone())
                    .with_data(json!({ "tool": name })),
            )],
        };
        Some(messages)
    }

    /// Dispatch a listed registry tool (not a built-in) through the tool-execution backend.
    ///
    /// Returns `Some(messages)` when `name` is a listed registry tool (the resolved response,
    /// whether success or error); returns `None` when it is not, so the caller falls through
    /// to the built-in `execute_tool` path. The tool-execution backend resolves each tool's
    /// `implementation` binding. `Unbound` tools fall through to the existing `-32601` "not
    /// yet executable" path below; an execution failure (the tool ran but errored) is
    /// surfaced as an `isError` tool result.
    fn dispatch_registry_tool(
        &self,
        id: &Value,
        name: &str,
        arguments: &Value,
    ) -> Option<Vec<Value>> {
        if let Some(registry) = self.registry.as_ref()
            && self.is_listed_registry_tool(name)
        {
            match self.executor.execute(registry, name, arguments) {
                Ok(outcome) => {
                    return Some(vec![success_message(id, &tool_result(&outcome.structured))]);
                }
                Err(tdw_tool_exec::ExecError::Unbound) => {
                    return Some(vec![error_message(
                        JsonRpcProblem::new(
                            id.clone(),
                            -32601,
                            format!("registry tool not yet executable: {name}"),
                        )
                        .with_data(json!({ "tool": name })),
                    )]);
                }
                Err(other) => {
                    // Do not leak the raw executor error to the client: map it to a generic
                    // category message and log the detail server-side (decision 3).
                    eprintln!("tdw-mcp: registry tool {name} error: {other}");
                    let category = match other {
                        tdw_tool_exec::ExecError::NotPermitted(_)
                        | tdw_tool_exec::ExecError::Blocked { .. } => {
                            "registry tool execution not permitted"
                        }
                        tdw_tool_exec::ExecError::BadArguments(_) => {
                            "invalid registry tool definition"
                        }
                        tdw_tool_exec::ExecError::InvalidArguments { .. } => {
                            "invalid registry tool arguments"
                        }
                        tdw_tool_exec::ExecError::ToolNotFound(_)
                        | tdw_tool_exec::ExecError::HandlerNotFound(_) => {
                            "registry tool not available"
                        }
                        tdw_tool_exec::ExecError::NotYetSupported(_) => {
                            "registry tool not yet executable"
                        }
                        tdw_tool_exec::ExecError::Backend(_)
                        | tdw_tool_exec::ExecError::Unbound => "registry tool execution failed",
                    };
                    return Some(vec![success_message(id, &tool_error_result(category))]);
                }
            }
        }
        None
    }

    /// True when `name` is exposed by the attached registry's `tool` resources and is NOT a
    /// built-in tool. Built-ins are checked against [`tool_descriptors`] so a registry tool
    /// that collides with a built-in (and is therefore deduped out of `tools/list`) is not
    /// treated as a listed registry tool.
    fn is_listed_registry_tool(&self, name: &str) -> bool {
        // `registry_descriptors` already excludes any registry tool whose name collides with
        // a built-in (deduped at attach time, built-ins win), so membership here is exactly
        // "exposed by the registry AND not a built-in" — the original semantics, without
        // re-projecting the registry per call. Empty (returns false) when no registry is
        // attached.
        self.registry_descriptors
            .iter()
            .any(|tool| tool.name == name)
    }

    fn read_resource(id: &Value, params: &Value) -> Value {
        let Some(params_object) = params.as_object() else {
            return error_message(JsonRpcProblem::new(
                id.clone(),
                -32602,
                "resources/read params must be an object",
            ));
        };
        let Some(uri) = string_field(params_object, "uri") else {
            return error_message(JsonRpcProblem::new(
                id.clone(),
                -32602,
                "resources/read requires string field: uri",
            ));
        };

        match resource_content(uri) {
            Ok(content) => success_message(id, &json!({ "contents": [content] })),
            Err(problem) => {
                error_message(problem.with_id(id.clone()).with_data(json!({ "uri": uri })))
            }
        }
    }

    fn get_prompt(id: &Value, params: &Value) -> Value {
        let Some(params_object) = params.as_object() else {
            return error_message(JsonRpcProblem::new(
                id.clone(),
                -32602,
                "prompts/get params must be an object",
            ));
        };
        let Some(name) = string_field(params_object, "name") else {
            return error_message(JsonRpcProblem::new(
                id.clone(),
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
                id.clone(),
                -32602,
                "prompts/get arguments must be an object",
            ));
        }

        match prompt_content(name, &arguments) {
            Ok(content) => success_message(id, &content),
            Err(problem) => error_message(
                problem
                    .with_id(id.clone())
                    .with_data(json!({ "prompt": name })),
            ),
        }
    }
}

#[must_use]
pub fn run_stdio_json_rpc() -> i32 {
    run_stdio_json_rpc_with_daemon(None)
}

/// Run the blocking stdio JSON-RPC loop, optionally pointing the embedded
/// daemon tools at an explicit [`DaemonClientConfig`] instead of the
/// environment-derived default.
///
/// When `daemon` is `Some`, the server is built via
/// [`McpServer::with_daemon_config`] so its daemon-backed tools submit to that
/// endpoint — used by the unified `tdw-backend` binary to point the in-process
/// MCP surface at the co-resident daemon's loopback address without mutating
/// the process environment. When `None`, behavior is byte-for-byte identical to
/// [`run_stdio_json_rpc`] (env-derived daemon config). In both cases the
/// `TDW_AGENT_REGISTRY_DIR` registry (if any) is attached.
#[must_use]
pub fn run_stdio_json_rpc_with_daemon(daemon: Option<DaemonClientConfig>) -> i32 {
    let stdin = std::io::stdin();
    let base = daemon.map_or_else(McpServer::new, McpServer::with_daemon_config);
    let mut server = match attach_env_registry(base) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("tdw-mcp registry configuration error: {error}");
            return 2;
        }
    };
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

/// Run the blocking stdio JSON-RPC loop with an explicit daemon config **and**
/// pre-built knowledge handles (knowledge-system F1/K-E3).
///
/// Used by the unified `tdw-backend` binary when running in `Both` (or
/// `McpOnly`) mode: the in-process `Backend` has already constructed a
/// [`KnowledgeRuntime`](tdw_knowledge::runtime::KnowledgeRuntime), a
/// [`RetrievalFeedbackStore`](tdw_agent_store::RetrievalFeedbackStore), and a
/// [`KnowledgeIndexer`](tdw_knowledge::indexer::KnowledgeIndexer); passing
/// them here injects them into the embedded MCP server so the knowledge read
/// tools, the `tdw.kg.feedback` write tool, and the `tdw.kg.ingest` tool are
/// live on the stdio surface.
///
/// When any of `knowledge`, `feedback`, or `indexer` are `None` the server
/// behaves identically to [`run_stdio_json_rpc_with_daemon`] for those surfaces.
#[must_use]
pub fn run_stdio_json_rpc_with_knowledge(
    daemon: Option<DaemonClientConfig>,
    knowledge: Option<Arc<tdw_knowledge::runtime::KnowledgeRuntime>>,
    feedback: Option<Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>>,
    indexer: Option<Arc<tokio::sync::Mutex<KnowledgeIndexer>>>,
) -> i32 {
    let stdin = std::io::stdin();
    let base = daemon.map_or_else(McpServer::new, McpServer::with_daemon_config);
    let base = if let Some(rt) = knowledge {
        base.with_knowledge(rt)
    } else {
        base
    };
    let base = if let Some(store) = feedback {
        base.with_feedback_store(store)
    } else {
        base
    };
    let base = if let Some(idx) = indexer {
        base.with_indexer(idx)
    } else {
        base
    };
    let mut server = match attach_env_registry(base) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("tdw-mcp registry configuration error: {error}");
            return 2;
        }
    };
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
        &request,
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

/// Environment variable naming the directory of `tdw-agent` registry definitions to load and
/// attach to every server entrypoint. Unset → no registry is attached (built-in tools only).
pub const REGISTRY_DIR_ENV: &str = "TDW_AGENT_REGISTRY_DIR";

/// Failure loading a `tdw-agent` registry from a configured directory.
///
/// Wraps [`tdw_agent::LoadError`] with the offending directory so a misconfigured
/// [`REGISTRY_DIR_ENV`] surfaces a precise, actionable message instead of being silently
/// ignored.
#[derive(Debug)]
pub struct RegistryConfigError {
    dir: String,
    source: tdw_agent::LoadError,
}

impl std::fmt::Display for RegistryConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "failed to load tdw-agent registry from {}: {}",
            self.dir, self.source
        )
    }
}

impl std::error::Error for RegistryConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Load a `tdw-agent` [`Registry`] from `dir` (a directory of `*.json5` definitions).
///
/// Thin wrapper over [`Registry::load_dir`] that attaches `dir` to any failure for a clear
/// diagnostic.
///
/// # Errors
///
/// Returns [`RegistryConfigError`] if the directory cannot be read or any definition is
/// invalid (see [`tdw_agent::LoadError`]).
pub fn registry_from_dir(dir: &Path) -> Result<Registry, RegistryConfigError> {
    Registry::load_dir(dir).map_err(|source| RegistryConfigError {
        dir: dir.display().to_string(),
        source,
    })
}

/// Resolve an optional registry from [`REGISTRY_DIR_ENV`].
///
/// - Unset (or empty) → `Ok(None)`: behavior is byte-for-byte unchanged (built-in tools only).
/// - Set → load that directory and return `Ok(Some(registry))`.
/// - Set but loading fails → `Err(..)`: a misconfiguration surfaces rather than being silently
///   ignored.
///
/// # Errors
///
/// Returns [`RegistryConfigError`] when the variable is set but the directory cannot be loaded.
pub fn registry_from_env() -> Result<Option<Registry>, RegistryConfigError> {
    non_empty_env(REGISTRY_DIR_ENV)
        .map_or(Ok(None), |dir| registry_from_dir(Path::new(&dir)).map(Some))
}

/// Attach the [`registry_from_env`] registry to `server` when [`REGISTRY_DIR_ENV`] is set.
///
/// On success returns `server` unchanged when the variable is unset, or with the loaded
/// registry attached when it is set. Used by every server-construction entrypoint so the
/// registry→MCP `tools/list` surface is reachable from a running server.
///
/// **Scope cut — feedback store (F1):** this function does **not** attach a
/// [`RetrievalFeedbackStore`](tdw_agent_store::RetrievalFeedbackStore). The
/// standalone `tdw-mcp` entrypoints (stdio, Streamable HTTP) have no
/// co-resident [`tdw_backend::data::Backend`] to bridge to; wiring those live
/// paths to a daemon's consolidation loop is deferred to F1, the same
/// deferral B8/B9 made for [`KnowledgeRuntime`](tdw_knowledge::runtime::KnowledgeRuntime)
/// daemon hosting. In-process embedding hosts that construct both facades
/// inject the shared handle via [`McpServer::with_feedback_store`] directly —
/// see [`tdw_agent_store::feedback`] module doc for the host-wiring protocol.
///
/// # Errors
///
/// Returns [`RegistryConfigError`] when the variable is set but the directory cannot be loaded.
fn attach_env_registry(mut server: McpServer) -> Result<McpServer, RegistryConfigError> {
    if let Some(registry) = registry_from_env()? {
        server.set_registry(registry);
    }
    Ok(server)
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

    const fn configured(config: DaemonClientConfig) -> Self {
        Self { config: Ok(config) }
    }

    fn submit(&self, envelope: &OpEnvelope) -> Result<DaemonSubmission, String> {
        let config = self.config.as_ref().map_err(Clone::clone)?;
        DaemonClient::new(config.clone())
            .submit_and_wait(envelope)
            .map_err(|error| daemon_client_error_message(config, &error))
    }
}

fn daemon_client_error_message(config: &DaemonClientConfig, error: &DaemonClientError) -> String {
    format!(
        "{error}; endpoint={}://{}",
        daemon_transport_label(config.endpoint().transport),
        config.endpoint().address
    )
}

fn daemon_client_config_from_env() -> Result<DaemonClientConfig, String> {
    let config = tdw_config_from_env()?;
    daemon_client_config_from_sources(
        &config,
        non_empty_env("TDW_MCP_DAEMON_TRANSPORT"),
        non_empty_env("TDW_MCP_DAEMON_ADDR").or_else(|| non_empty_env("TDW_DAEMON_TCP_BIND")),
        non_empty_env("TDW_MCP_DAEMON_TIMEOUT_MS").as_deref(),
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
    config: &TdwConfig,
    transport_override: Option<String>,
    address_override: Option<String>,
    timeout_ms: Option<&str>,
) -> Result<DaemonClientConfig, String> {
    let transport = match transport_override {
        Some(value) => parse_daemon_transport(&value)?,
        None => config.daemon.transport,
    };
    let address =
        address_override.unwrap_or_else(|| daemon_endpoint_address_from_config(config, transport));
    let timeout = timeout_ms
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

    #[must_use]
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
    run_streamable_http_with_daemon(bind, None)
}

/// Run the blocking Streamable HTTP loop, optionally pointing the embedded
/// daemon tools at an explicit [`DaemonClientConfig`] instead of the
/// environment-derived default.
///
/// See [`run_stdio_json_rpc_with_daemon`] for the rationale; this is the HTTP
/// counterpart used by the unified `tdw-backend` binary. When `daemon` is
/// `None`, behavior is identical to [`run_streamable_http`].
#[must_use]
pub fn run_streamable_http_with_daemon(bind: &str, daemon: Option<DaemonClientConfig>) -> i32 {
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
    // Non-blocking accept so the loop can observe the graceful-shutdown flag
    // (set by SIGTERM/Ctrl-C) between connections and drain.
    if let Err(error) = listener.set_nonblocking(true) {
        eprintln!("tdw-mcp Streamable HTTP set_nonblocking failed: {error}");
        return 1;
    }
    eprintln!("tdw-mcp Streamable HTTP listening on http://{bind}{STREAMABLE_HTTP_PATH}");

    // Resolve the daemon's TCP address (if daemon-routed over TCP) for the ops
    // `/ready` reachability probe, before `daemon` is moved into the server.
    let daemon_tcp_addr = daemon_tcp_addr_for_readiness(daemon.as_ref());

    let base = daemon.map_or_else(McpServer::new, McpServer::with_daemon_config);
    let server = match attach_env_registry(base) {
        Ok(server) => Arc::new(Mutex::new(server)),
        Err(error) => {
            eprintln!("tdw-mcp registry configuration error: {error}");
            return 2;
        }
    };

    // Graceful shutdown: SIGTERM (container/systemd stop) or Ctrl-C trips the
    // flag; the accept loop and the ops listener both observe it and stop.
    let shutdown = ops::Shutdown::new();
    ops::install_signal_handler(shutdown.clone());

    // Optional ops surface (/health, /ready, /metrics), env-gated and off by
    // default; bound on TDW_MCP_OPS_BIND on its own thread.
    let ops_thread = spawn_mcp_ops(&server, daemon_tcp_addr, shutdown.clone());

    let config = Arc::new(streamable_http_config_from_env());
    let exit_code = loop {
        if shutdown.is_triggered() {
            break 0;
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
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
                    break 1;
                }
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                eprintln!("tdw-mcp Streamable HTTP accept failed: {error}");
                break 1;
            }
        }
    };

    shutdown.trigger();
    if let Some(thread) = ops_thread {
        let _ = thread.join();
    }
    exit_code
}

/// Run the blocking Streamable HTTP loop with an explicit daemon config **and**
/// pre-built knowledge handles (knowledge-system F1/K-E3).
///
/// Used by the unified `tdw-backend` binary when running in `Both`/`McpOnly`
/// mode with an HTTP MCP transport: injects a co-resident
/// [`KnowledgeRuntime`](tdw_knowledge::runtime::KnowledgeRuntime),
/// [`RetrievalFeedbackStore`](tdw_agent_store::RetrievalFeedbackStore), and
/// [`KnowledgeIndexer`](tdw_knowledge::indexer::KnowledgeIndexer) into the
/// embedded MCP server so the knowledge read tools, `tdw.kg.feedback`, and
/// `tdw.kg.ingest` are live on the Streamable HTTP surface without another
/// loopback hop.
///
/// When any of `knowledge`, `feedback`, or `indexer` are `None` the server
/// behaves identically to [`run_streamable_http_with_daemon`] for those surfaces.
#[must_use]
pub fn run_streamable_http_with_knowledge(
    bind: &str,
    daemon: Option<DaemonClientConfig>,
    knowledge: Option<Arc<tdw_knowledge::runtime::KnowledgeRuntime>>,
    feedback: Option<Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>>,
    indexer: Option<Arc<tokio::sync::Mutex<KnowledgeIndexer>>>,
) -> i32 {
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
    if let Err(error) = listener.set_nonblocking(true) {
        eprintln!("tdw-mcp Streamable HTTP set_nonblocking failed: {error}");
        return 1;
    }
    eprintln!("tdw-mcp Streamable HTTP listening on http://{bind}{STREAMABLE_HTTP_PATH}");

    let daemon_tcp_addr = daemon_tcp_addr_for_readiness(daemon.as_ref());

    let base = daemon.map_or_else(McpServer::new, McpServer::with_daemon_config);
    let base = if let Some(rt) = knowledge {
        base.with_knowledge(rt)
    } else {
        base
    };
    let base = if let Some(store) = feedback {
        base.with_feedback_store(store)
    } else {
        base
    };
    let base = if let Some(idx) = indexer {
        base.with_indexer(idx)
    } else {
        base
    };
    let server = match attach_env_registry(base) {
        Ok(server) => Arc::new(Mutex::new(server)),
        Err(error) => {
            eprintln!("tdw-mcp registry configuration error: {error}");
            return 2;
        }
    };

    let shutdown = ops::Shutdown::new();
    ops::install_signal_handler(shutdown.clone());

    let ops_thread = spawn_mcp_ops(&server, daemon_tcp_addr, shutdown.clone());

    let config = Arc::new(streamable_http_config_from_env());
    let exit_code = loop {
        if shutdown.is_triggered() {
            break 0;
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
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
                    break 1;
                }
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                eprintln!("tdw-mcp Streamable HTTP accept failed: {error}");
                break 1;
            }
        }
    };

    shutdown.trigger();
    if let Some(thread) = ops_thread {
        let _ = thread.join();
    }
    exit_code
}

/// Resolve the daemon's `host:port` for the ops readiness probe: `Some` only
/// when the MCP is daemon-routed over TCP (the reachability check connects to
/// it). `None` for UDS/HTTP-SSE endpoints or when no daemon is configured.
fn daemon_tcp_addr_for_readiness(daemon: Option<&DaemonClientConfig>) -> Option<String> {
    let endpoint = daemon?.endpoint();
    match endpoint.transport {
        DaemonTransport::Tcp => Some(endpoint.address.clone()),
        DaemonTransport::Uds | DaemonTransport::HttpSse => None,
    }
}

/// Spawn the MCP ops listener thread when `TDW_MCP_OPS_BIND` is set. Returns the
/// thread handle, or `None` when the env var is unset (the default).
fn spawn_mcp_ops(
    server: &Arc<Mutex<McpServer>>,
    daemon_tcp_addr: Option<String>,
    shutdown: ops::Shutdown,
) -> Option<std::thread::JoinHandle<()>> {
    let bind = non_empty_env("TDW_MCP_OPS_BIND")?;
    let metrics = server.lock().ok()?.metrics();
    let readiness = ops::McpReadiness::new(daemon_tcp_addr);
    std::thread::Builder::new()
        .name("tdw-mcp-ops".to_string())
        .spawn(move || {
            if let Err(error) = ops::serve_ops_blocking(&bind, &metrics, &readiness, &shutdown) {
                eprintln!("tdw-mcp ops listener error: {error}");
            }
        })
        .ok()
}

#[must_use]
pub fn run_streamable_http_smoke() -> i32 {
    let mut server = match attach_env_registry(McpServer::new()) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("tdw-mcp registry configuration error: {error}");
            return 2;
        }
    };
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
    let initialized = handle_streamable_http_request(&mut server, &initialize);
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
    let response = handle_streamable_http_request(&mut server, &tool_call);
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
    request: &StreamableHttpRequest,
) -> StreamableHttpResponse {
    handle_streamable_http_request_with_config(server, request, &StreamableHttpConfig::new())
}

pub fn handle_streamable_http_request_with_config(
    server: &mut McpServer,
    request: &StreamableHttpRequest,
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

    if !request_is_authorized(request, config) {
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
            attach_cors_origin(&mut response, request);
            response
        }
        "GET" if accepts_sse(request) => {
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
            attach_cors_origin(&mut response, request);
            response
        }
        "POST" => handle_streamable_http_post(server, request),
        _ => method_not_allowed(),
    }
}

fn handle_streamable_http_post(
    server: &mut McpServer,
    request: &StreamableHttpRequest,
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
        attach_cors_origin(&mut response, request);
        return response;
    }

    let mut response = if accepts_sse(request) {
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
    attach_cors_origin(&mut response, request);
    response
}

fn handle_streamable_http_connection(
    mut stream: TcpStream,
    server: &Arc<Mutex<McpServer>>,
    config: &StreamableHttpConfig,
) -> std::io::Result<()> {
    let response = read_streamable_http_request(&mut stream).map_or_else(
        |response| response,
        |request| {
            server.lock().map_or_else(
                |_| {
                    text_response(
                        500,
                        "Internal Server Error",
                        "MCP server state lock poisoned",
                    )
                },
                |mut server| {
                    handle_streamable_http_request_with_config(&mut server, &request, config)
                },
            )
        },
    );
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
    std::env::var("TDW_MCP_HTTP_TOKEN").map_or_else(
        |_| StreamableHttpConfig::new(),
        |token| StreamableHttpConfig::new().with_auth_token(token),
    )
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
    scheme.eq_ignore_ascii_case("bearer") && constant_time_str_eq(candidate, token)
}

/// Constant-time string equality for secret comparison (bearer tokens).
///
/// A naive `==` short-circuits on the first differing byte, leaking the length
/// of the matching prefix through response timing. To compare without that side
/// channel — and without leaking the secret's length either — both inputs are
/// folded into a fixed-width 32-byte FNV-1a digest and the digests are compared
/// with [`subtle::ConstantTimeEq`]. Equal strings always produce equal digests;
/// unequal strings differ with overwhelming probability, and the comparison work
/// is independent of where (or whether) the inputs diverge.
fn constant_time_str_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    fixed_digest(a.as_bytes())
        .ct_eq(&fixed_digest(b.as_bytes()))
        .into()
}

/// Fold arbitrary-length bytes into a fixed 32-byte digest using FNV-1a across
/// four independently-seeded lanes. Used only to give
/// [`constant_time_str_eq`] equal-length inputs so the downstream
/// constant-time compare neither short-circuits nor leaks input length.
fn fixed_digest(bytes: &[u8]) -> [u8; 32] {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut digest = [0u8; 32];
    for (lane, chunk) in digest.chunks_mut(8).enumerate() {
        let mut hash = OFFSET ^ (lane as u64).wrapping_mul(PRIME);
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // Mix the length into each lane so two inputs of different lengths that
        // happen to collide on content still differ.
        hash ^= bytes.len() as u64;
        hash = hash.wrapping_mul(PRIME);
        chunk.copy_from_slice(&hash.to_le_bytes());
    }
    digest
}

fn bind_is_loopback(bind: &str) -> bool {
    let host = bind
        .rsplit_once(':')
        .map_or(bind, |(host, _)| host)
        .trim_matches(['[', ']']);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Env var holding extra exact origins (comma-separated) the Streamable-HTTP
/// transport accepts in addition to the built-in loopback hosts.
///
/// Each entry is normalized like an incoming `Origin` header (scheme + host
/// lowercased, any trailing `/` and path dropped) before comparison; an entry
/// that is blank or carries no `http://`/`https://` scheme is silently ignored.
/// Unset/empty leaves the loopback-only default unchanged. The bearer-token rule
/// is independent: a non-loopback bind still requires `TDW_MCP_HTTP_TOKEN`.
const ALLOWED_ORIGINS_ENV: &str = "TDW_MCP_ALLOWED_ORIGINS";

/// Normalize an `Origin` value to a comparable `scheme://host[:port]` form, or
/// `None` when it carries no `http`/`https` scheme.
///
/// The scheme and host are ASCII-lowercased (origins are case-insensitive in
/// both) and any path/trailing `/` is dropped so `https://Pro.OpenBB.co/` and
/// `https://pro.openbb.co` compare equal.
fn normalize_origin(origin: &str) -> Option<String> {
    let origin = origin.trim().to_ascii_lowercase();
    let rest = origin
        .strip_prefix("http://")
        .map(|rest| ("http://", rest))
        .or_else(|| {
            origin
                .strip_prefix("https://")
                .map(|rest| ("https://", rest))
        })?;
    let (scheme, rest) = rest;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}{authority}"))
}

/// Extract just the host from a normalized authority (`scheme://host[:port]`),
/// IPv6 brackets stripped.
fn origin_host(normalized: &str) -> &str {
    let authority = normalized
        .strip_prefix("http://")
        .or_else(|| normalized.strip_prefix("https://"))
        .unwrap_or(normalized);
    authority.strip_prefix('[').map_or_else(
        || authority.split(':').next().unwrap_or(""),
        |rest| rest.split(']').next().unwrap_or(""),
    )
}

fn origin_is_allowed(origin: &str) -> bool {
    origin_is_allowed_with(origin, std::env::var(ALLOWED_ORIGINS_ENV).ok().as_deref())
}

/// Pure core of [`origin_is_allowed`], parameterized on the allow-list string so
/// it is testable without mutating the process environment.
///
/// Loopback hosts are always accepted. When `allowed` is `Some`, its
/// comma-separated entries are normalized like the incoming origin and matched
/// exactly; blank/scheme-less entries are skipped. `None` (env unset) leaves the
/// loopback-only default unchanged.
fn origin_is_allowed_with(origin: &str, allowed: Option<&str>) -> bool {
    let Some(normalized) = normalize_origin(origin) else {
        return false;
    };
    if matches!(origin_host(&normalized), "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    allowed.is_some_and(|allowed| {
        allowed
            .split(',')
            .filter_map(normalize_origin)
            .any(|entry| entry == normalized)
    })
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
            serde_json::from_str::<Value>(message)
                .unwrap_or_else(|_| Value::String(message.clone()))
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
        Some(_) | None => Value::Null,
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

fn success_message(id: &Value, result: &Value) -> Value {
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

fn notification_message(method: &str, params: &Value) -> Value {
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

/// `TDW_MCP_SAMPLE_TOOLS=1` opts the `*.sample` demo tools into `tools/list`.
fn sample_tools_enabled() -> bool {
    std::env::var("TDW_MCP_SAMPLE_TOOLS").ok().as_deref() == Some("1")
}

fn tool_descriptors() -> Vec<ToolDescriptor> {
    let mut descriptors = tool_descriptors_evidence();
    descriptors.extend(tool_descriptors_client_and_daemon());
    descriptors.extend(tool_descriptors_widgets());
    descriptors.extend(tool_descriptors_routes());
    descriptors
}

/// Read-only widget-catalog tools, backed by the `tdw-widgets` projection of the
/// `OpenBB` Workspace `widgets.json` / `apps.json` contract. They let an agent
/// inside a Workspace app enumerate the derived widgets and apps without leaving
/// MCP, and they share the catalog every widget citation points back to.
fn tool_descriptors_widgets() -> Vec<ToolDescriptor> {
    vec![
        tool(
            "tdw.widgets.list",
            "List Workspace Widgets",
            "List the OpenBB Workspace widgets derived from the TDW endpoint catalog (id, name, category, and backend endpoint per widget).",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tdw.widgets.describe",
            "Describe Workspace Widget",
            "Return the full OpenBB Workspace WidgetConfig JSON for one widget id (call tdw.widgets.list for the available ids).",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Widget id, for example equity_price_historical." }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "tdw.apps.list",
            "List Workspace Apps",
            "List the curated OpenBB Workspace apps (name and description per app) derived from apps.json.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]
}

/// Dotted-namespace prefix for the dynamically-generated per-route Fetch tools.
///
/// Every `tdw-endpoint-catalog` `Fetch` route is exposed as one MCP tool whose
/// id is `tdw.route.` followed by the slash-route with each `'/'` rewritten to
/// `'.'` — e.g. route `equity/fundamental/income` →
/// `tdw.route.equity.fundamental.income`. The prefix is distinct from every
/// hand-wired static tool (which live under `tdw.<area>.<noun>` and never under
/// `tdw.route.`), so the families cannot collide.
const ROUTE_TOOL_PREFIX: &str = "tdw.route.";

/// Environment variable that disables the dynamic per-route tool family.
///
/// Set `TDW_MCP_ROUTE_TOOLS=off` (case-insensitive) to drop every `tdw.route.*`
/// tool from `tools/list` and reject their dispatch as unknown, so operators can
/// fall back to the fixed hand-wired surface. Any other value (or unset) keeps
/// the family enabled.
const ROUTE_TOOLS_ENV: &str = "TDW_MCP_ROUTE_TOOLS";

/// Whether the dynamic per-route tool family is enabled (default on).
///
/// Disabled only when [`ROUTE_TOOLS_ENV`] is set to `off` (case-insensitive,
/// trimmed); every other value keeps it on so the default surface advertises
/// every Fetch route.
fn route_tools_enabled() -> bool {
    route_tools_enabled_from(non_empty_env(ROUTE_TOOLS_ENV).as_deref())
}

/// Pure gate decision over the raw [`ROUTE_TOOLS_ENV`] value, split out so it is
/// testable without mutating process-global environment. `None` (unset/empty) is
/// enabled; `Some("off")` (case-insensitive) is the only disable.
fn route_tools_enabled_from(value: Option<&str>) -> bool {
    value.is_none_or(|value| !value.eq_ignore_ascii_case("off"))
}

/// Derive the MCP tool id for a catalog `route` by prefixing [`ROUTE_TOOL_PREFIX`]
/// and rewriting each route separator (`'/'`) to a dot.
fn route_tool_name(route: &str) -> String {
    format!(
        "{ROUTE_TOOL_PREFIX}{}",
        route.replace(tdw_endpoint_catalog::ROUTE_SEP, ".")
    )
}

/// Recover the slash-namespaced catalog route from a `tdw.route.*` tool id.
///
/// Returns `None` when `name` does not carry the [`ROUTE_TOOL_PREFIX`]. The
/// inverse of [`route_tool_name`]: strips the prefix and rewrites each `'.'`
/// back to a route separator. Catalog segments are `[a-z0-9_]` only, so no
/// segment contains a literal `'.'` and the round-trip is unambiguous.
fn route_from_tool_name(name: &str) -> Option<String> {
    name.strip_prefix(ROUTE_TOOL_PREFIX)
        .map(|tail| tail.replace('.', &tdw_endpoint_catalog::ROUTE_SEP.to_string()))
}

/// One dynamically-generated read-only Fetch tool per catalog `Fetch` route.
///
/// Reads [`tdw_endpoint_catalog::catalog`] at call time (never a hardcoded list)
/// and emits exactly one [`ToolDescriptor`] per `EndpointKind::Fetch` route, in
/// catalog order. `Compute` routes are intentionally skipped: they derive their
/// result from caller-supplied series data rather than a provider fetch, so they
/// need a different argument contract and are deferred to a later story. Returns
/// an empty vector when the family is disabled via [`ROUTE_TOOLS_ENV`].
fn tool_descriptors_routes() -> Vec<ToolDescriptor> {
    if !route_tools_enabled() {
        return Vec::new();
    }
    tdw_endpoint_catalog::catalog()
        .into_iter()
        // Compute routes need caller-supplied input data, not a provider fetch;
        // out of scope for the v1 per-route Fetch family (deferred).
        .filter(|entry| entry.kind == tdw_endpoint_catalog::EndpointKind::Fetch)
        .map(|entry| {
            tool(
                &route_tool_name(entry.route),
                &route_tool_title(entry.route),
                &format!(
                    "Fetch the `{}` catalog route through the TDW daemon and return its records. \
                     {DATA_MODE_DISCLOSURE} {}",
                    entry.route, entry.doc
                ),
                route_tool_input_schema(&entry),
            )
        })
        .collect()
}

/// A short human title for a route tool, e.g. `equity/fundamental/income` →
/// `Fetch equity/fundamental/income`.
fn route_tool_title(route: &str) -> String {
    format!("Fetch {route}")
}

/// Build a route tool's `inputSchema` from the route's `params_schema`, adding an
/// optional `provider` enum (the route's candidate providers) for pinning a
/// candidate.
///
/// The catalog's `params_schema` is the route's standardized query params; its
/// `properties` are carried through verbatim as the tool's own properties so the
/// agent sees the same fields the daemon decodes. A `provider` property is added
/// whose `enum` is the route's candidate provider ids (declaration order = the
/// daemon's fallback preference, so the first entry is the default). The schema
/// is left `additionalProperties: true` so any future standard param flows
/// through to the daemon without a tdw-mcp change — this crate adds no routes and
/// pins no param contract of its own.
fn route_tool_input_schema(entry: &tdw_endpoint_catalog::CatalogEntry) -> Value {
    let params_schema = serde_json::to_value((entry.params_schema)()).unwrap_or(Value::Null);
    let mut properties = params_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let providers: Vec<Value> = entry
        .candidates
        .iter()
        .map(|candidate| Value::String(candidate.provider.to_string()))
        .collect();
    let default_provider = entry.candidates.first().map(|candidate| candidate.provider);
    let provider_description = default_provider.map_or_else(
        || "Provider id to pin a candidate for this route.".to_string(),
        |provider| format!("Provider id to pin a candidate; defaults to {provider}."),
    );
    properties.insert(
        "provider".to_string(),
        json!({
            "type": "string",
            "description": provider_description,
            "enum": providers,
        }),
    );

    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": true,
    })
}

/// First group of built-in MCP tool descriptors (providers through KG/tag evidence).
fn tool_descriptors_evidence() -> Vec<ToolDescriptor> {
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
            &format!(
                "Fetch equity historical data through the TDW provider registry. {DATA_MODE_DISCLOSURE}"
            ),
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
            "tdw.provider.fetch",
            "Fetch Any Registered Provider",
            &format!(
                "Dispatch any registered TDW fetcher by (provider, endpoint) and return its \
                 OBBject as JSON. {} (provider, endpoint) pairs are dispatchable in this \
                 build — call tdw.providers.list for the catalog (kind=Fetcher only; Streamer \
                 endpoints are not dispatchable here). Keyed providers read their API keys \
                 from environment variables at fetch time; a missing key surfaces as a tool \
                 error on first use.",
                tdw_service_api::provider_fetch_targets().len()
            ),
            json!({
                "type": "object",
                "properties": {
                    "provider": { "type": "string", "description": "Provider id, for example coingecko or fileset." },
                    "endpoint": { "type": "string", "description": "Endpoint id for that provider, for example ohlc or equity_historical." },
                    "params": {
                        "type": "object",
                        "description": "Provider-specific query parameters. Defaults to {}.",
                        "additionalProperties": true
                    }
                },
                "required": ["provider", "endpoint"],
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
    ]
}

/// Second group of built-in MCP tool descriptors (client-event evidence plus daemon tools).
fn tool_descriptors_client_and_daemon() -> Vec<ToolDescriptor> {
    vec![
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

/// Convert a `tdw-agent` registry's `tool` resources into tdw-mcp [`ToolDescriptor`]s for
/// `tools/list`.
///
/// Each registry [`tdw_agent::McpTool`] maps as: `name` = `base.name`; `title` =
/// `base.title` (falling back to `base.name`); `description` = `base.description` (falling
/// back to the title); `inputSchema`/`outputSchema` are carried through verbatim; the
/// behavioral hints come from the tool's MCP annotations.
///
/// Note: this only surfaces registry tools in `tools/list`. `tools/call` dispatch for
/// registry-backed tools is intentionally OUT OF SCOPE — `execute_tool` does not know how
/// to run them. Rather than the generic `-32602 "unknown tool"`, calling a listed-but-
/// unexecutable registry tool returns the distinct `-32601 "registry tool not yet
/// executable"` (see [`McpServer::is_listed_registry_tool`]); truly-unknown names still
/// get `-32602` until a dispatch path is wired up.
///
/// Why execution is deferred: actually running a registry tool requires an execution backend
/// keyed off the tool's `implementation`/origin — concretely one of (a) a proxy that forwards
/// the call to a sub-MCP server, (b) a bound Rust function, or (c) an HTTP endpoint. None of
/// these exist in this crate yet, so wiring one in is left to a follow-up. Listing tools
/// without an executor is deliberate: the surface stays a "functional packet" where tools are
/// advertised truthfully and calls return a precise not-implemented error.
///
/// Protocol-version skew: tdw-agent's MCP adapter targets revision `2025-11-25`, while this
/// server negotiates [`MCP_PROTOCOL_VERSION`] (`2025-06-18`). The projected [`ToolDescriptor`]
/// only emits fields common to both revisions — name/title/description/inputSchema/
/// outputSchema plus the four boolean annotation hints (read-only/destructive/idempotent/
/// open-world) — and omits `icons` and other 11-25-only fields, so the projection stays
/// wire-safe under the older server protocol.
#[must_use]
pub(crate) fn registry_tool_descriptors(registry: &Registry) -> Vec<ToolDescriptor> {
    tdw_service_api::registry_mcp_tools(registry)
        .into_iter()
        .map(|mcp_tool| {
            let title = mcp_tool.display_name().to_string();
            let description = mcp_tool
                .base
                .description
                .clone()
                .unwrap_or_else(|| title.clone());
            let annotations = mcp_tool.annotations.as_ref();
            // Tools whose `implementation` cannot run in this build are listed but not
            // runnable. Surface that to clients via a `notExecutable` hint so the catalog
            // stays truthful (decision 4).
            let not_executable = registry_tool_not_executable(registry, &mcp_tool.base.name);
            ToolDescriptor {
                name: mcp_tool.base.name.clone(),
                title,
                description,
                input_schema: mcp_tool.input_schema.clone(),
                output_schema: mcp_tool.output_schema.clone(),
                annotations: json!({
                    "readOnlyHint": annotations
                        .and_then(|hints| hints.read_only_hint)
                        .unwrap_or(false),
                    "destructiveHint": annotations
                        .and_then(|hints| hints.destructive_hint)
                        .unwrap_or(false),
                    "idempotentHint": annotations
                        .and_then(|hints| hints.idempotent_hint)
                        .unwrap_or(false),
                    "openWorldHint": annotations
                        .and_then(|hints| hints.open_world_hint)
                        .unwrap_or(false),
                    "notExecutable": not_executable,
                }),
            }
        })
        .collect()
}

/// True when the registry `tool` named `name` is *not* runnable in this build.
///
/// Runnable implementations are `Builtin` and `Command { background: false }`. Everything
/// else (`Unbound`, `Pty`, `Wasm`, `Ref`, `Http`, `Mcp`, and `Command { background: true }`)
/// is listed but not executable, so it is surfaced with `notExecutable: true`. A missing
/// tool or one that fails to re-type is treated as executable (it is either absent or carries
/// a concrete binding the listing already reflects).
fn registry_tool_not_executable(registry: &Registry, name: &str) -> bool {
    registry
        .get(tdw_agent::EntityKind::Tool, name)
        .and_then(|resource| tdw_agent::entity_from_resource::<tdw_agent::Tool>(resource).ok())
        .is_some_and(|tool| {
            !matches!(
                tool.implementation,
                tdw_agent::ToolImplementation::Builtin { .. }
                    | tdw_agent::ToolImplementation::Command {
                        background: false,
                        ..
                    }
            )
        })
}

/// One-line market-data provenance disclosure, baked in at compile time.
///
/// The P1.3 audit found offline fixture bars are indistinguishable from real
/// market data in tool results, so the server now says which one it serves in
/// both the `initialize` instructions and the data tools' descriptions.
#[cfg(feature = "live")]
const DATA_MODE_DISCLOSURE: &str =
    "Market-data tools serve LIVE provider data in this build (the `live` feature is enabled).";
#[cfg(not(feature = "live"))]
const DATA_MODE_DISCLOSURE: &str = "Market-data tools serve DETERMINISTIC OFFLINE FIXTURES in      this build, not real market data (rebuild with `--features live` for live providers).";

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
        "tdw.provider.fetch" => {
            let provider = required_argument(arguments_object, "provider")?;
            let endpoint = required_argument(arguments_object, "endpoint")?;
            let params = optional_object_argument(arguments_object, "params")?
                .cloned()
                .map_or_else(|| json!({}), Value::Object);
            let response = tdw_service_api::fetch_provider_json(provider, endpoint, params)
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
        "tdw.widgets.list" => Ok(structured(json!({ "widgets": widget_summaries() }))),
        "tdw.widgets.describe" => execute_widget_describe(arguments_object),
        "tdw.apps.list" => Ok(structured(json!({ "apps": app_summaries() }))),
        _ => {
            // Dynamic per-route Fetch tools share the `tdw.route.*` namespace and
            // dispatch through the daemon; everything else is genuinely unknown.
            if let Some(route) = route_from_tool_name(name).filter(|_| route_tools_enabled()) {
                return execute_route_tool(daemon, &route, arguments_object);
            }
            Err(ToolFailure::Protocol(JsonRpcProblem::new(
                Value::Null,
                -32602,
                format!("unknown tool: {name}"),
            )))
        }
    }
}

/// Dispatch a dynamic `tdw.route.*` tool: resolve the catalog `Fetch` route,
/// build a no-cache [`Op::FetchData`] envelope (mirroring how the daemon tools
/// build their `RunQuery` envelope), submit it through the configured daemon, and
/// return the terminal event.
///
/// Resolution posture matches the existing daemon-backed tools: an unknown route,
/// a `Compute` route (no provider fetch), or a `provider` that is not one of the
/// route's catalog candidates is a **tool error** (`isError`), never a protocol
/// error; a missing daemon surfaces as a tool error from [`DaemonToolRuntime::submit`].
/// The route's other arguments are passed through verbatim into the `FetchData`
/// `params` so the daemon decodes them exactly as a REST caller would.
fn execute_route_tool(
    daemon: &DaemonToolRuntime,
    route: &str,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let Some(entry) = tdw_endpoint_catalog::lookup(route) else {
        return Err(ToolFailure::Execution(format!(
            "unknown catalog route: {route}"
        )));
    };
    if entry.kind != tdw_endpoint_catalog::EndpointKind::Fetch {
        return Err(ToolFailure::Execution(format!(
            "route {route} is not a Fetch route and is not dispatchable here"
        )));
    }
    // A pinned provider must be one of the route's catalog candidates; an unknown
    // pin is a tool error rather than a silent fallback to the default candidate.
    if let Some(provider) = optional_argument(arguments, "provider")
        && !entry
            .candidates
            .iter()
            .any(|candidate| candidate.provider == provider)
    {
        return Err(ToolFailure::Execution(format!(
            "provider {provider} is not a candidate for route {route}"
        )));
    }

    let params = Value::Object(arguments.clone());
    let envelope = route_fetch_data_envelope(route, params)?;
    let submission = daemon.submit(&envelope).map_err(ToolFailure::Execution)?;
    Ok(structured(daemon_submission_value(
        &route_tool_name(route),
        &submission,
        &json!({ "route": route }),
    )))
}

/// Session id stamped on every per-route Fetch op, mirroring the diagnostic
/// daemon tools' `session-mcp-daemon` default. The per-route tools carry no
/// caller-supplied session, so a fixed label keeps the daemon's event stream
/// attributable to this entrypoint.
const ROUTE_TOOL_SESSION_ID: &str = "session-mcp-route";

/// Build the `Op::FetchData` envelope for a per-route tool dispatch.
///
/// Mirrors [`daemon_run_query_envelope`]'s actor/session shape so the per-route
/// tools and the diagnostic daemon tools present an identical caller identity to
/// the daemon. `params` is the tool's arguments object passed straight through —
/// it carries the route's standard query params plus an optional `provider` the
/// daemon's resolver reads to pin a candidate.
fn route_fetch_data_envelope(route: &str, params: Value) -> Result<OpEnvelope, ToolFailure> {
    let session_id = SessionId::new(ROUTE_TOOL_SESSION_ID)
        .map_err(|error| protocol_argument_failure(error.to_string()))?;
    Ok(OpEnvelope::new(
        session_id,
        1,
        ActorRef {
            actor_id: "mcp:tdw-mcp".to_string(),
            kind: ActorKind::Service,
            tenant_id: Some("default".to_string()),
        },
        Op::FetchData {
            route: route.to_string(),
            params,
        },
    ))
}

/// `[{ id, name, category, endpoint }]` for every derived widget, in catalog order.
fn widget_summaries() -> Vec<Value> {
    tdw_widgets::catalog_widgets()
        .into_iter()
        .map(|widget| {
            json!({
                "id": widget.id,
                "name": widget.name,
                "category": widget.category,
                "endpoint": widget.endpoint,
            })
        })
        .collect()
}

/// `[{ name, description }]` for every curated Workspace app in `apps.json`.
fn app_summaries() -> Vec<Value> {
    tdw_widgets::apps_json()
        .as_object()
        .map(|apps| {
            apps.values()
                .filter_map(|app| {
                    let name = app.get("name")?.as_str()?;
                    let description = app.get("description").and_then(Value::as_str).unwrap_or("");
                    Some(json!({ "name": name, "description": description }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn execute_widget_describe(arguments: &Map<String, Value>) -> Result<ToolExecution, ToolFailure> {
    let id = required_argument(arguments, "id")?;
    tdw_widgets::catalog_widgets()
        .into_iter()
        .find(|widget| widget.id == id)
        .ok_or_else(|| ToolFailure::Execution(format!("unknown widget id: {id}")))
        .and_then(|widget| {
            serde_json::to_value(&widget)
                .map(structured)
                .map_err(|error| ToolFailure::Execution(error.to_string()))
        })
}

fn execute_daemon_triage(
    daemon: &DaemonToolRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let requested_op_id = optional_argument(arguments, "op_id").map(ToString::to_string);
    let sql = "select 'tdw.daemon.triage' as diagnostic_check";
    let envelope = daemon_run_query_envelope(arguments, sql)?;
    let submission = daemon.submit(&envelope).map_err(ToolFailure::Execution)?;
    Ok(structured(daemon_submission_value(
        "tdw.daemon.triage",
        &submission,
        &json!({
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
    let submission = daemon.submit(&envelope).map_err(ToolFailure::Execution)?;
    Ok(structured(daemon_submission_value(
        "tdw.daemon.query.submit",
        &submission,
        &json!({
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

fn daemon_submission_value(tool: &str, submission: &DaemonSubmission, extra: &Value) -> Value {
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

const fn daemon_transport_label(transport: DaemonTransport) -> &'static str {
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

fn tool_result(structured: &Value) -> Value {
    json!({
        "content": [
            {
                "type": "text",
                "text": pretty_json(structured),
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
                    &json!({
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
                &json!({
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

/// Read an optional object-valued argument. Absent → `Ok(None)`; present and an
/// object → `Ok(Some(_))`; present but not an object → a `-32602` protocol error.
fn optional_object_argument<'a>(
    arguments: &'a Map<String, Value>,
    name: &str,
) -> Result<Option<&'a Map<String, Value>>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(object)) => Ok(Some(object)),
        Some(_) => Err(protocol_argument_failure(format!(
            "{name} must be an object"
        ))),
    }
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
        "tdw://quality/mcp-worker-product-boundaries" => {
            Ok(resource_text(uri, "text/markdown", MCP_BOUNDARY_DOC))
        }
        "tdw://quality/daemon-hardening-test-taxonomy" => {
            Ok(resource_text(uri, "text/markdown", TEST_TAXONOMY_DOC))
        }
        "tdw://service/protocol-config-sample" => {
            let sample = tdw_service_api::protocol_config_sample().map_err(|error| {
                JsonRpcProblem::new(
                    Value::Null,
                    -32603,
                    format!("protocol config resource failed: {error}"),
                )
            })?;
            Ok(resource_text(
                uri,
                "application/json",
                &pretty_json(&sample),
            ))
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
                &pretty_json(&capabilities),
            ))
        }
        _ => Err(JsonRpcProblem::new(
            Value::Null,
            -32602,
            format!("unknown resource: {uri}"),
        )),
    }
}

fn resource_text(uri: &str, mime_type: &str, text: &str) -> Value {
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
                &format!(
                    "Use TDW MCP tools to research {symbol} with provider {provider} over horizon {horizon}. Start with tdw.providers.list, fetch tdw.equity.historical, call tdw.kg_tag.sample for context, then summarize data quality, rows observed, warehouse follow-ups, and risk notes."
                ),
            ))
        }
        "tdw.daemon.triage" => {
            let op_id = optional_argument(arguments_object, "op_id").unwrap_or("the target op");
            Ok(prompt_messages(
                "TDW daemon operation triage",
                &format!(
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
                &format!(
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

fn prompt_messages(description: &str, text: &str) -> Value {
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

    /// `(command, args)` for a shell that runs `script`, portable across CI runners.
    /// On Windows: `cmd /c <script>`; on Unix: `sh -c "<script>"`.
    fn shell_command(script: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/c".to_string(), script.to_string()],
            )
        } else {
            ("sh".to_string(), vec!["-c".to_string(), script.to_string()])
        }
    }

    /// The shell binary name, for the executor allow-list.
    fn shell_bin() -> &'static str {
        if cfg!(windows) { "cmd" } else { "sh" }
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

    fn visible_builtin_count() -> usize {
        mcp_tool_catalog()
            .iter()
            .filter(|name| !name.ends_with(".sample"))
            .count()
    }

    #[test]
    fn tools_list_hides_sample_tools_by_default_but_keeps_them_callable() {
        let mut server = McpServer::new().with_sample_tools(false);
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));
        assert!(
            !tools.iter().any(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| name.ends_with(".sample"))
            }),
            "sample tools must be hidden from the default catalog"
        );
        // Hidden does not mean disabled: the sample tool still executes.
        let call = decode(&server.handle_json_rpc_line(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.agent.sample","arguments":{}}}"#,
        )[0]);
        assert_eq!(call["result"]["isError"], Value::Bool(false));
    }

    #[test]
    fn tools_list_returns_spec_shaped_descriptors() {
        let mut server = McpServer::new().with_sample_tools(false);
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
        assert_eq!(visible_builtin_count(), tools.len());
    }

    #[test]
    fn tools_list_includes_registry_tools_alongside_builtins() {
        use tdw_agent::{
            Adaptivity, EntityMeta, Origin, Registry, RegistryEntity, Source, Tier, Tool,
            ToolEffect,
        };

        let registry_tool = Tool {
            meta: EntityMeta::new(
                "registry.search",
                "registry.search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Registry Search")
            .with_description("Search exposed via the tdw-agent registry."),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: tdw_agent::ToolImplementation::Unbound,
        };
        let registry = Registry::from_resources([registry_tool
            .to_resource()
            .unwrap_or_else(|error| panic!("tool resource: {error}"))])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        let mut server = McpServer::new()
            .with_sample_tools(false)
            .with_registry(registry);
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));

        // Built-ins remain present.
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "tdw.equity.historical")
        );
        // The registry tool is appended.
        let registry_descriptor = tools
            .iter()
            .find(|tool| tool["name"] == "registry.search")
            .unwrap_or_else(|| panic!("registry tool should be listed"));
        assert_eq!(registry_descriptor["title"], "Registry Search");
        assert!(registry_descriptor["inputSchema"].is_object());
        // Total = built-ins + the one registry tool.
        assert_eq!(tools.len(), visible_builtin_count() + 1);
    }

    #[test]
    fn registry_descriptor_cache_is_consistent_and_refreshes() {
        use tdw_agent::{
            Adaptivity, EntityMeta, Origin, Registry, RegistryEntity, Source, Tier, Tool,
            ToolEffect,
        };

        let make_registry = |name: &str| {
            let tool = Tool {
                meta: EntityMeta::new(
                    name,
                    name,
                    "0.1.0",
                    Origin {
                        tier: Tier::Domain,
                        source: Source::Internal,
                    },
                    Adaptivity::None,
                    false,
                )
                .with_title("Cached Registry Tool")
                .with_description("Tool exercising the attach-time descriptor cache."),
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                output_schema: None,
                effect: ToolEffect::ReadOnly,
                idempotent: true,
                open_world: false,
                implementation: tdw_agent::ToolImplementation::Unbound,
            };
            Registry::from_resources([tool
                .to_resource()
                .unwrap_or_else(|error| panic!("tool resource: {error}"))])
            .unwrap_or_else(|error| panic!("registry should build: {error}"))
        };

        let mut server = McpServer::new();
        server.set_registry(make_registry("registry.alpha"));

        // The cache feeds both hot paths: the registry tool is listed and recognized.
        assert!(
            server
                .all_tool_descriptors()
                .iter()
                .any(|tool| tool.name == "registry.alpha")
        );
        assert!(server.is_listed_registry_tool("registry.alpha"));
        // A name that is not in the registry is not listed.
        assert!(!server.is_listed_registry_tool("registry.beta"));

        // A second `set_registry` refreshes the cache for the new registry.
        server.set_registry(make_registry("registry.beta"));
        assert!(
            server
                .all_tool_descriptors()
                .iter()
                .any(|tool| tool.name == "registry.beta")
        );
        assert!(server.is_listed_registry_tool("registry.beta"));
        // The previous registry's tool is gone from both the listing and the membership check.
        assert!(
            !server
                .all_tool_descriptors()
                .iter()
                .any(|tool| tool.name == "registry.alpha")
        );
        assert!(!server.is_listed_registry_tool("registry.alpha"));
    }

    #[test]
    fn tools_list_dedups_registry_tool_colliding_with_builtin() {
        use tdw_agent::{Adaptivity, EntityMeta, Origin, Registry, Source, Tier, Tool, ToolEffect};

        use tdw_agent::RegistryEntity;

        // Registry tool intentionally collides with a built-in name.
        let colliding = Tool {
            meta: EntityMeta::new(
                "tdw.agent.sample",
                "tdw.agent.sample",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Shadow Agent Sample")
            .with_description("Registry tool colliding with a built-in name."),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: tdw_agent::ToolImplementation::Unbound,
        };
        let registry = Registry::from_resources([colliding
            .to_resource()
            .unwrap_or_else(|error| panic!("tool resource: {error}"))])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        let mut server = McpServer::new()
            .with_sample_tools(true)
            .with_registry(registry);
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));

        // The colliding name appears exactly once...
        let occurrences = tools
            .iter()
            .filter(|tool| tool["name"] == "tdw.agent.sample")
            .count();
        assert_eq!(occurrences, 1);
        // ...and it is the built-in (title from tool_descriptors, not the registry title).
        let descriptor = tools
            .iter()
            .find(|tool| tool["name"] == "tdw.agent.sample")
            .unwrap_or_else(|| panic!("colliding tool should be listed once"));
        assert_eq!(descriptor["title"], "Agent Surface Evidence");
        // No net add for the colliding name: total equals the built-in count.
        assert_eq!(tools.len(), mcp_tool_catalog().len());
    }

    #[test]
    fn tools_call_registry_tool_returns_method_not_found_not_unknown() {
        use tdw_agent::{Adaptivity, EntityMeta, Origin, Registry, Source, Tier, Tool, ToolEffect};

        use tdw_agent::RegistryEntity;

        let registry_tool = Tool {
            meta: EntityMeta::new(
                "registry.search",
                "registry.search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Registry Search")
            .with_description("Search exposed via the tdw-agent registry."),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: tdw_agent::ToolImplementation::Unbound,
        };
        let registry = Registry::from_resources([registry_tool
            .to_resource()
            .unwrap_or_else(|error| panic!("tool resource: {error}"))])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        let mut server = McpServer::new().with_registry(registry);
        initialize(&mut server);

        // A listed-but-stubbed registry tool yields -32601 (method not found).
        let listed = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"registry.search","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(listed["error"]["code"], -32601);
        assert!(
            listed["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("registry.search"))
        );

        // A genuinely-unknown name still yields -32602 (invalid params / unknown tool).
        let unknown = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.does.not.exist","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(unknown["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_executes_bound_command_registry_tool() {
        use tdw_agent::{
            Adaptivity, EntityMeta, Origin, Registry, Source, Tier, Tool, ToolEffect,
            ToolImplementation,
        };

        use tdw_agent::RegistryEntity;

        let command_tool = Tool {
            meta: EntityMeta::new(
                "registry.echo",
                "registry.echo",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Registry Echo")
            .with_description("Runs a captured command via the tool-execution backend."),
            input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: {
                let (command, args) = shell_command("echo hello");
                ToolImplementation::Command {
                    command,
                    args,
                    background: false,
                }
            },
        };
        let registry = Registry::from_resources([command_tool
            .to_resource()
            .unwrap_or_else(|error| panic!("tool resource: {error}"))])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        // The executor denies command execution unless the command is allow-listed. Inject an
        // explicit `CommandPolicy` (race-free, no process-global env mutation) instead of
        // setting `TDW_TOOL_EXEC_ALLOWED_COMMANDS`.
        let executor = tdw_tool_exec::ToolExecutor::new().with_command_policy(
            tdw_tool_exec::CommandPolicy::new(
                Some(vec![shell_bin().to_string()]),
                std::time::Duration::from_secs(30),
            ),
        );
        let mut server = McpServer::new()
            .with_registry(registry)
            .with_executor(executor);
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"registry.echo","arguments":{}}}"#,
            )[0],
        );
        // The bound tool actually runs (no -32601) and returns the captured output.
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(response["result"]["structuredContent"]["exitCode"], 0);
        assert!(
            response["result"]["structuredContent"]["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("hello")
        );
    }

    #[test]
    fn unbound_registry_tool_is_listed_not_executable_and_calls_yield_method_not_found() {
        use tdw_agent::{
            Adaptivity, EntityMeta, Origin, Registry, Source, Tier, Tool, ToolEffect,
            ToolImplementation,
        };

        use tdw_agent::RegistryEntity;

        let make_tool = |name: &str, implementation: ToolImplementation| -> Tool {
            Tool {
                meta: EntityMeta::new(
                    name,
                    name,
                    "0.1.0",
                    Origin {
                        tier: Tier::Domain,
                        source: Source::Internal,
                    },
                    Adaptivity::None,
                    false,
                )
                .with_title(name)
                .with_description("registry tool: listed."),
                input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
                output_schema: None,
                effect: ToolEffect::ReadOnly,
                idempotent: true,
                open_world: false,
                implementation,
            }
        };

        let unbound_tool = make_tool("registry.stub", ToolImplementation::Unbound);
        let pty_tool = make_tool(
            "registry.pty",
            ToolImplementation::Pty {
                command: "bash".to_string(),
                args: Vec::new(),
            },
        );
        let command_tool = make_tool(
            "registry.cmd",
            ToolImplementation::Command {
                command: shell_bin().to_string(),
                args: Vec::new(),
                background: false,
            },
        );
        let registry = Registry::from_resources([
            unbound_tool
                .to_resource()
                .unwrap_or_else(|error| panic!("tool resource: {error}")),
            pty_tool
                .to_resource()
                .unwrap_or_else(|error| panic!("tool resource: {error}")),
            command_tool
                .to_resource()
                .unwrap_or_else(|error| panic!("tool resource: {error}")),
        ])
        .unwrap_or_else(|error| panic!("registry should build: {error}"));

        let mut server = McpServer::new().with_registry(registry);
        initialize(&mut server);

        // tools/list marks the unbound and pty tools not-executable, but the runnable
        // foreground Command tool executable.
        let listed = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));
        let find = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("tool {name} should be listed"))
        };
        let descriptor = find("registry.stub");
        assert_eq!(descriptor["annotations"]["notExecutable"], true);
        assert_eq!(find("registry.pty")["annotations"]["notExecutable"], true);
        assert_eq!(find("registry.cmd")["annotations"]["notExecutable"], false);

        // tools/call on the unbound tool still yields -32601.
        let called = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"registry.stub","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(called["error"]["code"], -32601);
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
    fn tools_call_provider_fetch_dispatches_fileset_structured_content() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.provider.fetch","arguments":{"provider":"fileset","endpoint":"equity_historical","params":{"symbol":"aapl"}}}}"#,
            )[0],
        );
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["structuredContent"]["provider"],
            "fileset"
        );
        assert_eq!(
            response["result"]["structuredContent"]["endpoint"],
            "equity_historical"
        );
        assert_eq!(
            response["result"]["structuredContent"]["rows"][0]["symbol"],
            "AAPL"
        );
        assert_eq!(response["result"]["content"][0]["type"], "text");
    }

    #[test]
    fn tools_call_provider_fetch_unknown_provider_is_tool_error_not_protocol_error() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tdw.provider.fetch","arguments":{"provider":"nope","endpoint":"missing"}}}"#,
            )[0],
        );
        assert!(
            response["error"].is_null(),
            "unknown provider must not be a protocol error: {response}"
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("no fetcher for nope/missing")),
            "unexpected error text: {response}"
        );
    }

    #[test]
    fn daemon_config_uses_config_and_env_style_overrides() {
        let mut config = TdwConfig::default();
        config.daemon.tcp_bind = Some("127.0.0.1:9001".to_string());

        let from_config = daemon_client_config_from_sources(&config, None, None, Some("75"))
            .unwrap_or_else(|error| panic!("config override should resolve: {error}"));
        assert_eq!(from_config.endpoint().address, "127.0.0.1:9001");
        assert_eq!(from_config.timeout(), Duration::from_millis(75));

        let from_env_style = daemon_client_config_from_sources(
            &TdwConfig::default(),
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
    fn constant_time_str_eq_matches_equality_semantics() {
        // Equal strings compare equal.
        assert!(constant_time_str_eq(
            "super-secret-token",
            "super-secret-token"
        ));
        assert!(constant_time_str_eq("", ""));
        // Any difference — content, a single byte, or length — compares unequal.
        assert!(!constant_time_str_eq(
            "super-secret-token",
            "super-secret-toke"
        ));
        assert!(!constant_time_str_eq(
            "super-secret-token",
            "Super-secret-token"
        ));
        assert!(!constant_time_str_eq("secret", "secret-with-suffix"));
        assert!(!constant_time_str_eq("secret", ""));
        // A correct-length-but-wrong-content candidate is rejected.
        assert!(!constant_time_str_eq("aaaaaa", "bbbbbb"));
    }

    #[test]
    fn request_is_authorized_uses_constant_time_token_compare() {
        let config = StreamableHttpConfig::new().with_auth_token("secret");

        let correct = StreamableHttpRequest::new(
            "POST",
            "/mcp",
            vec![("Authorization".to_string(), "Bearer secret".to_string())],
            "",
        );
        assert!(request_is_authorized(&correct, &config));

        // Wrong token, missing header, and non-bearer scheme are all rejected.
        let wrong = StreamableHttpRequest::new(
            "POST",
            "/mcp",
            vec![("Authorization".to_string(), "Bearer wrong".to_string())],
            "",
        );
        assert!(!request_is_authorized(&wrong, &config));

        let no_header = StreamableHttpRequest::new("POST", "/mcp", Vec::new(), "");
        assert!(!request_is_authorized(&no_header, &config));

        let wrong_scheme = StreamableHttpRequest::new(
            "POST",
            "/mcp",
            vec![("Authorization".to_string(), "Basic secret".to_string())],
            "",
        );
        assert!(!request_is_authorized(&wrong_scheme, &config));

        // Case-insensitive bearer scheme still matches.
        let lower_scheme = StreamableHttpRequest::new(
            "POST",
            "/mcp",
            vec![("Authorization".to_string(), "bearer secret".to_string())],
            "",
        );
        assert!(request_is_authorized(&lower_scheme, &config));

        // With no token configured every request is authorized.
        let open = StreamableHttpConfig::new();
        assert!(request_is_authorized(&no_header, &open));
    }

    #[test]
    fn streamable_http_sse_streams_progress_before_response() {
        let mut server = McpServer::new();
        initialize(&mut server);

        let response = handle_streamable_http_request(
            &mut server,
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new(
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
            &StreamableHttpRequest::new("GET", "/mcp", Vec::new(), ""),
        );
        assert_eq!(no_sse.status, 405);
        assert_eq!(no_sse.header("allow"), Some("GET, POST, OPTIONS"));

        let wrong_path = handle_streamable_http_request(
            &mut server,
            &StreamableHttpRequest::new("POST", "/other", Vec::new(), ""),
        );
        assert_eq!(wrong_path.status, 404);

        let wrong_media = handle_streamable_http_request(
            &mut server,
            &StreamableHttpRequest::new(
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

    #[test]
    fn registry_from_dir_attaches_tools_to_tools_list() {
        // Real filesystem: write one `tool` *.json5, load it via `Registry::load_dir`, attach
        // it to a server, and confirm `tools/list` exposes it alongside the built-ins.
        let dir = std::env::temp_dir().join(format!("tdw_mcp_registry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("mkdir: {error}"));
        std::fs::write(
            dir.join("search.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"tool",
                 metadata:{ name:"registry.dir.search", id:"registry.dir.search", version:"0.1.0",
                   origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false,
                   title:"Registry Dir Search" },
                 spec:{ input_schema:{ type:"object" } } }"#,
        )
        .unwrap_or_else(|error| panic!("write tool: {error}"));

        let registry =
            registry_from_dir(&dir).unwrap_or_else(|error| panic!("registry should load: {error}"));
        let mut server = McpServer::new()
            .with_sample_tools(false)
            .with_registry(registry);
        initialize(&mut server);

        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "tdw.equity.historical")
        );
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "registry.dir.search"),
            "registry tool loaded from dir should be listed"
        );
        assert_eq!(tools.len(), visible_builtin_count() + 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_from_dir_surfaces_load_errors() {
        // A directory that does not exist is a misconfiguration and must surface, not be
        // silently ignored.
        let missing =
            std::env::temp_dir().join(format!("tdw_mcp_registry_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&missing);
        let error = registry_from_dir(&missing)
            .err()
            .unwrap_or_else(|| panic!("loading a missing dir should fail"));
        assert!(
            error
                .to_string()
                .contains("failed to load tdw-agent registry")
        );
    }

    #[test]
    fn registry_from_env_unset_returns_none() {
        // The dir-attach test covers the set path deterministically; here we only assert the
        // unset path so a parallel test that sets the var cannot race us. This is best-effort:
        // if some other test in the binary has the var set, treat that as the set path.
        if std::env::var(REGISTRY_DIR_ENV).is_err() {
            let resolved = registry_from_env()
                .unwrap_or_else(|error| panic!("unset env should not error: {error}"));
            assert!(resolved.is_none(), "unset env must yield no registry");
        } else {
            // The var is set in this process; the unset invariant is not testable here.
        }
    }

    // ---- Configurable origin allow-list (G009) ----

    #[test]
    fn origin_allow_list_default_is_loopback_only() {
        // env unset -> only loopback hosts pass; a remote origin is rejected.
        assert!(origin_is_allowed_with("http://localhost:3000", None));
        assert!(origin_is_allowed_with("http://127.0.0.1:8788", None));
        assert!(origin_is_allowed_with("https://[::1]:8788", None));
        assert!(!origin_is_allowed_with("https://pro.openbb.co", None));
        // A scheme-less / malformed origin is never allowed.
        assert!(!origin_is_allowed_with("pro.openbb.co", None));
    }

    #[test]
    fn origin_allow_list_env_adds_an_exact_origin() {
        let allowed = Some("https://pro.openbb.co");
        assert!(origin_is_allowed_with("https://pro.openbb.co", allowed));
        // Loopback still works alongside the configured origin.
        assert!(origin_is_allowed_with("http://localhost", allowed));
        // A different host is still rejected even with the env set.
        assert!(!origin_is_allowed_with("https://evil.example.com", allowed));
        // Scheme is significant: http:// does not match the configured https:// entry.
        assert!(!origin_is_allowed_with("http://pro.openbb.co", allowed));
    }

    #[test]
    fn origin_allow_list_ignores_malformed_entries() {
        // Blank entries and scheme-less entries are skipped; the one valid entry still works.
        let allowed = Some(" , not-a-url , https://pro.openbb.co , ");
        assert!(origin_is_allowed_with("https://pro.openbb.co", allowed));
        assert!(!origin_is_allowed_with("http://not-a-url", allowed));
        // An allow-list with only malformed entries grants nothing beyond loopback.
        assert!(!origin_is_allowed_with(
            "https://pro.openbb.co",
            Some("garbage,,foo")
        ));
    }

    #[test]
    fn origin_allow_list_is_case_insensitive_on_scheme_and_host() {
        let allowed = Some("HTTPS://Pro.OpenBB.co");
        // Mixed-case incoming origin and trailing slash both normalize to a match.
        assert!(origin_is_allowed_with("https://PRO.openbb.CO/", allowed));
        assert!(origin_is_allowed_with("HTTPS://pro.openbb.co", allowed));
    }

    // ---- Read-only widget-catalog tools (G009) ----

    #[test]
    fn widget_tools_are_listed_and_read_only() {
        let mut server = McpServer::new();
        initialize(&mut server);
        let response = decode(
            &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
        );
        let tools = response["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools should be an array"));
        for name in ["tdw.widgets.list", "tdw.widgets.describe", "tdw.apps.list"] {
            let descriptor = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name} should be listed"));
            assert_eq!(
                descriptor["annotations"]["readOnlyHint"], true,
                "{name} must be annotated read-only"
            );
        }
    }

    #[test]
    fn widgets_describe_returns_the_equity_historical_widget() {
        let mut server = McpServer::new();
        initialize(&mut server);
        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.widgets.describe","arguments":{"id":"equity_price_historical"}}}"#,
            )[0],
        );
        assert_eq!(response["result"]["isError"], false);
        let widget = &response["result"]["structuredContent"];
        assert_eq!(widget["id"], "equity_price_historical");
        assert_eq!(widget["type"], "chart");
        assert_eq!(widget["endpoint"], "/widget-data/equity/price/historical");
    }

    #[test]
    fn widgets_list_includes_the_equity_historical_summary() {
        let mut server = McpServer::new();
        initialize(&mut server);
        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.widgets.list","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(response["result"]["isError"], false);
        let widgets = response["result"]["structuredContent"]["widgets"]
            .as_array()
            .unwrap_or_else(|| panic!("widgets should be an array"));
        let equity = widgets
            .iter()
            .find(|widget| widget["id"] == "equity_price_historical")
            .unwrap_or_else(|| panic!("equity historical widget should be listed"));
        assert_eq!(equity["category"], "equity");
        assert!(equity["name"].is_string(), "summary carries a name");
        assert!(
            equity["endpoint"].is_string(),
            "summary carries an endpoint"
        );
    }

    #[test]
    fn apps_list_returns_the_curated_app() {
        let mut server = McpServer::new();
        initialize(&mut server);
        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.apps.list","arguments":{}}}"#,
            )[0],
        );
        assert_eq!(response["result"]["isError"], false);
        let apps = response["result"]["structuredContent"]["apps"]
            .as_array()
            .unwrap_or_else(|| panic!("apps should be an array"));
        assert!(
            !apps.is_empty(),
            "at least the curated default app is listed"
        );
        assert!(
            apps.iter()
                .all(|app| app["name"].is_string() && app["description"].is_string()),
            "every app summary carries a name and description"
        );
    }

    #[test]
    fn widgets_describe_unknown_id_is_a_tool_error() {
        let mut server = McpServer::new();
        initialize(&mut server);
        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.widgets.describe","arguments":{"id":"no_such_widget"}}}"#,
            )[0],
        );
        // A bad id is a tool-level error (isError), not a protocol error.
        assert!(
            response["error"].is_null(),
            "no protocol error for a bad id"
        );
        assert_eq!(response["result"]["isError"], true);
    }

    // ---- Widget-citation contract (G009) ----

    /// Every `mcp_tool.tool_id` a tdw-widgets widget cites MUST resolve to a real
    /// tool in this server's catalog, so a Workspace widget can never reference a
    /// nonexistent MCP tool. Both crates are visible here (tdw-mcp dev/normal-dep
    /// on tdw-widgets), which is why the contract is asserted in tdw-mcp.
    #[test]
    fn every_widget_mcp_tool_id_exists_in_the_mcp_catalog() {
        let catalog: std::collections::BTreeSet<String> = mcp_tool_catalog().into_iter().collect();
        let mut checked = 0_usize;
        for widget in tdw_widgets::catalog_widgets() {
            if let Some(binding) = widget.mcp_tool.as_ref() {
                checked += 1;
                assert_eq!(
                    binding.mcp_server, "tdw-mcp",
                    "widget {} cites a non-tdw-mcp server: {}",
                    widget.id, binding.mcp_server
                );
                assert!(
                    catalog.contains(&binding.tool_id),
                    "widget {} cites MCP tool '{}' which is absent from the tdw-mcp catalog",
                    widget.id,
                    binding.tool_id
                );
            }
        }
        assert!(
            checked > 0,
            "at least one widget should carry an mcp_tool citation to exercise the contract"
        );
    }

    // ---- Dynamic per-route Fetch tools (L5.6) ----

    /// The number of `Fetch` routes in the live catalog — the exact count of
    /// `tdw.route.*` tools the family must generate.
    fn catalog_fetch_route_count() -> usize {
        tdw_endpoint_catalog::catalog()
            .into_iter()
            .filter(|entry| entry.kind == tdw_endpoint_catalog::EndpointKind::Fetch)
            .count()
    }

    #[test]
    fn route_tool_name_round_trips_through_the_route() {
        let route = "equity/fundamental/income";
        let name = route_tool_name(route);
        assert_eq!(name, "tdw.route.equity.fundamental.income");
        assert_eq!(route_from_tool_name(&name).as_deref(), Some(route));
        // A non-route tool id yields None (the dispatch fall-through stays exact).
        assert_eq!(route_from_tool_name("tdw.equity.historical"), None);
    }

    #[test]
    fn every_fetch_route_produces_exactly_one_route_tool_descriptor() {
        let descriptors = tool_descriptors_routes();
        assert_eq!(
            descriptors.len(),
            catalog_fetch_route_count(),
            "one route tool per Fetch route"
        );
        // Each Fetch route's derived tool id is present exactly once; Compute
        // routes are absent (they are not provider fetches).
        let names: std::collections::BTreeSet<&str> =
            descriptors.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names.len(),
            descriptors.len(),
            "no duplicate route tool ids"
        );
        for entry in tdw_endpoint_catalog::catalog() {
            let expected = route_tool_name(entry.route);
            match entry.kind {
                tdw_endpoint_catalog::EndpointKind::Fetch => assert!(
                    names.contains(expected.as_str()),
                    "Fetch route {} is missing its tool",
                    entry.route
                ),
                tdw_endpoint_catalog::EndpointKind::Compute => assert!(
                    !names.contains(expected.as_str()),
                    "Compute route {} must not produce a route tool",
                    entry.route
                ),
            }
        }
    }

    #[test]
    fn route_tools_are_read_only_idempotent_and_carry_a_provider_enum() {
        for descriptor in tool_descriptors_routes() {
            assert_eq!(descriptor.annotations["readOnlyHint"], true);
            assert_eq!(descriptor.annotations["idempotentHint"], true);
            let route = route_from_tool_name(&descriptor.name)
                .unwrap_or_else(|| panic!("route tool id: {}", descriptor.name));
            let entry =
                tdw_endpoint_catalog::lookup(&route).unwrap_or_else(|| panic!("route {route}"));
            // The provider arg enumerates exactly the route's candidate providers.
            let providers: Vec<&str> = descriptor.input_schema["properties"]["provider"]["enum"]
                .as_array()
                .unwrap_or_else(|| panic!("provider enum for {route}"))
                .iter()
                .map(|value| value.as_str().unwrap_or_default())
                .collect();
            let expected: Vec<&str> = entry.candidates.iter().map(|c| c.provider).collect();
            assert_eq!(providers, expected, "provider enum for {route}");
        }
    }

    #[test]
    fn route_tools_do_not_collide_with_static_tool_names() {
        let static_names: std::collections::BTreeSet<String> = tool_descriptors_evidence()
            .into_iter()
            .chain(tool_descriptors_client_and_daemon())
            .chain(tool_descriptors_widgets())
            .map(|tool| tool.name)
            .collect();
        for descriptor in tool_descriptors_routes() {
            assert!(
                !static_names.contains(&descriptor.name),
                "route tool {} collides with a static tool",
                descriptor.name
            );
        }
    }

    #[test]
    fn route_tools_are_present_in_the_mcp_tool_catalog_and_keep_it_consistent() {
        // mcp_tool_catalog() is derived from tool_descriptors(), so advertising
        // and the catalog stay in lockstep — the same invariant the tools/list
        // contract test asserts at the protocol boundary.
        let catalog: std::collections::BTreeSet<String> = mcp_tool_catalog().into_iter().collect();
        assert_eq!(
            catalog.len(),
            mcp_tool_catalog().len(),
            "no duplicate tool ids across the full catalog"
        );
        for descriptor in tool_descriptors_routes() {
            assert!(
                catalog.contains(&descriptor.name),
                "route tool {} absent from mcp_tool_catalog()",
                descriptor.name
            );
        }
        assert_eq!(
            tool_descriptors().len(),
            mcp_tool_catalog().len(),
            "descriptor list and catalog name list must be the same length"
        );
    }

    #[test]
    fn route_tools_env_gate_defaults_on_and_disables_on_off() {
        assert!(route_tools_enabled_from(None), "unset is enabled");
        assert!(
            route_tools_enabled_from(Some("on")),
            "any value but off enabled"
        );
        assert!(!route_tools_enabled_from(Some("off")));
        assert!(!route_tools_enabled_from(Some("OFF")), "case-insensitive");
    }

    #[test]
    fn route_fetch_data_envelope_carries_route_and_passthrough_params() {
        let route = "equity/price/historical";
        let mut arguments = Map::new();
        arguments.insert("symbol".to_string(), json!("AAPL"));
        arguments.insert("provider".to_string(), json!("fileset"));
        let envelope = route_fetch_data_envelope(route, Value::Object(arguments))
            .unwrap_or_else(|_| panic!("envelope should build for {route}"));
        match envelope.op {
            Op::FetchData {
                route: ref op_route,
                ref params,
            } => {
                assert_eq!(op_route, route);
                assert_eq!(params["symbol"], json!("AAPL"));
                // The provider arg flows through so the daemon's resolver pins it.
                assert_eq!(params["provider"], json!("fileset"));
            }
            ref other => panic!("expected FetchData op, got {other:?}"),
        }
    }

    #[test]
    fn route_tool_dispatch_unknown_provider_is_a_tool_error_not_protocol_error() {
        let mut server = McpServer::new();
        initialize(&mut server);
        // A provider that is not a candidate for the route is rejected before any
        // daemon submit, as a tool-level error (isError), never a protocol error.
        let response = decode(
            &server.handle_json_rpc_line(
                r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"tdw.route.equity.price.historical","arguments":{"symbol":"AAPL","provider":"not_a_candidate"}}}"#,
            )[0],
        );
        assert!(
            response["error"].is_null(),
            "unknown provider must not be a protocol error: {response}"
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|text| text.contains("not a candidate")),
            "unexpected error text: {response}"
        );
    }

    #[test]
    fn route_tool_dispatch_fails_closed_when_daemon_unavailable() {
        // Reserve then drop a port so the configured daemon endpoint refuses.
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
                r#"{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"tdw.route.equity.price.historical","arguments":{"symbol":"AAPL"}}}"#,
            )[0],
        );
        assert_eq!(response["id"], 22);
        // A missing daemon is a tool error, matching the daemon-tool posture.
        assert_eq!(response["result"]["isError"], true);
        let error_text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("daemon error text");
        assert!(error_text.contains(&format!("endpoint=tcp://{addr}")));
    }

    #[test]
    fn route_tool_dispatch_unknown_route_is_a_tool_error() {
        // A `tdw.route.*` id whose route is absent from the catalog is a tool
        // error, not the generic -32602 unknown-tool protocol error.
        let result = execute_route_tool(
            &DaemonToolRuntime::from_env(),
            "does/not/exist",
            &Map::new(),
        );
        match result {
            Err(ToolFailure::Execution(message)) => {
                assert!(message.contains("unknown catalog route"), "{message}");
            }
            other => panic!("expected execution error, got {:?}", other.is_ok()),
        }
    }
}
