//! `tdw kg note <title>` — capture a first-class analyst finding via MCP
//! (knowledge-system K-X6).
//!
//! Submits a `tdw.kg.finding` JSON-RPC call to the running MCP server's
//! Streamable HTTP endpoint (default `http://127.0.0.1:8788/mcp`).  The
//! server must have a knowledge runtime with a graph engine and a bound user
//! id attached; otherwise the tool returns a tool-error response that this
//! command surfaces as a non-zero exit.
//!
//! All writing goes through the MCP server so the finding is indexed for
//! hybrid search by the same pipeline the daemon manages — the CLI is a thin
//! JSON-RPC client, not an in-process write path.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::CliError;

/// Default MCP Streamable HTTP address.
const DEFAULT_MCP_ADDR: &str = "127.0.0.1:8788";

/// Run the `kg note` subcommand.  `args` are the full process arguments.
///
/// # Errors
///
/// Returns a descriptive error on I/O failure, missing title, or a tool-error
/// response from the MCP server.
// Argument extraction + two MCP round-trips in one function — splitting would
// complicate error propagation without improving clarity.
#[allow(clippy::too_many_lines)]
pub fn run(args: &[String]) -> Result<(), CliError> {
    // Positional: title is the first free argument after `kg note`.
    let title = positional_title(args)
        .ok_or("usage: tdw kg note <title> [--body TEXT] [--url URL] [--tag TAG]... [--link ID:REL]...")?;

    let body = flag_value(args, "--body");
    let source_url = flag_value(args, "--url");
    let as_of = flag_value(args, "--as-of");
    let tags = flag_values(args, "--tag");

    let addr = flag_value(args, "--mcp-addr")
        .unwrap_or_else(|| DEFAULT_MCP_ADDR.to_string());

    // Build the JSON-RPC request.
    let mut tool_args = serde_json::json!({ "title": title });
    if let Some(b) = &body {
        tool_args["body"] = serde_json::json!(b);
    }
    if let Some(url) = &source_url {
        tool_args["source_url"] = serde_json::json!(url);
    }
    if let Some(date) = &as_of {
        tool_args["as_of"] = serde_json::json!(date);
    }
    if !tags.is_empty() {
        tool_args["tags"] = serde_json::json!(tags);
    }

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "tdw.kg.finding",
            "arguments": tool_args,
        }
    });

    // Submit `initialize` first (required by MCP protocol), then `tools/call`.
    let response = submit_mcp_sequence(&addr, &request)
        .map_err(|e| format!("MCP request to {addr} failed: {e}"))?;

    // Parse the response: look for `result.content[0].text` (MCP tool result).
    let tool_result = extract_tool_result(&response)?;

    // Check for `isError` in the tool result.
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
        return Err(format!("tdw.kg.finding error: {message}").into());
    }

    // Print the result content.
    let content_text = tool_result
        .get("content")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("text"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(no content)");

    // Try to pretty-print if the content is JSON.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content_text) {
        println!(
            "{}",
            serde_json::to_string_pretty(&parsed)
                .unwrap_or_else(|_| content_text.to_string())
        );
    } else {
        println!("{content_text}");
    }

    // Handle --link flags: each is `<id>:<rel>` and submits a `tdw.kg.link` call.
    let finding_id = serde_json::from_str::<serde_json::Value>(content_text)
        .ok()
        .and_then(|v| v.get("finding_id").and_then(serde_json::Value::as_str).map(ToOwned::to_owned));

    let links = flag_values(args, "--link");
    if !links.is_empty() {
        let finding_id = finding_id
            .ok_or("cannot resolve --link: finding id not returned by server")?;
        for link in &links {
            let (to_id, rel) = parse_link(link)?;
            let link_args = serde_json::json!({
                "from_finding_id": finding_id,
                "to": to_id,
                "rel": rel,
            });
            let link_request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "tdw.kg.link",
                    "arguments": link_args,
                }
            });
            let link_response = submit_mcp_call(&addr, &link_request)
                .map_err(|e| format!("MCP link request failed: {e}"))?;
            let link_result = extract_tool_result(&link_response)?;
            if link_result
                .get("isError")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                let message = link_result
                    .get("content")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("link error");
                return Err(format!("tdw.kg.link error: {message}").into());
            }
            let link_text = link_result
                .get("content")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("text"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("linked");
            println!("linked: {link_text}");
        }
    }

    Ok(())
}

/// Parse a `<to_id>:<rel>` link spec.  The `rel` is the last colon-separated
/// token; everything before it is the entity id (which may itself contain colons,
/// e.g. `instrument:AAPL`).
fn parse_link(spec: &str) -> Result<(&str, &str), CliError> {
    let colon = spec
        .rfind(':')
        .ok_or_else(|| format!("--link must be <id>:<rel>, got {spec:?}"))?;
    let to_id = &spec[..colon];
    let rel = &spec[colon + 1..];
    if to_id.is_empty() || rel.is_empty() {
        return Err(format!("--link must be <id>:<rel>, got {spec:?}").into());
    }
    Ok((to_id, rel))
}

/// Send the MCP `initialize` handshake followed by a single `tools/call` request
/// on a fresh TCP connection, returning the parsed JSON-RPC response for the call.
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
    // Send initialize.
    send_jsonrpc_line(&mut stream, &init)?;
    // Read the initialize response (discard — we only need it to proceed).
    let _init_resp = read_jsonrpc_line(&mut stream)?;
    // Send initialized notification (no response expected).
    send_jsonrpc_line(&mut stream, &initialized)?;
    // Send the actual call.
    send_jsonrpc_line(&mut stream, call)?;
    let response = read_jsonrpc_line(&mut stream)?;
    Ok(response)
}

/// Send a single `tools/call` on an already-initialized connection (reused for
/// `--link` calls on the same session).
fn submit_mcp_call(
    addr: &str,
    call: &serde_json::Value,
) -> Result<serde_json::Value, CliError> {
    // Each `--link` opens a fresh session — simpler than reusing the TCP stream
    // across async boundaries and safe for the very-low-volume CLI use case.
    submit_mcp_sequence(addr, call)
}

/// Open a blocking TCP connection with a 5-second timeout.
fn connect_tcp(addr: &str) -> Result<TcpStream, CliError> {
    let socket_addr: std::net::SocketAddr = addr
        .parse()
        .map_err(|e| format!("invalid MCP address {addr:?}: {e}"))?;
    TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5))
        .map_err(|e| format!("cannot connect to MCP server at {addr}: {e}").into())
}

/// Send one JSON-RPC message as a newline-terminated line.
fn send_jsonrpc_line(stream: &mut TcpStream, value: &serde_json::Value) -> Result<(), CliError> {
    let line = serde_json::to_string(value)
        .map_err(|e| format!("serialize JSON-RPC: {e}"))?;
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("write JSON-RPC: {e}"))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("write newline: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

/// Read one newline-terminated JSON-RPC line from the stream with a 5-second
/// read timeout.
fn read_jsonrpc_line(stream: &mut TcpStream) -> Result<serde_json::Value, CliError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("set read timeout: {e}"))?;
    let mut line = String::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let ch = byte[0] as char;
                if ch == '\n' {
                    break;
                }
                line.push(ch);
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut
                || e.kind() == std::io::ErrorKind::WouldBlock =>
            {
                break;
            }
            Err(e) => return Err(format!("read JSON-RPC: {e}").into()),
        }
    }
    serde_json::from_str(line.trim())
        .map_err(|e| format!("parse JSON-RPC response: {e} (got: {line:?})").into())
}

/// Pull the `result` object from a JSON-RPC response, or surface the `error`
/// message.
fn extract_tool_result(
    response: &serde_json::Value,
) -> Result<&serde_json::Value, CliError> {
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

/// The positional title: the first argument that does NOT start with `--` and
/// appears AFTER the `note` keyword in the arg list.
fn positional_title(args: &[String]) -> Option<String> {
    let note_pos = args.iter().position(|a| a == "note")?;
    args.get(note_pos + 1)
        .filter(|a| !a.starts_with('-'))
        .map(ToOwned::to_owned)
}

/// First occurrence of `--flag <value>`.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// All occurrences of `--flag <value>` (multi-value flag).
fn flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut idx = 0;
    while idx < args.len() {
        if args[idx] == flag
            && let Some(val) = args.get(idx + 1)
            && !val.starts_with('-')
        {
            values.push(val.trim().to_string());
            idx += 2;
            continue;
        }
        idx += 1;
    }
    values
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positional_title_parses_correctly() {
        let a = args(&["tdw", "kg", "note", "My finding title"]);
        assert_eq!(
            positional_title(&a),
            Some("My finding title".to_string())
        );
    }

    #[test]
    fn positional_title_missing_returns_none() {
        let a = args(&["tdw", "kg", "note", "--body", "some body"]);
        assert_eq!(positional_title(&a), None);
    }

    #[test]
    fn flag_value_returns_first_occurrence() {
        let a = args(&["tdw", "kg", "note", "title", "--body", "text"]);
        assert_eq!(flag_value(&a, "--body"), Some("text".to_string()));
        assert_eq!(flag_value(&a, "--url"), None);
    }

    #[test]
    fn flag_values_collects_all_occurrences() {
        let a = args(&[
            "tdw", "kg", "note", "title",
            "--tag", "sector:tech",
            "--tag", "asset:equity",
        ]);
        let tags = flag_values(&a, "--tag");
        assert_eq!(tags, vec!["sector:tech", "asset:equity"]);
    }

    #[test]
    fn parse_link_splits_on_last_colon() {
        // `instrument:AAPL` is the to_id, `supports` is the rel.
        let (to, rel) = parse_link("instrument:AAPL:supports").expect("valid spec");
        assert_eq!(to, "instrument:AAPL");
        assert_eq!(rel, "supports");
    }

    #[test]
    fn parse_link_rejects_missing_colon() {
        assert!(parse_link("no-colon-rel").is_err());
    }

    #[test]
    fn parse_link_rejects_empty_parts() {
        assert!(parse_link(":rel").is_err());
        assert!(parse_link("id:").is_err());
    }
}
