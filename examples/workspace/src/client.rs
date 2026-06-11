//! A tiny offline HTTP/1.1 client for talking to the booted loopback servers.
//!
//! Just enough to issue a `GET` or a `POST` and read the full response — no
//! third-party HTTP dependency. The servers are local, single-shot, and
//! `Connection: close`, so a raw socket is the smallest thing that teaches the
//! request/response shape.

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// A parsed HTTP response: the status code, the raw header block, and the body.
#[derive(Clone, Debug)]
pub struct Response {
    /// The numeric HTTP status (e.g. `200`).
    pub status: u16,
    /// The raw header block (everything before the blank line).
    pub headers: String,
    /// The response body as a string.
    pub body: String,
}

impl Response {
    /// Parse the body as JSON, or [`Value::Null`] if it is not valid JSON.
    #[must_use]
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }

    /// The ordered SSE `event:` names in the body (for the agent streams).
    #[must_use]
    pub fn sse_event_names(&self) -> Vec<String> {
        self.body
            .lines()
            .filter_map(|line| line.strip_prefix("event: ").map(str::to_string))
            .collect()
    }

    /// The concatenated SSE `data:` payloads in the body.
    #[must_use]
    pub fn sse_data(&self) -> String {
        self.body
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect()
    }
}

/// Issue `GET <target>` (with any extra CRLF-terminated headers) and read the
/// full response.
///
/// # Errors
///
/// Returns an error on connect / write / read failure or on a malformed
/// response (no header/body separator).
pub async fn get(addr: SocketAddr, target: &str, extra_headers: &str) -> std::io::Result<Response> {
    let request = format!(
        "GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra_headers}\r\n"
    );
    send(addr, &request).await
}

/// Issue `POST <target>` with a JSON `body` (and any extra headers) and read the
/// full response.
///
/// # Errors
///
/// Returns an error on connect / write / read failure or on a malformed
/// response.
pub async fn post(
    addr: SocketAddr,
    target: &str,
    body: &str,
    extra_headers: &str,
) -> std::io::Result<Response> {
    let request = format!(
        "POST {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra_headers}\r\n{body}",
        body.len()
    );
    send(addr, &request).await
}

/// Write a fully-formed request to a fresh connection and parse the response.
async fn send(addr: SocketAddr, request: &str) -> std::io::Result<Response> {
    let mut conn = TcpStream::connect(addr).await?;
    conn.write_all(request.as_bytes()).await?;
    conn.flush().await?;
    let mut raw = Vec::new();
    // A small ceiling so a misbehaving server can never hang the example/test.
    tokio::time::timeout(Duration::from_secs(5), conn.read_to_end(&mut raw))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "response read timed out")
        })??;
    parse(&raw)
}

/// Parse raw response bytes into a [`Response`].
fn parse(raw: &[u8]) -> std::io::Result<Response> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "no header/body separator")
    })?;
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "no status code in response",
            )
        })?;
    Ok(Response {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    })
}
