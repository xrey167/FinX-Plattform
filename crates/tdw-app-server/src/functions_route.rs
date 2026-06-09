//! HTTP control-plane route for the functions registry.
//!
//! Exposed only when the `functions-route` feature is enabled (which implies
//! `transport-http`).  All three endpoints share the same hand-rolled
//! HTTP/1.1 pattern used by [`super::transport_http`]:
//!
//! | Method | Path                   | Action                         |
//! |--------|------------------------|--------------------------------|
//! | `GET`  | `/functions`           | Enumerate registered functions |
//! | `POST` | `/functions/invoke`    | Invoke a function by id        |
//! | `PUT`  | `/functions/resume`    | Resume a partially-done run    |
//!
//! # Transport-layer decoupling
//!
//! To avoid a package-level dependency cycle (`tdw-app-server` →
//! `tdw-functions` → `tdw-cron` → `tdw-worker` → `tdw-app-client` →
//! `tdw-app-server`), the route does **not** import `tdw-functions` directly.
//! Instead callers implement the [`FunctionsHandler`] trait on their own
//! wrapper type and pass an `Arc<dyn FunctionsHandler>` to
//! [`handle_functions_conn`] / [`serve_functions_http`].
//!
//! A blanket implementation for `tdw_functions::FunctionRegistry` is provided
//! by the *caller's* crate (e.g. `tdw-service-api`), which has no cycle with
//! either `tdw-app-server` or `tdw-functions`.
//!
//! # HMAC request-signature verification
//!
//! When [`SigningConfig::signing_secret`] is `Some`, every request **must**
//! carry the header:
//!
//! ```text
//! x-tdw-signature: sha256=<hex(HMAC-SHA256(body, secret))>
//! ```
//!
//! where *body* is the raw request body bytes (empty for `GET /functions`).
//! Verification uses `hmac::Mac::verify_slice`, which is constant-time.
//! A missing or invalid signature yields `401 Unauthorized`.
//! When the secret is `None` no verification is performed — presence-only
//! boundary; non-cryptographic deployments are an explicit operator choice.
//!
//! # run\_id generation
//!
//! HTTP-originated invocations derive the run id as
//! `"{app_id}-{function_id}-{now_ms}"`.  The caller supplies `now_ms`
//! (milliseconds since the Unix epoch) so the handler is deterministic and
//! testable without a real clock.
//!
//! # Public-path handling
//!
//! `tdw-service-api` has no public-path allow-list concept.  The
//! `/functions` route is HMAC-authenticated rather than session-authenticated;
//! HMAC *is* the auth boundary for this route.  No additional registration in
//! `tdw-service-api` is required or performed.

#![cfg(feature = "functions-route")]

use std::sync::Arc;

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum body size accepted by the invoke / resume endpoints.
const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MiB

/// Header name that carries the HMAC signature.
const SIGNATURE_HEADER: &str = "x-tdw-signature";

/// Expected prefix on the signature header value.
const SIGNATURE_PREFIX: &str = "sha256=";

// ---------------------------------------------------------------------------
// Public trait — caller implements this to inject the registry
// ---------------------------------------------------------------------------

/// A single function entry returned by [`FunctionsHandler::enumerate`].
#[derive(Clone, Debug)]
pub struct FunctionEntry {
    /// Stable, unique identifier for the function.
    pub id: String,
    /// Triggers serialised as a JSON value (callers serialize their native
    /// [`Trigger`] vec before handing it in).
    ///
    /// [`Trigger`]: https://docs.rs/tdw-functions/latest/tdw_functions/enum.Trigger.html
    pub triggers_json: serde_json::Value,
}

/// Error variants returned by [`FunctionsHandler`] operations.
#[derive(Debug)]
pub enum HandlerError {
    /// No function with the given id is registered.
    UnknownFunction(String),
    /// No run record exists for the given run id.
    UnknownRun(String),
    /// Any other error (step failure, store error, …).
    Other(String),
}

/// Abstract interface over a function registry, used by the HTTP route
/// handlers.  Implement this on a newtype wrapper around
/// `tdw_functions::FunctionRegistry` in the crate that wires everything
/// together (e.g. `tdw-service-api`).
///
/// All methods are `async` and take `&self` so implementations can hold an
/// `Arc`-shared registry.
#[async_trait::async_trait]
pub trait FunctionsHandler: Send + Sync {
    /// Return all registered functions as `(id, triggers_json)` pairs,
    /// sorted in deterministic order.
    fn enumerate(&self) -> Vec<FunctionEntry>;

    /// Invoke the function identified by `function_id` with `payload`.
    ///
    /// `run_id` and `now_ms` are caller-supplied (the route generates them).
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError::UnknownFunction`] if the id is not registered.
    async fn invoke(
        &self,
        function_id: &str,
        payload: serde_json::Value,
        run_id: String,
        now_ms: i64,
    ) -> Result<serde_json::Value, HandlerError>;

    /// Resume a partially-completed run identified by `run_id`.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError::UnknownRun`] if no run record exists.
    async fn resume(&self, run_id: &str, now_ms: i64) -> Result<serde_json::Value, HandlerError>;
}

// ---------------------------------------------------------------------------
// Signing config (passed independently of the handler trait)
// ---------------------------------------------------------------------------

/// HMAC signing configuration, mirroring the fields from
/// `tdw_functions::RuntimeConfig` that are relevant to request verification.
#[derive(Clone, Debug, Default)]
pub struct SigningConfig {
    /// Application id, used to namespace generated `run_id` values.
    pub app_id: String,
    /// Optional HMAC-SHA256 signing secret.  When `None`, all requests pass
    /// without signature verification.
    pub signing_secret: Option<String>,
}

// ---------------------------------------------------------------------------
// Public entry point called from transport_http
// ---------------------------------------------------------------------------

/// Handle a single connection that matched the `/functions` path prefix.
///
/// Reads the request line + headers, dispatches to the appropriate handler,
/// and writes the HTTP response.  Mirrors
/// [`super::transport_http::handle_http_conn`] in structure so the two stay
/// easy to compare.
pub(super) async fn handle_functions_conn(
    mut stream: TcpStream,
    handler: Arc<dyn FunctionsHandler>,
    config: Arc<SigningConfig>,
    now_ms: i64,
) {
    const MAX_HEADER_BYTES: usize = 8 * 1024;

    let mut header_buf = vec![0u8; MAX_HEADER_BYTES];
    let mut filled = 0usize;

    // Read until `\r\n\r\n` or buffer full.
    let header_end = loop {
        if filled == header_buf.len() {
            let _ = stream
                .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
        let n = match stream.read(&mut header_buf[filled..]).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        filled += n;
        if let Some(pos) = header_buf[..filled]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            break pos;
        }
    };

    let header_section = &header_buf[..header_end];
    let body_already_read = header_buf[header_end + 4..filled].to_vec();

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

    // Collect remaining header lines into a Vec so we can scan them twice
    // (once for Content-Length, once for the signature).
    let header_lines: Vec<&str> = lines.collect();

    // Dispatch based on (method, path).
    match (method, path) {
        ("GET", "/functions") => {
            // Body is empty for GET; signature covers the empty byte slice.
            if verify_signature(&config, b"", &header_lines).is_err() {
                write_401(&mut stream).await;
                return;
            }
            handle_enumerate(&mut stream, &*handler).await;
        }

        ("POST", "/functions/invoke") => {
            let content_length = parse_content_length(header_lines.iter().copied());
            if content_length > MAX_BODY_BYTES {
                let _ = stream
                    .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            }
            let body = read_body(&mut stream, body_already_read, content_length).await;
            let Some(body) = body else { return };

            if verify_signature(&config, &body, &header_lines).is_err() {
                write_401(&mut stream).await;
                return;
            }
            handle_invoke(&mut stream, &*handler, &config, &body, now_ms).await;
        }

        ("PUT", "/functions/resume") => {
            let content_length = parse_content_length(header_lines.iter().copied());
            if content_length > MAX_BODY_BYTES {
                let _ = stream
                    .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                    .await;
                return;
            }
            let body = read_body(&mut stream, body_already_read, content_length).await;
            let Some(body) = body else { return };

            if verify_signature(&config, &body, &header_lines).is_err() {
                write_401(&mut stream).await;
                return;
            }
            handle_resume(&mut stream, &*handler, &body, now_ms).await;
        }

        _ => {
            // Reached only if transport_http routes here incorrectly; guard.
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Handler: GET /functions → enumerate
// ---------------------------------------------------------------------------

async fn handle_enumerate(stream: &mut TcpStream, handler: &dyn FunctionsHandler) {
    let entries = handler.enumerate();
    // Each entry is `{ "id": "...", "triggers": [...] }`.
    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id,
                "triggers": entry.triggers_json,
            })
        })
        .collect();
    write_json_200(stream, &serde_json::Value::Array(items)).await;
}

// ---------------------------------------------------------------------------
// Handler: POST /functions/invoke
// ---------------------------------------------------------------------------

/// Request body shape for `POST /functions/invoke`.
#[derive(serde::Deserialize)]
struct InvokeRequest {
    function_id: String,
    payload: serde_json::Value,
}

async fn handle_invoke(
    stream: &mut TcpStream,
    handler: &dyn FunctionsHandler,
    config: &SigningConfig,
    body: &[u8],
    now_ms: i64,
) {
    let req = match serde_json::from_slice::<InvokeRequest>(body) {
        Ok(req) => req,
        Err(_) => {
            write_error(stream, 400, "invalid JSON body").await;
            return;
        }
    };

    // Derive run_id: "{app_id}-{function_id}-{now_ms}"
    let run_id = format!("{}-{}-{}", config.app_id, req.function_id, now_ms);

    match handler
        .invoke(&req.function_id, req.payload, run_id.clone(), now_ms)
        .await
    {
        Ok(result) => {
            write_json_200(
                stream,
                &serde_json::json!({
                    "run_id": run_id,
                    "result": result,
                }),
            )
            .await;
        }
        Err(HandlerError::UnknownFunction(_)) => {
            write_error(stream, 404, "unknown function").await;
        }
        Err(err) => {
            write_error(stream, 500, &format!("{err:?}")).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Handler: PUT /functions/resume
// ---------------------------------------------------------------------------

/// Request body shape for `PUT /functions/resume`.
#[derive(serde::Deserialize)]
struct ResumeRequest {
    run_id: String,
}

async fn handle_resume(
    stream: &mut TcpStream,
    handler: &dyn FunctionsHandler,
    body: &[u8],
    now_ms: i64,
) {
    let req = match serde_json::from_slice::<ResumeRequest>(body) {
        Ok(req) => req,
        Err(_) => {
            write_error(stream, 400, "invalid JSON body").await;
            return;
        }
    };

    match handler.resume(&req.run_id, now_ms).await {
        Ok(result) => {
            write_json_200(
                stream,
                &serde_json::json!({
                    "run_id": req.run_id,
                    "result": result,
                }),
            )
            .await;
        }
        Err(HandlerError::UnknownRun(_)) => {
            write_error(stream, 404, "unknown run").await;
        }
        Err(err) => {
            write_error(stream, 500, &format!("{err:?}")).await;
        }
    }
}

// ---------------------------------------------------------------------------
// HMAC signature verification
// ---------------------------------------------------------------------------

/// Verify `x-tdw-signature: sha256=<hex>` against `body`.
///
/// Returns `Ok(())` when:
/// - `config.signing_secret` is `None` (verification disabled), OR
/// - the header is present and the HMAC-SHA256 matches.
///
/// Returns `Err(())` when:
/// - the secret is configured but the header is absent, OR
/// - the header value does not have the `sha256=` prefix, OR
/// - the constant-time comparison fails.
pub(crate) fn verify_signature(
    config: &SigningConfig,
    body: &[u8],
    header_lines: &[&str],
) -> Result<(), ()> {
    let Some(secret) = config.signing_secret.as_deref() else {
        // No secret configured — pass all requests without checking.
        return Ok(());
    };

    // Find the x-tdw-signature header (case-insensitive key scan).
    let sig_value = header_lines
        .iter()
        .find_map(|line| {
            let mut kv = line.splitn(2, ':');
            let key = kv.next()?.trim();
            let val = kv.next()?.trim();
            if key.eq_ignore_ascii_case(SIGNATURE_HEADER) {
                Some(val)
            } else {
                None
            }
        })
        .ok_or(())?; // header absent → reject

    let hex_digest = sig_value.strip_prefix(SIGNATURE_PREFIX).ok_or(())?; // missing "sha256=" prefix → reject

    let expected_bytes = hex_decode(hex_digest).ok_or(())?;

    // Constant-time comparison via hmac::Mac::verify_slice.
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| ())?;
    mac.update(body);
    mac.verify_slice(&expected_bytes).map_err(|_| ())
}

/// Compute the HMAC-SHA256 of `body` with `secret` and return the lowercase
/// hex string suitable for the `x-tdw-signature` header.
///
/// Exposed for tests and callers that need to build valid signed requests.
///
/// # Errors
///
/// Returns `Err` only if the key length is rejected by the HMAC constructor
/// (in practice this never happens: HMAC accepts any key length).
pub fn sign_body(secret: &str, body: &[u8]) -> Result<String, String> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|error| error.to_string())?;
    mac.update(body);
    let result = mac.finalize().into_bytes();
    Ok(hex_encode(&result))
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

/// Read the full request body, combining `already_read` with further socket
/// reads until `content_length` bytes have been collected.  Returns `None` on
/// I/O error (caller should drop the connection).
async fn read_body(
    stream: &mut TcpStream,
    already_read: Vec<u8>,
    content_length: usize,
) -> Option<Vec<u8>> {
    let mut body = already_read;
    while body.len() < content_length {
        let mut tmp = vec![0u8; content_length - body.len()];
        match stream.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
            Err(_) => return None,
        }
    }
    Some(body)
}

/// Write a `200 OK` response with a JSON body.
async fn write_json_200(stream: &mut TcpStream, value: &serde_json::Value) {
    let body = match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) => {
            let _ = stream
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
}

/// Write a `{status} ...` response with `{ "error": message }` body.
async fn write_error(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    let body = serde_json::json!({ "error": message });
    let body_bytes = match serde_json::to_vec(&body) {
        Ok(bytes) => bytes,
        Err(_) => b"{}".to_vec(),
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body_bytes.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body_bytes).await;
}

/// Write a bare `401 Unauthorized` with `{ "error": "invalid signature" }`.
async fn write_401(stream: &mut TcpStream) {
    write_error(stream, 401, "invalid signature").await;
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

// ---------------------------------------------------------------------------
// Minimal hex helpers (no external dep)
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|chunk| {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            Some((hi << 4) | lo)
        })
        .collect()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::{
        FunctionEntry, FunctionsHandler, HandlerError, SigningConfig, handle_functions_conn,
        sign_body, verify_signature,
    };

    const NOW: i64 = 1_700_000_000_000;

    // -------------------------------------------------------------------------
    // Stub FunctionsHandler for tests (no tdw-functions dep in this crate)
    // -------------------------------------------------------------------------

    struct StubHandler {
        entries: Vec<(String, serde_json::Value)>,
    }

    impl StubHandler {
        fn new() -> Self {
            Self {
                entries: vec![("fn-a".to_string(), json!([{"Event": "price.updated"}]))],
            }
        }
    }

    #[async_trait::async_trait]
    impl FunctionsHandler for StubHandler {
        fn enumerate(&self) -> Vec<FunctionEntry> {
            self.entries
                .iter()
                .map(|(id, triggers)| FunctionEntry {
                    id: id.clone(),
                    triggers_json: triggers.clone(),
                })
                .collect()
        }

        async fn invoke(
            &self,
            function_id: &str,
            _payload: serde_json::Value,
            run_id: String,
            _now_ms: i64,
        ) -> Result<serde_json::Value, HandlerError> {
            if function_id == "fn-a" {
                Ok(json!({"run_id": run_id, "status": "ok"}))
            } else {
                Err(HandlerError::UnknownFunction(function_id.to_string()))
            }
        }

        async fn resume(
            &self,
            run_id: &str,
            _now_ms: i64,
        ) -> Result<serde_json::Value, HandlerError> {
            if run_id.starts_with("known-") {
                Ok(json!("resumed"))
            } else {
                Err(HandlerError::UnknownRun(run_id.to_string()))
            }
        }
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn no_secret_config() -> SigningConfig {
        SigningConfig {
            app_id: "test".to_string(),
            signing_secret: None,
        }
    }

    fn with_secret_config(secret: &str) -> SigningConfig {
        SigningConfig {
            app_id: "test".to_string(),
            signing_secret: Some(secret.to_string()),
        }
    }

    /// Spin up a functions-route server on a random port, run `f(addr)`.
    async fn with_server<F, Fut>(config: SigningConfig, f: F)
    where
        F: FnOnce(std::net::SocketAddr) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let handler: Arc<dyn FunctionsHandler> = Arc::new(StubHandler::new());
        let cfg = Arc::new(config);

        let server_handler = Arc::clone(&handler);
        let server_cfg = Arc::clone(&cfg);
        let server = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let h = Arc::clone(&server_handler);
                let c = Arc::clone(&server_cfg);
                tokio::spawn(handle_functions_conn(stream, h, c, NOW));
            }
        });

        f(addr).await;
        server.abort();
    }

    /// Send a raw HTTP request and return the full response bytes.
    async fn raw_request(addr: std::net::SocketAddr, request: &[u8]) -> Vec<u8> {
        let mut conn = TcpStream::connect(addr).await.expect("connect");
        conn.write_all(request).await.expect("write request");
        conn.flush().await.expect("flush");

        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        let result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            loop {
                let n = conn.read(&mut buf).await.expect("read");
                if n == 0 {
                    break;
                }
                resp.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&resp);
                if text.contains("\r\n\r\n") {
                    if let Some(cl) = extract_content_length(&resp) {
                        if let Some(header_end) = resp.windows(4).position(|w| w == b"\r\n\r\n") {
                            let body_start = header_end + 4;
                            if resp.len() >= body_start + cl {
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }
            }
        })
        .await;
        result.ok();
        resp
    }

    fn extract_content_length(resp: &[u8]) -> Option<usize> {
        let text = std::str::from_utf8(resp).ok()?;
        for line in text.lines() {
            let mut kv = line.splitn(2, ':');
            let key = kv.next()?.trim();
            let val = kv.next()?.trim();
            if key.eq_ignore_ascii_case("content-length") {
                return val.parse().ok();
            }
        }
        None
    }

    fn response_status(resp: &[u8]) -> u16 {
        let text = std::str::from_utf8(resp).unwrap_or("");
        let status_str = text.split(' ').nth(1).unwrap_or("0");
        status_str.parse().unwrap_or(0)
    }

    fn response_body_json(resp: &[u8]) -> serde_json::Value {
        let text = std::str::from_utf8(resp).unwrap_or("");
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .unwrap_or("");
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
    }

    // -------------------------------------------------------------------------
    // Test 1: GET /functions enumerates registered functions
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn enumerate_returns_registered_functions() {
        with_server(no_secret_config(), |addr| async move {
            let request = b"GET /functions HTTP/1.1\r\nHost: localhost\r\n\r\n";
            let resp = raw_request(addr, request).await;
            assert_eq!(
                response_status(&resp),
                200,
                "response: {}",
                String::from_utf8_lossy(&resp)
            );
            let body = response_body_json(&resp);
            let arr = body.as_array().expect("array");
            assert_eq!(arr.len(), 1);
            assert_eq!(arr[0]["id"], "fn-a");
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 2: POST /functions/invoke happy path returns run result
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn invoke_happy_path_returns_run_result() {
        with_server(no_secret_config(), |addr| async move {
            let body = serde_json::to_vec(&json!({
                "function_id": "fn-a",
                "payload": {}
            }))
            .expect("serialize");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(
                response_status(&resp),
                200,
                "response: {}",
                String::from_utf8_lossy(&resp)
            );
            let value = response_body_json(&resp);
            assert!(value["run_id"].as_str().is_some(), "run_id present");
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 3: invoke unknown function → 404
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn invoke_unknown_function_returns_404() {
        with_server(no_secret_config(), |addr| async move {
            let body = serde_json::to_vec(&json!({
                "function_id": "does-not-exist",
                "payload": {}
            }))
            .expect("serialize");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(response_status(&resp), 404);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 4: resume unknown run → 404
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn resume_unknown_run_returns_404() {
        with_server(no_secret_config(), |addr| async move {
            let body = serde_json::to_vec(&json!({ "run_id": "ghost-run" })).expect("serialize");
            let request = format!(
                "PUT /functions/resume HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(response_status(&resp), 404);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 5: bad JSON body → 400
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn bad_json_body_returns_400() {
        with_server(no_secret_config(), |addr| async move {
            let body = b"not valid json";
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(body);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(response_status(&resp), 400);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 6: HMAC — valid signature accepted
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn valid_signature_accepted() {
        let secret = "my-secret";
        with_server(with_secret_config(secret), |addr| async move {
            let body_json = serde_json::to_vec(&json!({
                "function_id": "fn-a",
                "payload": {}
            }))
            .expect("serialize");
            let sig = sign_body(secret, &body_json).expect("sign");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nx-tdw-signature: sha256={}\r\n\r\n",
                body_json.len(),
                sig
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body_json);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(
                response_status(&resp),
                200,
                "response: {}",
                String::from_utf8_lossy(&resp)
            );
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 7: HMAC — wrong signature → 401
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn wrong_signature_returns_401() {
        let secret = "my-secret";
        with_server(with_secret_config(secret), |addr| async move {
            let body_json = serde_json::to_vec(&json!({
                "function_id": "fn-a",
                "payload": {}
            }))
            .expect("serialize");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nx-tdw-signature: sha256=deadbeef00000000000000000000000000000000000000000000000000000000\r\n\r\n",
                body_json.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body_json);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(response_status(&resp), 401);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 8: HMAC — missing header when secret configured → 401
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn missing_signature_header_returns_401() {
        let secret = "my-secret";
        with_server(with_secret_config(secret), |addr| async move {
            let body_json = serde_json::to_vec(&json!({
                "function_id": "fn-a",
                "payload": {}
            }))
            .expect("serialize");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body_json.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body_json);

            let resp = raw_request(addr, &req_bytes).await;
            assert_eq!(response_status(&resp), 401);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 9: HMAC — no secret configured → requests pass unsigned
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn no_secret_requests_pass_unsigned() {
        with_server(no_secret_config(), |addr| async move {
            let request = b"GET /functions HTTP/1.1\r\nHost: localhost\r\n\r\n";
            let resp = raw_request(addr, request).await;
            assert_eq!(response_status(&resp), 200);
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 10: GET /functions signs empty body for HMAC check
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_signs_empty_body() {
        let secret = "get-secret";
        with_server(with_secret_config(secret), |addr| async move {
            let sig = sign_body(secret, b"").expect("sign empty");
            let request = format!(
                "GET /functions HTTP/1.1\r\nHost: localhost\r\nx-tdw-signature: sha256={sig}\r\n\r\n"
            );
            let resp = raw_request(addr, request.as_bytes()).await;
            assert_eq!(
                response_status(&resp),
                200,
                "GET with correct empty-body sig must be 200"
            );
        })
        .await;
    }

    // -------------------------------------------------------------------------
    // Test 11: verify_signature returns Ok when secret is None (allow-list)
    // -------------------------------------------------------------------------

    #[test]
    fn verify_signature_no_secret_always_ok() {
        let config = no_secret_config();
        assert!(verify_signature(&config, b"anything", &[]).is_ok());
        assert!(verify_signature(&config, b"", &[]).is_ok());
    }

    // -------------------------------------------------------------------------
    // Test 12: verify_signature rejects missing header when secret set
    // -------------------------------------------------------------------------

    #[test]
    fn verify_signature_missing_header_returns_err() {
        let config = with_secret_config("s");
        assert!(verify_signature(&config, b"body", &[]).is_err());
    }

    // -------------------------------------------------------------------------
    // Test 13: run_id format is app_id-function_id-now_ms
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn invoke_run_id_format_is_app_fn_now() {
        with_server(no_secret_config(), |addr| async move {
            let body_json = serde_json::to_vec(&json!({
                "function_id": "fn-a",
                "payload": {}
            }))
            .expect("serialize");
            let request = format!(
                "POST /functions/invoke HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
                body_json.len()
            );
            let mut req_bytes = request.into_bytes();
            req_bytes.extend_from_slice(&body_json);

            let resp = raw_request(addr, &req_bytes).await;
            let value = response_body_json(&resp);
            let run_id = value["run_id"].as_str().expect("run_id string");
            // format: "test-fn-a-1700000000000"
            assert_eq!(run_id, format!("test-fn-a-{NOW}"));
        })
        .await;
    }
}
