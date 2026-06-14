//! Integration tests for `tdw.partner.brief` — the proactive morning brief
//! (partner-system W3.6).
//!
//! Drives the tool over the real MCP JSON-RPC surface (`initialize` →
//! `tools/list` → `tools/call`) against an offline `PartnerCore`, proving the
//! e2e path: gathered signals → one ranked nudge stream. Also asserts descriptor
//! gating (the tool is listed only when a `PartnerCore` is attached) and the
//! ranking order from a fixed golden input.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_eval_runner::StubLanguageModel;
use tdw_mcp::McpServer;
use tdw_partner::{DataPlane, DataPlaneError, PartnerCore};

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
fn partner_brief_is_listed_only_when_core_attached() {
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let names = list_tool_names(&mut bare);
    assert!(
        !names.contains(&"tdw.partner.brief".to_string()),
        "tdw.partner.brief must be absent without a PartnerCore: {names:?}"
    );

    let mut server = McpServer::new().with_partner(partner_core());
    initialize(&mut server);
    let names = list_tool_names(&mut server);
    assert!(
        names.contains(&"tdw.partner.brief".to_string()),
        "tdw.partner.brief must be listed when a PartnerCore is attached: {names:?}"
    );
}

#[test]
fn partner_brief_ranks_a_golden_input_e2e() {
    let mut server = McpServer::new().with_partner(partner_core());
    initialize(&mut server);

    let response = call(
        &mut server,
        "tdw.partner.brief",
        &json!({
            "inputs": {
                "alerts": [
                    { "id": "a-1", "symbol": "AAPL", "headline": "AAPL crossed $200", "fired_at": "2026-06-14" }
                ],
                "signals": [
                    { "id": "q-3", "kind": "OpenQuestion", "severity": "Medium",
                      "headline": "open question", "kg_nodes": ["question:q-3"], "as_of": "2026-06-14" },
                    { "id": "t-7", "kind": "ThesisHealth", "severity": "High",
                      "headline": "thesis weakening", "kg_nodes": ["finding:t-7"], "as_of": "2026-06-13" }
                ]
            }
        }),
    );
    let structured = &response["result"]["structuredContent"];
    assert_eq!(
        structured["count"],
        json!(3),
        "all three signals ranked: {structured}"
    );
    let nudges = structured["nudges"].as_array().expect("nudges array");
    // Critical alert leads, then High thesis, then Medium question.
    assert_eq!(nudges[0]["headline"], json!("AAPL crossed $200"));
    assert_eq!(nudges[1]["headline"], json!("thesis weakening"));
    assert_eq!(nudges[2]["headline"], json!("open question"));
}

#[test]
fn partner_brief_without_core_is_a_tool_error_not_protocol_error() {
    let mut server = McpServer::new();
    initialize(&mut server);
    let response = call(&mut server, "tdw.partner.brief", &json!({}));
    assert!(
        response.get("error").is_none(),
        "missing partner surface must be a tool error, not a protocol error: {response}"
    );
    assert_eq!(
        response["result"]["isError"], true,
        "tool-level error flag set: {response}"
    );
}
