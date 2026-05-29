#![forbid(unsafe_code)]

//! Env-gated MCP daemon integration.
//!
//! Default `cargo test` compiles this test and skips it when no live daemon
//! address is provided. Set the integration-specific
//! `TDW_MCP_DAEMON_INTEGRATION_ADDR` variable to make it hit a real daemon
//! endpoint through the public MCP JSON-RPC surface.

use std::time::Duration;

use serde_json::{Value, json};
use tdw_app_client::DaemonClientConfig;
use tdw_app_server::{DaemonEndpoint, DaemonTransport};
use tdw_mcp::{MCP_PROTOCOL_VERSION, McpServer};

fn integration_addr() -> Option<String> {
    std::env::var("TDW_MCP_DAEMON_INTEGRATION_ADDR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn integration_transport() -> DaemonTransport {
    let value =
        std::env::var("TDW_MCP_DAEMON_INTEGRATION_TRANSPORT").unwrap_or_else(|_| "tcp".to_string());
    match value.trim().to_ascii_lowercase().as_str() {
        "tcp" => DaemonTransport::Tcp,
        "uds" | "unix" => DaemonTransport::Uds,
        "http" | "http-sse" | "httpsse" => DaemonTransport::HttpSse,
        other => panic!("unknown TDW_MCP_DAEMON_INTEGRATION_TRANSPORT: {other}"),
    }
}

fn integration_timeout() -> Duration {
    std::env::var("TDW_MCP_DAEMON_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map_or_else(|| Duration::from_secs(3), Duration::from_millis)
}

fn decode(message: &str) -> Value {
    serde_json::from_str(message)
        .unwrap_or_else(|error| panic!("response should be JSON: {error}; {message}"))
}

fn initialize(server: &mut McpServer) {
    let messages = server.handle_json_rpc_line(&format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{MCP_PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"tdw-mcp-daemon-integration","version":"1.0.0"}}}}}}"#
    ));
    assert_eq!(messages.len(), 1);
    let response = decode(&messages[0]);
    assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
}

#[test]
fn env_gated_daemon_query_submit_runs_through_mcp_surface() {
    let Some(address) = integration_addr() else {
        eprintln!("TDW_MCP_DAEMON_INTEGRATION_ADDR not set; skipping MCP daemon integration test");
        return;
    };

    let transport = integration_transport();
    let mut server = McpServer::with_daemon_config(
        DaemonClientConfig::new(DaemonEndpoint {
            transport,
            address: address.clone(),
        })
        .with_timeout(integration_timeout()),
    );
    initialize(&mut server);

    let session_id = format!("session-mcp-daemon-integration-{}", std::process::id());
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "tdw.daemon.query.submit",
            "arguments": {
                "sql": "select 1",
                "session_id": session_id
            }
        }
    })
    .to_string();
    let messages = server.handle_json_rpc_line(&request);
    assert_eq!(messages.len(), 1);
    let response = decode(&messages[0]);

    assert_eq!(response["id"], 2);
    assert_eq!(
        response["result"]["isError"], false,
        "daemon-backed MCP tool should submit successfully when the env gate is set: {response}"
    );
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["tool"], "tdw.daemon.query.submit");
    assert_eq!(structured["daemon"]["address"], json!(address));
    assert_eq!(structured["extra"]["sql"], "select 1");

    let submitted_op_id = structured["submitted_op_id"]
        .as_str()
        .expect("submitted_op_id should be present");
    let terminal = &structured["terminal_event"];
    assert_eq!(terminal["op_id"], submitted_op_id);
    assert!(
        matches!(
            terminal["type"].as_str(),
            Some("completed") | Some("failed") | Some("cancelled")
        ),
        "terminal event should prove daemon execution reached a terminal state: {terminal}"
    );
}
