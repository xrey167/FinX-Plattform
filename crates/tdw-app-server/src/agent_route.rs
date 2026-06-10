//! `OpenBB` Workspace **agent protocol** bridge route family
//! (feature = `agent-route`).
//!
//! Extends the [`super::workspace_route`] family (same hand-rolled HTTP/1.1
//! listener style, same CORS + optional `X-TDW-API-KEY` auth posture) with the
//! two endpoints `OpenBB` Workspace needs to drive a backend agent as a custom
//! **copilot**:
//!
//! | Method   | Path           | Action                                          |
//! |----------|----------------|-------------------------------------------------|
//! | `GET`    | `/agents.json` | The copilot discovery document (one agent)      |
//! | `POST`   | `/v1/query`    | A `QueryRequest`; responds with an SSE stream    |
//! | `OPTIONS`| (either)       | CORS preflight                                  |
//!
//! Like the rest of the Workspace family this is a plain `tokio::net::TcpStream`
//! surface (no axum/hyper). The agent loop itself — parse the `QueryRequest`,
//! map it to a prompt, drive the [`StreamingLanguageModel`], and turn the result
//! into the ordered copilot SSE events — lives in the pure
//! [`tdw_openbb_agent`] crate and is reached through the [`AgentBridgeHandler`]
//! trait seam (implemented by `tdw-service-api`), so this crate performs no LLM
//! I/O and never forms a dependency cycle.
//!
//! # SSE
//!
//! `POST /v1/query` answers with `Content-Type: text/event-stream` and writes
//! one frame per event (`event: <type>\ndata: <json>\n\n`), flushing after each,
//! exactly as [`super::transport_http`]'s `/events` stream does. On the
//! two-request widget-data leg the agent emits a `get_widget_data` frame and the
//! transport closes the stream; the frontend re-POSTs the folded conversation.
//!
//! Sources (public docs): the `agents.json` reference and the copilot SSE
//! protocol are from <https://docs.openbb.co/workspace/copilots>.

#![cfg(feature = "agent-route")]

use std::sync::Arc;

use tdw_openbb_agent::{Answer, QueryRequest, SseEvent, default_manifest};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::workspace_route::{
    RequestHead, WorkspaceConfig, cors_headers, key_matches, parse_request_head,
};

const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Maximum `POST /v1/query` body size (a conversation with widget descriptions
/// stays well under this; a larger body is rejected `413`).
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// The copilot discovery document path.
const AGENTS_JSON_PATH: &str = "/agents.json";

/// The copilot query path Workspace POSTs a [`QueryRequest`] to.
const QUERY_PATH: &str = "/v1/query";

/// The transport-decoupling seam over the agent loop, implemented by the crate
/// that owns the agent backend + language model (e.g. `tdw-service-api`) so this
/// crate need not depend on it.
///
/// The single method takes a parsed [`QueryRequest`] and returns the ordered
/// [`Answer`] (the SSE events to write plus whether the turn closed early to
/// await widget data). All LLM driving happens inside the implementation; the
/// transport only writes the frames.
#[async_trait::async_trait]
pub trait AgentBridgeHandler: Send + Sync {
    /// Run one copilot turn for `request`, returning the ordered events to
    /// stream back. Implementations drive the configured
    /// [`StreamingLanguageModel`](tdw_openbb_agent) through the pure
    /// [`tdw_openbb_agent::answer`] sequencer.
    async fn answer(&self, request: QueryRequest) -> Answer;
}

/// Spawn an HTTP/1.1 listener serving the copilot agent-protocol family.
///
/// `query_url` is the absolute URL the served `agents.json` advertises as the
/// agent's `endpoints.query` (so Workspace POSTs back to this same listener);
/// the caller composes it from the bind address. Returns when
/// `cancel.cancelled()`.
///
/// # Errors
///
/// Returns an `io::Error` if accepting a connection fails.
pub async fn serve_agent_http(
    listener: TcpListener,
    handler: Arc<dyn AgentBridgeHandler>,
    config: WorkspaceConfig,
    query_url: String,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let config = Arc::new(config);
    let query_url = Arc::new(query_url);
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                let h = Arc::clone(&handler);
                let c = Arc::clone(&config);
                let q = Arc::clone(&query_url);
                tokio::spawn(handle_agent_conn(stream, h, c, q));
            }
        }
    }
    Ok(())
}

/// Handle one agent-family connection.
async fn handle_agent_conn(
    mut stream: TcpStream,
    handler: Arc<dyn AgentBridgeHandler>,
    config: Arc<WorkspaceConfig>,
    query_url: Arc<String>,
) {
    let mut buf = vec![0u8; MAX_HEADER_BYTES];
    let mut filled = 0usize;
    let header_end = loop {
        if filled == buf.len() {
            return;
        }
        let n = match stream.read(&mut buf[filled..]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        filled += n;
        if let Some(pos) = buf[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
    };
    let Some(header_str) = std::str::from_utf8(&buf[..header_end]).ok() else {
        return;
    };
    let Some(head) = parse_request_head(header_str) else {
        return;
    };
    let cors = cors_headers(&config, head.origin.as_deref());

    if head.method == "OPTIONS" {
        write_response(&mut stream, 204, "No Content", &cors, b"").await;
        return;
    }

    // Optional shared-key auth (constant-time compare), fail closed.
    if let Some(expected) = config.api_key.as_deref()
        && !key_matches(expected, head.api_key.as_deref())
    {
        write_error(&mut stream, 401, "missing or invalid X-TDW-API-KEY", &cors).await;
        return;
    }

    match (head.method.as_str(), head.path.as_str()) {
        ("GET", AGENTS_JSON_PATH) => {
            let document = default_manifest(&query_url);
            write_json(&mut stream, &document, &cors).await;
        }
        ("POST", QUERY_PATH) => {
            serve_query(
                &mut stream,
                &handler,
                &head,
                &buf[..filled],
                header_end,
                &cors,
            )
            .await;
        }
        ("GET", QUERY_PATH) | ("POST", AGENTS_JSON_PATH) => {
            write_error(&mut stream, 405, "method not allowed", &cors).await;
        }
        _ => write_error(&mut stream, 404, "not an agent route", &cors).await,
    }
}

/// Read the `POST /v1/query` body, parse it as a [`QueryRequest`], drive the
/// handler, and stream the resulting events back as SSE.
async fn serve_query(
    stream: &mut TcpStream,
    handler: &Arc<dyn AgentBridgeHandler>,
    head: &RequestHead,
    head_buf: &[u8],
    header_end: usize,
    cors: &[(String, String)],
) {
    let Some(content_length) = head.content_length else {
        write_error(stream, 400, "missing Content-Length", cors).await;
        return;
    };
    if content_length > MAX_BODY_BYTES {
        write_error(stream, 413, "request body too large", cors).await;
        return;
    }
    // Bytes after the header terminator already in the buffer, plus the rest.
    let mut body = head_buf[(header_end + 4).min(head_buf.len())..].to_vec();
    while body.len() < content_length {
        let mut tmp = vec![0u8; content_length - body.len()];
        match stream.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => return,
        }
    }

    let request: QueryRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            write_error(stream, 400, &format!("invalid QueryRequest: {error}"), cors).await;
            return;
        }
    };

    let answer = handler.answer(request).await;
    write_sse(stream, &answer, cors).await;
}

/// Write the SSE response head (with CORS) and then one frame per event.
async fn write_sse(stream: &mut TcpStream, answer: &Answer, cors: &[(String, String)]) {
    let mut head = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n",
    );
    for (name, value) in cors {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    for event in &answer.events {
        let frame = SseEvent::to_sse_frame(event);
        if stream.write_all(frame.as_bytes()).await.is_err() {
            return;
        }
        let _ = stream.flush().await;
    }
    let _ = stream.flush().await;
}

/// Write a `200 OK` JSON response (with CORS headers).
async fn write_json(stream: &mut TcpStream, value: &serde_json::Value, cors: &[(String, String)]) {
    let Ok(body) = serde_json::to_vec(value) else {
        write_response(stream, 500, "Internal Server Error", cors, b"").await;
        return;
    };
    let mut headers = cors.to_vec();
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    write_response(stream, 200, "OK", &headers, &body).await;
}

/// Write an error response with a `{ "error": message }` body (with CORS).
async fn write_error(
    stream: &mut TcpStream,
    status: u16,
    message: &str,
    cors: &[(String, String)],
) {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    let body = serde_json::to_vec(&serde_json::json!({ "error": message }))
        .unwrap_or_else(|_| b"{}".to_vec());
    let mut headers = cors.to_vec();
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    write_response(stream, status, reason, &headers, &body).await;
}

/// Write a full HTTP/1.1 response with the given status, headers, and body.
async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_json_path_constants_are_documented_values() {
        assert_eq!(AGENTS_JSON_PATH, "/agents.json");
        assert_eq!(QUERY_PATH, "/v1/query");
    }

    #[test]
    fn manifest_advertises_the_passed_query_url() {
        let document = default_manifest("http://127.0.0.1:6900/v1/query");
        let value = serde_json::to_value(&document).expect("serializes");
        // The single agent entry points its query endpoint back at this URL.
        let endpoint = value
            .as_object()
            .and_then(|map| map.values().next())
            .and_then(|entry| entry.get("endpoints"))
            .and_then(|endpoints| endpoints.get("query"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(endpoint, Some("http://127.0.0.1:6900/v1/query"));
    }
}
