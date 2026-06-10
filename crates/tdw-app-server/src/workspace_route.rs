//! `OpenBB` Workspace bridge route family (feature = `workspace-route`).
//!
//! Serves the three endpoints `OpenBB` Workspace needs from a custom data
//! backend, byte-compatible with the published *backends-for-openbb* contract:
//!
//! | Method   | Path                              | Action                               |
//! |----------|-----------------------------------|--------------------------------------|
//! | `GET`    | `/widgets.json`                   | Catalog-derived widget manifest      |
//! | `GET`    | `/apps.json`                      | Curated default app manifest         |
//! | `GET`    | `/widget-data/{route...}?<params>`| Resolve a catalog route + fetch rows |
//! | `OPTIONS`| (any of the above)                | CORS preflight                       |
//!
//! Like [`super::rest_route`], this is a hand-rolled HTTP/1.1 surface over a
//! plain `tokio::net::TcpStream` (no axum/hyper). `widgets.json` / `apps.json`
//! are derived from the endpoint catalog via [`tdw_widgets`]; `widget-data`
//! reuses the **same** policy-guarded fetch seam ([`super::RestApiHandler`]) the
//! REST family uses, so the Workspace surface never bypasses the daemon's
//! policy / hook / mask guards.
//!
//! # CORS
//!
//! Workspace runs in a browser at `pro.openbb.co`, so the family answers
//! `OPTIONS` preflights and stamps `Access-Control-Allow-*` on every response.
//! Allowed origins are configurable ([`WorkspaceConfig::allowed_origins`]); the
//! default permits `https://pro.openbb.co` plus the documented local dev
//! origins.
//!
//! # Auth
//!
//! Optional: when [`WorkspaceConfig::api_key`] is set, every request must carry
//! a matching `X-TDW-API-KEY` header (compared in constant time). The fetch seam
//! is otherwise the daemon's policy guard, mirroring the REST family's posture.
//!
//! Sources (public docs): the `widgets.json` / `apps.json` contract and CORS
//! requirements are from <https://docs.openbb.co/workspace/getting-started/custom-backend>.

#![cfg(feature = "workspace-route")]

use std::sync::Arc;

use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::rest_route::{RestApiHandler, RestError};

const MAX_HEADER_BYTES: usize = 16 * 1024;

/// Path prefix for the per-widget data endpoint; the remainder is the catalog
/// route (slash form).
const WIDGET_DATA_PREFIX: &str = "/widget-data/";

/// The header carrying the optional shared API key.
const API_KEY_HEADER: &str = "x-tdw-api-key";

/// Default CORS origins: the hosted `OpenBB` Workspace plus the documented
/// local dev origins (the Workspace dev server runs on `localhost:1420`).
const DEFAULT_ORIGINS: &[&str] = &[
    "https://pro.openbb.co",
    "http://localhost:1420",
    "http://127.0.0.1:1420",
];

/// Runtime configuration for the Workspace route family.
#[derive(Clone, Debug)]
pub struct WorkspaceConfig {
    /// Origins allowed by CORS. Empty means "fall back to [`DEFAULT_ORIGINS`]".
    pub allowed_origins: Vec<String>,
    /// Optional shared API key; when `Some`, requests must present a matching
    /// `X-TDW-API-KEY` header.
    pub api_key: Option<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            allowed_origins: DEFAULT_ORIGINS.iter().map(|s| (*s).to_string()).collect(),
            api_key: None,
        }
    }
}

impl WorkspaceConfig {
    /// Build a config from environment variables:
    /// - `TDW_WORKSPACE_CORS_ORIGINS` — comma-separated origin allow-list.
    /// - `TDW_WORKSPACE_API_KEY` — the optional shared key.
    #[must_use]
    pub fn from_env() -> Self {
        let allowed_origins = std::env::var("TDW_WORKSPACE_CORS_ORIGINS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|origin| !origin.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|origins| !origins.is_empty())
            .unwrap_or_else(|| DEFAULT_ORIGINS.iter().map(|s| (*s).to_string()).collect());
        let api_key = std::env::var("TDW_WORKSPACE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        Self {
            allowed_origins,
            api_key,
        }
    }

    /// Whether `origin` is permitted by the CORS allow-list.
    #[must_use]
    pub fn origin_allowed(&self, origin: &str) -> bool {
        self.allowed_origins.iter().any(|allowed| allowed == origin)
    }
}

/// Spawn an HTTP/1.1 listener serving the `OpenBB` Workspace bridge family.
///
/// `widgets.json` / `apps.json` are derived from the catalog; `widget-data`
/// resolves through `handler`. Returns when `cancel.cancelled()`.
///
/// # Errors
///
/// Returns an `io::Error` if accepting a connection fails.
pub async fn serve_workspace_http(
    listener: TcpListener,
    handler: Arc<dyn RestApiHandler>,
    config: WorkspaceConfig,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    let config = Arc::new(config);
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                let h = Arc::clone(&handler);
                let c = Arc::clone(&config);
                tokio::spawn(handle_workspace_conn(stream, h, c));
            }
        }
    }
    Ok(())
}

/// A parsed request head: method, path, query, and the headers we care about.
struct RequestHead {
    method: String,
    path: String,
    query: String,
    origin: Option<String>,
    api_key: Option<String>,
}

/// Handle one Workspace-family connection.
async fn handle_workspace_conn(
    mut stream: TcpStream,
    handler: Arc<dyn RestApiHandler>,
    config: Arc<WorkspaceConfig>,
) {
    let Some(head) = read_request_head(&mut stream).await else {
        return;
    };
    let cors = cors_headers(&config, head.origin.as_deref());

    // CORS preflight: answer OPTIONS without auth so the browser can probe.
    if head.method == "OPTIONS" {
        write_response(&mut stream, 204, "No Content", &cors, b"").await;
        return;
    }
    if head.method != "GET" {
        write_error(&mut stream, 405, "method not allowed", &cors).await;
        return;
    }
    // Optional shared-key auth (constant-time compare). Fail closed on mismatch.
    if let Some(expected) = config.api_key.as_deref()
        && !key_matches(expected, head.api_key.as_deref())
    {
        write_error(&mut stream, 401, "missing or invalid X-TDW-API-KEY", &cors).await;
        return;
    }

    route_request(&mut stream, &handler, &head, &cors).await;
}

/// Dispatch an authorized `GET` to the right handler.
async fn route_request(
    stream: &mut TcpStream,
    handler: &Arc<dyn RestApiHandler>,
    head: &RequestHead,
    cors: &[(String, String)],
) {
    match head.path.as_str() {
        "/widgets.json" => write_json(stream, &tdw_widgets::widgets_json(), cors).await,
        "/apps.json" => write_json(stream, &tdw_widgets::apps_json(), cors).await,
        path => {
            if let Some(route) = path.strip_prefix(WIDGET_DATA_PREFIX) {
                serve_widget_data(stream, handler, route, &head.query, cors).await;
            } else {
                write_error(stream, 404, "not a workspace route", cors).await;
            }
        }
    }
}

/// Resolve a `widget-data` route through the policy-guarded fetch seam and
/// return the daemon's result envelope (rows live under `results`, the
/// `dataKey` the derived widgets declare).
async fn serve_widget_data(
    stream: &mut TcpStream,
    handler: &Arc<dyn RestApiHandler>,
    route: &str,
    query: &str,
    cors: &[(String, String)],
) {
    if route.is_empty() {
        write_error(stream, 404, "empty widget-data route", cors).await;
        return;
    }
    let params = parse_query_params(query);
    match handler.fetch_route(route, params).await {
        Ok(body) => write_json(stream, &body, cors).await,
        Err(RestError::UnknownRoute(message) | RestError::InvalidParams(message)) => {
            write_error(stream, 400, &message, cors).await;
        }
        Err(RestError::Provider(message)) => write_error(stream, 502, &message, cors).await,
    }
}

/// Read and parse the request head (up to the blank line). Returns `None` on a
/// malformed / oversized / truncated head.
async fn read_request_head(stream: &mut TcpStream) -> Option<RequestHead> {
    let mut buf = vec![0u8; MAX_HEADER_BYTES];
    let mut filled = 0usize;
    let header_end = loop {
        if filled == buf.len() {
            return None;
        }
        let n = match stream.read(&mut buf[filled..]).await {
            Ok(0) | Err(_) => return None,
            Ok(n) => n,
        };
        filled += n;
        if let Some(pos) = buf[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
    };
    let header_str = std::str::from_utf8(&buf[..header_end]).ok()?;
    parse_head(header_str)
}

/// Parse a request head string into a [`RequestHead`].
fn parse_head(header_str: &str) -> Option<RequestHead> {
    let mut lines = header_str.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next().unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    let mut origin = None;
    let mut api_key = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let lower = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if lower == "origin" {
                origin = Some(value);
            } else if lower == API_KEY_HEADER {
                api_key = Some(value);
            }
        }
    }
    Some(RequestHead {
        method,
        path: path.to_string(),
        query: query.to_string(),
        origin,
        api_key,
    })
}

/// Build the CORS response headers for a request from `origin`.
///
/// When the origin is allow-listed it is echoed back (the spec form for
/// credentialed requests); otherwise the first configured origin is returned so
/// the browser blocks a disallowed caller. Always advertises the methods and
/// the `X-TDW-API-KEY` / `Authorization` request headers.
fn cors_headers(config: &WorkspaceConfig, origin: Option<&str>) -> Vec<(String, String)> {
    let allow_origin = match origin {
        Some(value) if config.origin_allowed(value) => value.to_string(),
        _ => config
            .allowed_origins
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_ORIGINS[0].to_string()),
    };
    vec![
        ("Access-Control-Allow-Origin".to_string(), allow_origin),
        (
            "Access-Control-Allow-Methods".to_string(),
            "GET, OPTIONS".to_string(),
        ),
        (
            "Access-Control-Allow-Headers".to_string(),
            "Content-Type, Authorization, X-TDW-API-KEY".to_string(),
        ),
        ("Vary".to_string(), "Origin".to_string()),
    ]
}

/// Constant-time check that `candidate` equals the configured key.
fn key_matches(expected: &str, candidate: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    constant_time_str_eq(expected, candidate)
}

/// Constant-time string equality, folding inputs into a fixed-width digest so
/// the compare neither short-circuits nor leaks input length (the same approach
/// `tdw-mcp` uses for its bearer-token check).
fn constant_time_str_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    fixed_digest(a.as_bytes())
        .ct_eq(&fixed_digest(b.as_bytes()))
        .into()
}

/// Fold arbitrary-length bytes into a fixed 32-byte FNV-1a digest across four
/// independently-seeded lanes (length-hiding input for the constant-time eq).
fn fixed_digest(bytes: &[u8]) -> [u8; 32] {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut digest = [0u8; 32];
    for (lane, chunk) in digest.chunks_mut(8).enumerate() {
        let mut hash = OFFSET ^ (lane as u64).wrapping_mul(PRIME);
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
        chunk.copy_from_slice(&hash.to_le_bytes());
    }
    digest
}

/// Parse a URL query string into a JSON object (scalar coercion for
/// numbers/bools), matching the REST family's parsing.
fn parse_query_params(query: &str) -> Value {
    let mut map = Map::new();
    if query.is_empty() {
        return Value::Object(map);
    }
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(raw_key);
        if key.is_empty() {
            continue;
        }
        map.insert(key, coerce_scalar(&percent_decode(raw_value)));
    }
    Value::Object(map)
}

/// Coerce a decoded query value to a JSON scalar when unambiguous, else string.
fn coerce_scalar(value: &str) -> Value {
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if let Ok(int) = value.parse::<i64>() {
        return Value::from(int);
    }
    if let Ok(float) = value.parse::<f64>()
        && float.is_finite()
        && let Some(number) = serde_json::Number::from_f64(float)
    {
        return Value::Number(number);
    }
    Value::String(value.to_string())
}

/// Minimal `application/x-www-form-urlencoded` percent-decoder (`%XX` + `+`).
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Write a `200 OK` JSON response (with CORS headers).
async fn write_json(stream: &mut TcpStream, value: &Value, cors: &[(String, String)]) {
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
        502 => "Bad Gateway",
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
    fn default_config_allows_pro_openbb_and_localhost() {
        let config = WorkspaceConfig::default();
        assert!(config.origin_allowed("https://pro.openbb.co"));
        assert!(config.origin_allowed("http://localhost:1420"));
        assert!(!config.origin_allowed("https://evil.example"));
    }

    #[test]
    fn cors_echoes_allowed_origin_and_blocks_others() {
        let config = WorkspaceConfig::default();
        let allowed = cors_headers(&config, Some("https://pro.openbb.co"));
        let origin = allowed
            .iter()
            .find(|(name, _)| name == "Access-Control-Allow-Origin")
            .map(|(_, value)| value.as_str());
        assert_eq!(origin, Some("https://pro.openbb.co"));

        // A disallowed origin is NOT echoed (the browser then blocks the read).
        let blocked = cors_headers(&config, Some("https://evil.example"));
        let blocked_origin = blocked
            .iter()
            .find(|(name, _)| name == "Access-Control-Allow-Origin")
            .map(|(_, value)| value.as_str());
        assert_ne!(blocked_origin, Some("https://evil.example"));
    }

    #[test]
    fn cors_advertises_the_api_key_request_header() {
        let headers = cors_headers(&WorkspaceConfig::default(), None);
        let allow_headers = headers
            .iter()
            .find(|(name, _)| name == "Access-Control-Allow-Headers")
            .map_or("", |(_, value)| value.as_str());
        assert!(allow_headers.contains("X-TDW-API-KEY"));
        assert!(allow_headers.contains("Authorization"));
    }

    #[test]
    fn key_matches_is_constant_time_equal() {
        assert!(key_matches("secret", Some("secret")));
        assert!(!key_matches("secret", Some("wrong")));
        assert!(!key_matches("secret", Some("secret-with-suffix")));
        assert!(!key_matches("secret", None));
    }

    #[test]
    fn query_params_coerce_scalars() {
        let value = parse_query_params("symbol=AAPL&limit=10&active=true");
        assert_eq!(value["symbol"], "AAPL");
        assert_eq!(value["limit"], 10);
        assert_eq!(value["active"], true);
    }

    #[test]
    fn parse_head_extracts_method_path_query_and_headers() {
        let raw = "GET /widget-data/equity/price/historical?symbol=AAPL HTTP/1.1\r\n\
                   Host: localhost\r\n\
                   Origin: https://pro.openbb.co\r\n\
                   X-TDW-API-KEY: abc123\r\n";
        let head = parse_head(raw).expect("head parses");
        assert_eq!(head.method, "GET");
        assert_eq!(head.path, "/widget-data/equity/price/historical");
        assert_eq!(head.query, "symbol=AAPL");
        assert_eq!(head.origin.as_deref(), Some("https://pro.openbb.co"));
        assert_eq!(head.api_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn from_env_default_when_unset_uses_default_origins() {
        // Construct directly (env-free) to assert the default-origin invariant
        // the from_env fallback relies on, without mutating process env.
        let config = WorkspaceConfig::default();
        assert_eq!(config.allowed_origins.len(), DEFAULT_ORIGINS.len());
        assert!(config.api_key.is_none());
    }
}
