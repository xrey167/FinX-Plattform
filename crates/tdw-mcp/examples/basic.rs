//! Offline, no-network example for `tdw-mcp`.
//!
//! Drives an in-process `McpServer` through a JSON-RPC session — `initialize`,
//! then `tools/list`, then a `tools/call` of the deterministic offline
//! `tdw.providers.list` tool — by feeding JSON-RPC lines directly to
//! `handle_json_rpc_line`. No transport socket, no daemon, no network. (The
//! daemon-backed tools like `tdw.daemon.query.submit` are listed but not called
//! here, since they require a running daemon.)
//!
//! Run with: `cargo run -p tdw-mcp --example tdw_mcp_basic`

use tdw_mcp::{MCP_PROTOCOL_VERSION, McpServer};

fn main() {
    let mut server = McpServer::new();

    // 1. initialize — the handshake every MCP client sends first.
    let initialize = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{MCP_PROTOCOL_VERSION}","capabilities":{{}},"clientInfo":{{"name":"example","version":"1.0.0"}}}}}}"#
    );
    for line in server.handle_json_rpc_line(&initialize) {
        println!("initialize -> {line}");
    }
    assert!(server.is_initialized());

    // 2. tools/list — the catalog (built-in offline tools + daemon-backed tools).
    for line in
        server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
    {
        // The full catalog is verbose; just confirm a known tool is present.
        let present = line.contains("tdw.providers.list");
        println!("tools/list -> contains tdw.providers.list: {present}");
    }

    // 3. tools/call of an offline tool — no daemon required.
    let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.providers.list","arguments":{}}}"#;
    for line in server.handle_json_rpc_line(call) {
        let ok = line.contains("\"id\":3") && !line.contains("\"error\"");
        println!("tools/call tdw.providers.list -> success: {ok}");
    }
}
