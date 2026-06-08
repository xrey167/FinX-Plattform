//! Minimal hand-rolled HTTP/1.1 + SSE transport (feature = "transport-http").
//!
//! Endpoints:
//!   POST /op   — submit an `OpEnvelope` JSON body; responds 202 Accepted.
//!   GET  /events — open SSE stream; emits `data: {event-json}\n\n` per `EventMsg`.
//!
//! No axum/hyper dependency; plain `tokio::net::TcpStream` IO only.

#![cfg(feature = "transport-http")]

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};
use tokio_util::sync::CancellationToken;

const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Per-read deadline on the inbound request (headers + body). A slow-loris
/// client that dribbles or stalls its request is dropped instead of holding
/// the connection (and its permit) open indefinitely (TT2). This bounds only
/// the request-read phase; the GET /events SSE phase that follows is
/// write-only, so long-lived event subscribers are unaffected.
const REQUEST_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Upper bound on concurrently-handled HTTP connections. Each accepted
/// connection holds a permit for its lifetime; once this many are live, further
/// connections are rejected (closed immediately) instead of spawning unbounded
/// handler tasks — a connection-exhaustion DoS guard (TT1).
const MAX_CONCURRENT_CONNECTIONS: usize = 256;

/// Spawn an HTTP/1.1 listener on `listener` that:
/// - `POST /op`     → deserialise body as `OpEnvelope`, submit, respond 202.
/// - `GET /events`  → SSE stream of serialised `EventMsg` lines.
/// - anything else  → 404.
///
/// Cancellation: returns when `cancel.cancelled()`.
///
/// # Errors
///
/// Returns an `io::Error` if accepting a connection or reading/writing a
/// socket fails.
pub async fn serve_http(
    listener: TcpListener,
    handle: crate::SubmissionHandle,
    mut events: tokio::sync::mpsc::UnboundedReceiver<tdw_protocol::EventMsg>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let (tx, _) = broadcast::channel::<String>(1024);

    let tx_for_pump = tx.clone();
    let cancel_for_pump = cancel.clone();
    let pump = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel_for_pump.cancelled() => break,
                maybe = events.recv() => match maybe {
                    Some(event) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            let _ = tx_for_pump.send(json);
                        }
                    }
                    None => break,
                }
            }
        }
    });

    let conn_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                // Reject (close) the connection when at capacity rather than
                // spawning an unbounded task. The permit is held by the handler
                // for the connection's lifetime and released when it ends.
                let Ok(permit) = Arc::clone(&conn_limit).try_acquire_owned() else {
                    drop(stream);
                    continue;
                };
                let conn_handle = handle.clone();
                let subscriber = tx.subscribe();
                let cancel_conn = cancel.clone();
                tokio::spawn(handle_http_conn(stream, conn_handle, subscriber, cancel_conn, permit));
            }
        }
    }

    pump.abort();
    Ok(())
}

async fn handle_http_conn(
    mut stream: TcpStream,
    handle: crate::SubmissionHandle,
    mut subscriber: broadcast::Receiver<String>,
    cancel: CancellationToken,
    // Held for the connection's lifetime; dropping it frees a slot in the
    // connection cap (TT1). Not otherwise used.
    _permit: OwnedSemaphorePermit,
) {
    // Read request headers into a buffer (up to MAX_HEADER_BYTES).
    let mut header_buf = vec![0u8; MAX_HEADER_BYTES];
    let mut filled = 0usize;

    // Read until we see \r\n\r\n or fill the buffer.
    let header_end = loop {
        if filled == header_buf.len() {
            // Buffer full — too large.
            let _ = stream
                .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
        let n = match tokio::time::timeout(
            REQUEST_READ_TIMEOUT,
            stream.read(&mut header_buf[filled..]),
        )
        .await
        {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return, // EOF, error, or read timed out
            Ok(Ok(n)) => n,
        };
        filled += n;
        if let Some(pos) = find_header_end(&header_buf[..filled]) {
            break pos;
        }
    };

    let header_section = &header_buf[..header_end];
    let body_already_read = &header_buf[header_end + 4..filled];

    // Parse request line.
    let Ok(header_str) = std::str::from_utf8(header_section) else {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    };

    let mut lines = header_str.lines();
    let Some(request_line) = lines.next() else {
        return;
    };

    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    match (method, path) {
        ("POST", "/op") => {
            // Parse Content-Length.
            let content_length = parse_content_length(lines);
            if content_length > 16 * 1024 * 1024 {
                let _ = stream
                    .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            }

            // Collect body.
            let mut body = body_already_read.to_vec();
            while body.len() < content_length {
                let mut tmp = vec![0u8; content_length - body.len()];
                match tokio::time::timeout(REQUEST_READ_TIMEOUT, stream.read(&mut tmp)).await {
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => body.extend_from_slice(&tmp[..n]),
                    Ok(Err(_)) | Err(_) => return,
                }
            }

            if let Ok(env) = serde_json::from_slice::<tdw_protocol::OpEnvelope>(&body) {
                let _ = handle.submit(env);
                let _ = stream
                    .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                    .await;
            } else {
                let _ = stream
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        }

        ("GET", "/events") => {
            // Send SSE headers.
            let sse_header = b"HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\n\
                Connection: keep-alive\r\n\r\n";
            if stream.write_all(sse_header).await.is_err() {
                return;
            }

            // Stream events until cancelled or write error.
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return,
                    res = subscriber.recv() => match res {
                        Ok(json) => {
                            let line = format!("data: {json}\n\n");
                            if stream.write_all(line.as_bytes()).await.is_err() {
                                return;
                            }
                            let _ = stream.flush().await;
                        }
                        Err(_) => return,
                    }
                }
            }
        }

        _ => {
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    }
}

/// Find the position of `\r\n\r\n` in a byte slice, returning the index of `\r`.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse `Content-Length: N` from header lines (case-insensitive key).
fn parse_content_length<'a>(lines: impl Iterator<Item = &'a str>) -> usize {
    for line in lines {
        let mut kv = line.splitn(2, ':');
        let key = kv.next().unwrap_or("").trim();
        let val = kv.next().unwrap_or("").trim();
        if key.eq_ignore_ascii_case("content-length") {
            return val.parse::<usize>().unwrap_or(0);
        }
    }
    0
}
