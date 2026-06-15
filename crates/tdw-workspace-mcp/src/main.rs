#![forbid(unsafe_code)]
//! The `tdw-workspace-mcp` binary: serve the Workspace control-plane MCP tools
//! over a blocking stdio JSON-RPC loop, mirroring how `tdw-mcp` serves.

fn main() {
    std::process::exit(tdw_workspace_mcp::run_stdio_json_rpc());
}
