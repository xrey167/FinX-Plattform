//! `tdw kg status` (knowledge-system-2 K-E2).
//!
//! Queries the daemon's REST endpoint `GET /api/v1/knowledge/status` and
//! prints the result in a human-readable format. This is an online command:
//! it requires the daemon to be running and the REST surface to be enabled
//! (the daemon must be started with `TDW_REST_BIND` set). If the endpoint
//! returns 404 the knowledge runtime is not attached and the command exits
//! with a clear message.
//!
//! Daemon address convention: `--rest-addr <host:port>` (default
//! `127.0.0.1:8787`, matching the daemon's `TDW_REST_BIND` default).

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::CliError;

const DEFAULT_REST_ADDR: &str = "127.0.0.1:8787";
const STATUS_PATH: &str = "/api/v1/knowledge/status";
const CONNECT_TIMEOUT_SECS: u64 = 5;
const READ_TIMEOUT_SECS: u64 = 10;

/// Run `tdw kg status`. `args` are the full process arguments.
///
/// Prints all observability fields human-readably to stdout. Exits via a
/// descriptive `CliError` on any connectivity or parse failure.
///
/// # Errors
///
/// Returns a descriptive error when the daemon is unreachable, the REST
/// surface is not enabled, or the response cannot be parsed.
pub async fn run(args: &[String]) -> Result<(), CliError> {
    let addr_str = flag_value(args, "--rest-addr").unwrap_or_else(|| DEFAULT_REST_ADDR.to_string());
    let addr: SocketAddr = addr_str
        .parse()
        .map_err(|_| format!("invalid --rest-addr {addr_str:?}: expected host:port"))?;

    let body = http_get(addr, STATUS_PATH).await?;
    let status: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| format!("parse status response: {error}"))?;

    print_status(&status);
    Ok(())
}

/// Render the `KgStatus` JSON snapshot human-readably.
fn print_status(status: &serde_json::Value) {
    println!("=== Knowledge System Status ===");
    println!();

    // --- Vector / document channel ---
    println!(
        "Vector collection : {}",
        string_field(status, "vector_collection")
    );
    println!(
        "Embedder model    : {}",
        string_field(status, "embedder_model")
    );
    println!(
        "Document count    : (see note) {}",
        string_field(status, "document_count_note")
    );
    println!();

    // --- Taxonomy ---
    println!("Taxonomy kinds    : {}", status["taxonomy_kind_count"]);
    println!();

    // --- Graph backend health ---
    match status.get("graph_health") {
        Some(serde_json::Value::Null) | None => {
            println!("Graph backend     : not attached");
        }
        Some(health) => {
            let backend = string_field(health, "backend_name");
            let reachable = health["reachable"].as_bool().unwrap_or(false);
            let health_str = if reachable {
                "reachable"
            } else {
                "UNREACHABLE"
            };
            println!("Graph backend     : {backend} — {health_str}");
            if let Some(error) = health.get("error").and_then(|v| v.as_str()) {
                println!("  Error           : {error}");
            }
        }
    }
    println!();

    // --- Version triple ---
    if let Some(versions) = status.get("versions") {
        println!(
            "Embedder model    : {}",
            string_field(versions, "embedder_model")
        );
        println!("Rules version     : {}", opt_u64(versions, "rules_version"));
        println!("Infer version     : {}", opt_u64(versions, "infer_version"));
    }
    println!();

    // --- Proposals ---
    match status.get("proposals") {
        Some(serde_json::Value::Null) | None => {
            println!("Proposals         : queue not attached");
        }
        Some(proposals) => {
            println!(
                "Proposals         : {} validated, {} ready",
                proposals["validated"], proposals["ready"]
            );
            let op_auth = proposals["operator_authority"].as_bool().unwrap_or(false);
            println!("Operator authority: {}", if op_auth { "YES" } else { "no" });
        }
    }
    println!();

    // --- Language-model grade (autonomy-gap#5) ---
    println!(
        "Language model    : {}",
        string_field(status, "language_model_grade")
    );
}

/// Extract a string field or return `"<missing>"`.
fn string_field<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>")
}

/// Format an optional u64 field as a string or `"none"`.
fn opt_u64(value: &serde_json::Value, key: &str) -> String {
    match value.get(key) {
        Some(serde_json::Value::Null) | None => "none".to_string(),
        Some(v) => v.to_string(),
    }
}

/// Send a minimal HTTP/1.1 GET request and return the response body.
async fn http_get(addr: SocketAddr, path: &str) -> Result<Vec<u8>, CliError> {
    let mut stream = tokio::time::timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        TcpStream::connect(addr),
    )
    .await
    .map_err(|_| format!("connect timeout to {addr}"))?
    .map_err(|e| format!("connect to {addr} failed: {e}"))?;

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("write request: {e}"))?;
    stream.flush().await.map_err(|e| format!("flush: {e}"))?;

    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(READ_TIMEOUT_SECS), async {
        stream
            .read_to_end(&mut buf)
            .await
            .map_err(|e| format!("read response: {e}"))
    })
    .await
    .map_err(|_| format!("read timeout from {addr}"))??;

    // Split at the blank line separating headers from body.
    let body_start = buf
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|pos| pos + 4)
        .ok_or_else(|| "response has no header/body separator".to_string())?;

    // Check the status line — surface a clear message for 404.
    let header = std::str::from_utf8(&buf[..body_start])
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");
    if header.contains("404") {
        return Err(
            "knowledge runtime not attached on the daemon — start the daemon with a \
             KnowledgeRuntime and TDW_REST_BIND set"
                .into(),
        );
    }
    if !header.contains("200") {
        return Err(format!("unexpected HTTP response: {header}").into());
    }

    Ok(buf[body_start..].to_vec())
}

/// `--flag value` lookup (mirrors `reindex.rs`).
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn print_status_does_not_panic_on_minimal_snapshot() {
        let snapshot = json!({
            "vector_collection": "tdw_knowledge__local_hash_8",
            "embedder_model": "local-hash-8",
            "document_count_note": "VectorEngine has no count()",
            "taxonomy_kind_count": 51,
            "graph_health": null,
            "versions": {
                "embedder_model": "local-hash-8",
                "rules_version": null,
                "infer_version": null
            },
            "proposals": null,
            "language_model_grade": "stub — eval feedback and auto-materialization disabled"
        });
        // Should not panic; output goes to stdout and is not asserted here.
        print_status(&snapshot);
    }

    #[test]
    fn flag_value_parses_and_rejects_empty() {
        let args: Vec<String> = ["tdw", "kg", "status", "--rest-addr", "127.0.0.1:8787"]
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            flag_value(&args, "--rest-addr").as_deref(),
            Some("127.0.0.1:8787")
        );
        assert_eq!(flag_value(&args, "--missing"), None);
    }
}
