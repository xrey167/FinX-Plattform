//! `tdw kg export` — portable research-trail export via MCP (knowledge-system K-X10).
//!
//! Submits a `tdw.kg.export` JSON-RPC call to the running MCP server's
//! Streamable HTTP / TCP endpoint (default `127.0.0.1:8788`). The server must
//! have a knowledge runtime with a graph engine attached; otherwise the tool
//! returns a tool-error response that this command surfaces as a non-zero exit.
//!
//! The trail is assembled server-side (read-only) from the graph and returned
//! as both structured JSON and human-readable Markdown. The CLI is a thin
//! JSON-RPC client — it never reads or writes the graph in-process.
//!
//! Usage:
//!   tdw kg export --thesis <id> [--as-of YYYY-MM-DD] [--out json|markdown|both]
//!   tdw kg export --finding <id> ...
//!   tdw kg export --entity  <id> ...
//!
//! The `generated_at` stamp is injected by THIS command (the only wall-clock
//! read happens here, in the client) so the library stays clock-free and the
//! artifact records who/when generated it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::CliError;

/// Default MCP TCP address.
const DEFAULT_MCP_ADDR: &str = "127.0.0.1:8788";

/// Run the `kg export` subcommand. `args` are the full process arguments.
///
/// # Errors
///
/// Returns a descriptive error on I/O failure, a missing/ambiguous root, or a
/// tool-error response from the MCP server.
pub fn run(args: &[String]) -> Result<(), CliError> {
    let thesis = flag_value(args, "--thesis");
    let finding = flag_value(args, "--finding");
    let entity = flag_value(args, "--entity");
    let roots = [thesis.as_ref(), finding.as_ref(), entity.as_ref()]
        .into_iter()
        .flatten()
        .count();
    if roots != 1 {
        return Err("usage: tdw kg export (--thesis|--finding|--entity) <id> \
                    [--as-of YYYY-MM-DD] [--out json|markdown|both] [--mcp-addr ADDR]"
            .into());
    }

    let as_of = flag_value(args, "--as-of");
    let out = flag_value(args, "--out").unwrap_or_else(|| "markdown".to_string());
    let addr = flag_value(args, "--mcp-addr").unwrap_or_else(|| DEFAULT_MCP_ADDR.to_string());

    // generated_at is injected HERE (the client) — the library never reads the
    // wall clock, so the artifact's stamp is the caller's, not the server's.
    let generated_at = current_utc_rfc3339();

    let mut tool_args = serde_json::json!({ "generated_at": generated_at });
    if let Some(id) = &thesis {
        tool_args["thesis_id"] = serde_json::json!(id);
    }
    if let Some(id) = &finding {
        tool_args["finding_id"] = serde_json::json!(id);
    }
    if let Some(id) = &entity {
        tool_args["entity_id"] = serde_json::json!(id);
    }
    if let Some(date) = &as_of {
        tool_args["as_of"] = serde_json::json!(date);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "tdw.kg.export", "arguments": tool_args },
    });

    let response = submit_mcp_sequence(&addr, &request)
        .map_err(|e| format!("MCP request to {addr} failed: {e}"))?;

    let tool_result = extract_tool_result(&response)?;

    if tool_result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        let message = tool_result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("tool error");
        return Err(format!("tdw.kg.export error: {message}").into());
    }

    // The structured content carries `json` and `markdown` artifacts.
    let structured = tool_result.get("structuredContent");
    let content_text = tool_result
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(serde_json::Value::as_str);

    // Prefer the structured artifact; fall back to the raw content text.
    let parsed = structured.cloned().or_else(|| {
        content_text.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
    });

    match (out.as_str(), parsed) {
        ("markdown", Some(v)) => {
            if let Some(md) = v.get("markdown").and_then(serde_json::Value::as_str) {
                println!("{md}");
            } else {
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
        }
        ("json", Some(v)) => {
            let artifact = v.get("json").cloned().unwrap_or(v);
            println!(
                "{}",
                serde_json::to_string_pretty(&artifact).unwrap_or_default()
            );
        }
        ("both", Some(v)) => {
            if let Some(md) = v.get("markdown").and_then(serde_json::Value::as_str) {
                println!("{md}");
            }
            println!("---");
            let artifact = v.get("json").cloned().unwrap_or_else(|| v.clone());
            println!(
                "{}",
                serde_json::to_string_pretty(&artifact).unwrap_or_default()
            );
        }
        (_, Some(v)) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        (_, None) => {
            println!("{}", content_text.unwrap_or("(no content)"));
        }
    }

    Ok(())
}

/// Format the current UTC instant as an RFC 3339 string. This is the ONLY
/// wall-clock read in the export path, and it lives in the client by design.
fn current_utc_rfc3339() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

// ── MCP transport (mirrors kg note) ────────────────────────────────────────────

fn submit_mcp_sequence(
    addr: &str,
    call: &serde_json::Value,
) -> Result<serde_json::Value, CliError> {
    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "clientInfo": { "name": "tdw-cli", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": {}
        }
    });
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    });

    let mut stream = connect_tcp(addr)?;
    send_jsonrpc_line(&mut stream, &init)?;
    let _init_resp = read_jsonrpc_line(&mut stream)?;
    send_jsonrpc_line(&mut stream, &initialized)?;
    send_jsonrpc_line(&mut stream, call)?;
    read_jsonrpc_line(&mut stream)
}

fn connect_tcp(addr: &str) -> Result<TcpStream, CliError> {
    // Resolve through `ToSocketAddrs` so hostnames (e.g. `localhost:8788`) work,
    // not just raw IPs. We still connect with an explicit timeout, so take the
    // first resolved socket address.
    use std::net::ToSocketAddrs;
    let socket_addr = addr
        .to_socket_addrs()
        .map_err(|e| format!("invalid MCP address {addr:?}: {e}"))?
        .next()
        .ok_or_else(|| format!("no addresses resolved for MCP address {addr:?}"))?;
    TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5))
        .map_err(|e| format!("cannot connect to MCP server at {addr}: {e}").into())
}

fn send_jsonrpc_line(stream: &mut TcpStream, value: &serde_json::Value) -> Result<(), CliError> {
    let line = serde_json::to_string(value).map_err(|e| format!("serialize JSON-RPC: {e}"))?;
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write JSON-RPC: {e}"))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("write newline: {e}"))?;
    stream.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

fn read_jsonrpc_line(stream: &mut TcpStream) -> Result<serde_json::Value, CliError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("set read timeout: {e}"))?;
    // Accumulate raw bytes and decode once at the end. A byte-by-byte
    // `as char` cast would corrupt any multi-byte UTF-8 sequence (common in
    // trail titles, snippets, and labels), so we never touch `char` mid-stream.
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                bytes.push(byte[0]);
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                break;
            }
            Err(e) => return Err(format!("read JSON-RPC: {e}").into()),
        }
    }
    let line = String::from_utf8(bytes)
        .map_err(|e| format!("invalid UTF-8 in JSON-RPC response: {e}"))?;
    serde_json::from_str(line.trim())
        .map_err(|e| format!("parse JSON-RPC response: {e} (got: {line:?})").into())
}

fn extract_tool_result(response: &serde_json::Value) -> Result<&serde_json::Value, CliError> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("JSON-RPC error");
        return Err(format!("JSON-RPC error: {message}").into());
    }
    response
        .get("result")
        .ok_or_else(|| "JSON-RPC response missing 'result' field".into())
}

// ── Argument helpers ──────────────────────────────────────────────────────────

/// First occurrence of `--flag <value>`. A following token that itself looks
/// like a flag (`-…`) is not treated as the value, so `--thesis --finding`
/// surfaces a clear "missing root" error rather than swallowing `--finding`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && !v.starts_with('-'))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn flag_value_returns_first_occurrence() {
        let a = args(&["tdw", "kg", "export", "--thesis", "finding:t1"]);
        assert_eq!(flag_value(&a, "--thesis"), Some("finding:t1".to_string()));
        assert_eq!(flag_value(&a, "--finding"), None);
    }

    #[test]
    fn current_utc_rfc3339_has_z_suffix() {
        let ts = current_utc_rfc3339();
        assert!(ts.ends_with('Z'), "got {ts}");
        assert_eq!(ts.len(), 20, "RFC3339 second precision: {ts}");
    }
}
