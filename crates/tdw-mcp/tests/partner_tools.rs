//! Integration tests for `tdw.partner.ask` — the Partner Core front door
//! (partner-system W2.6).
//!
//! Drives the tool over the real MCP JSON-RPC surface (`initialize` →
//! `tools/list` → `tools/call`) against an offline `PartnerCore` built on the
//! deterministic `StubLanguageModel`, proving the e2e path: ask → streamed
//! answer + citation, fully offline. Also asserts descriptor gating (the tool is
//! listed only when a `PartnerCore` is attached).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_eval_runner::StubLanguageModel;
use tdw_mcp::McpServer;
use tdw_partner::{DataPlane, DataPlaneError, PartnerCore};

/// A no-op data plane: the Workspace/MCP surfaces don't fetch server-side in this
/// offline e2e (the stub answers from context), so an honest fetch error is fine.
struct NoopPlane;

#[async_trait]
impl DataPlane for NoopPlane {
    async fn fetch(&self, route: &str, _params: Value) -> Result<Value, DataPlaneError> {
        Err(DataPlaneError::Fetch {
            route: route.to_string(),
            message: "no server-side data plane in this offline e2e".to_string(),
        })
    }
}

fn partner_core() -> Arc<PartnerCore> {
    Arc::new(PartnerCore::new(
        Arc::new(StubLanguageModel),
        Arc::new(NoopPlane),
    ))
}

fn initialize(server: &mut McpServer) {
    let messages = server.handle_json_rpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
    );
    assert_eq!(messages.len(), 1);
}

fn list_tool_names(server: &mut McpServer) -> Vec<String> {
    let listed: Value = serde_json::from_str(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    )
    .expect("tools/list json");
    listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str().map(ToString::to_string))
        .collect()
}

fn call(server: &mut McpServer, name: &str, arguments: &Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    let messages = server.handle_json_rpc_line(&request.to_string());
    serde_json::from_str(&messages[0])
        .unwrap_or_else(|e| panic!("response should be json: {e}; {}", messages[0]))
}

#[test]
fn partner_ask_is_listed_only_when_core_attached() {
    // Bare server: no partner core → tool absent.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let names = list_tool_names(&mut bare);
    assert!(
        !names.contains(&"tdw.partner.ask".to_string()),
        "tdw.partner.ask must be absent without a PartnerCore: {names:?}"
    );

    // Attached: tool listed.
    let mut server = McpServer::new().with_partner(partner_core());
    initialize(&mut server);
    let names = list_tool_names(&mut server);
    assert!(
        names.contains(&"tdw.partner.ask".to_string()),
        "tdw.partner.ask must be listed when a PartnerCore is attached: {names:?}"
    );
}

#[test]
fn partner_ask_streams_answer_and_citation_e2e() {
    let mut server = McpServer::new().with_partner(partner_core());
    initialize(&mut server);

    let response = call(
        &mut server,
        "tdw.partner.ask",
        &json!({ "utterance": "What is a P/E ratio?" }),
    );
    let structured = &response["result"]["structuredContent"];

    // The streamed answer echoes the question (the stub grounds on context).
    let answer = structured["answer"].as_str().unwrap_or_default();
    assert!(
        answer.contains("P/E ratio"),
        "ask streams a grounded answer: {structured}"
    );
    // The reasoning trail is present (the ordered PartnerEvent::Reasoning steps).
    assert!(
        structured["reasoning"]
            .as_array()
            .is_some_and(|steps| !steps.is_empty()),
        "ask carries the reasoning trail: {structured}"
    );
    // The citation block exists (null is allowed when nothing resolved, but the
    // key is always present so a caller can render it uniformly).
    assert!(
        structured.get("citation").is_some(),
        "ask carries a citation block: {structured}"
    );
}

#[test]
fn partner_ask_without_core_is_a_tool_error_not_protocol_error() {
    let mut server = McpServer::new();
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.partner.ask",
        &json!({ "utterance": "hi" }),
    );
    // A missing surface is a tool error (result with isError), never a protocol
    // error (top-level error) — the established MCP convention.
    assert!(
        response.get("error").is_none(),
        "missing partner surface must be a tool error, not a protocol error: {response}"
    );
    assert_eq!(
        response["result"]["isError"], true,
        "tool-level error flag set: {response}"
    );
}
