//! Catalog-derived REST surface (feature = "rest-api-route").
//!
//! "Routes are data, not code": this route family turns every catalog `Fetch`
//! route into a `GET` endpoint without a per-route handler. It mirrors the
//! hand-rolled HTTP/1.1 pattern of [`super::transport_http`] /
//! [`super::functions_route`] (plain `tokio::net::TcpStream`, no axum/hyper).
//!
//! | Method | Path                          | Action                              |
//! |--------|-------------------------------|-------------------------------------|
//! | `GET`  | `/api/v1/{route...}?<params>` | Resolve a catalog route + fetch     |
//! | `GET`  | `/openapi.json`               | Serve the generated `OpenAPI` doc   |
//!
//! # Transport-layer decoupling
//!
//! Like [`super::functions_route`], this route does **not** depend on
//! `tdw-service-api` (that would form a dependency cycle). Callers implement the
//! [`RestApiHandler`] trait on a wrapper around their `AppState` and pass an
//! `Arc<dyn RestApiHandler>` to [`serve_rest_http`]. The handler runs the same
//! policy-guarded `Op::FetchData` path, so REST never bypasses the daemon's
//! auth / hook / mask guards.
//!
//! # Auth
//!
//! This family mirrors `POST /op`: it is **unauthenticated on the loopback
//! transport**. `POST /op` (the daemon's primary ingress) performs no
//! per-request signature check — the daemon's security boundary is the loopback
//! bind plus the policy guard inside the handler. The REST family adopts the
//! same posture: no HMAC/session check at the transport, with the in-handler
//! policy guard (`enforce_request_path_with_backend`) as the real boundary.
//! Operators binding a non-loopback address must front the daemon with a
//! token/mTLS/reverse-proxy layer (see `docs/products/rest-api.md`).
//!
//! # `OpenAPI` source of truth
//!
//! `/openapi.json` is served from the document embedded via `include_str!` of
//! the checked-in `docs/schemas/openapi.json`, which the xtask `openapi-sync`
//! command generates from the same catalog. The `openapi-check` drift gate keeps
//! the checked-in doc — and therefore the served doc — in lockstep with the
//! catalog, so the server and the file can never diverge.

#![cfg(feature = "rest-api-route")]

use std::sync::Arc;

use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const MAX_HEADER_BYTES: usize = 16 * 1024;

/// The route-family prefix. Anything under this path is treated as a catalog
/// route lookup; anything else (except `/openapi.json`) is `404`.
const API_PREFIX: &str = "/api/v1/";

/// The generated `OpenAPI` 3.1 document, embedded so the served document and the
/// checked-in `docs/schemas/openapi.json` cannot drift.
const OPENAPI_JSON: &str = include_str!("../../../docs/schemas/openapi.json");

/// Failure classes a [`RestApiHandler`] can return, mapped to HTTP status codes
/// by the transport.
#[derive(Debug)]
pub enum RestError {
    /// The path is a well-formed `/api/v1/...` route but is not in the catalog
    /// (or is not a fetch route). Mapped to `400`; the message should include
    /// the known-routes list so clients can discover valid routes.
    UnknownRoute(String),
    /// The query parameters failed validation (a caller error that would fail
    /// against every provider). Mapped to `400`.
    InvalidParams(String),
    /// Every candidate provider failed (a provider-side error). Mapped to `502`.
    Provider(String),
}

/// Abstract interface over the daemon's catalog fetch path, implemented by the
/// crate that wires everything together (e.g. `tdw-service-api`) to avoid a
/// dependency cycle.
///
/// All methods are `async` and take `&self` so implementations can hold an
/// `Arc`-shared `AppState`.
#[async_trait::async_trait]
pub trait RestApiHandler: Send + Sync {
    /// Resolve `route` against the catalog and fetch its records through the
    /// same policy-guarded path `Op::FetchData` uses, returning the response
    /// JSON body (the `ResultEnvelope`-shaped value the daemon emits).
    ///
    /// `params` is the query string already parsed into a JSON object (string
    /// values, with JSON-scalar coercion for numbers/bools); the handler's
    /// `QueryParams` validation coerces the rest.
    ///
    /// # Errors
    ///
    /// Returns [`RestError::UnknownRoute`] for a non-catalog/non-fetch route,
    /// [`RestError::InvalidParams`] for invalid query parameters, or
    /// [`RestError::Provider`] when every candidate provider fails.
    async fn fetch_route(&self, route: &str, params: Value) -> Result<Value, RestError>;
}

/// Spawn an HTTP/1.1 listener serving the catalog-derived REST family.
///
/// - `GET /openapi.json`          → the embedded `OpenAPI` 3.1 document.
/// - `GET /api/v1/{route...}?...` → resolve + fetch via `handler`.
/// - anything else                → `404`.
///
/// Cancellation: returns when `cancel.cancelled()`.
///
/// # Errors
///
/// Returns an `io::Error` if accepting a connection fails.
pub async fn serve_rest_http(
    listener: TcpListener,
    handler: Arc<dyn RestApiHandler>,
    cancel: CancellationToken,
) -> std::io::Result<()> {
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _peer) = accept?;
                let h = Arc::clone(&handler);
                tokio::spawn(handle_rest_conn(stream, h));
            }
        }
    }
    Ok(())
}

/// Handle a single connection that matched the REST route family.
pub(super) async fn handle_rest_conn(mut stream: TcpStream, handler: Arc<dyn RestApiHandler>) {
    let mut header_buf = vec![0u8; MAX_HEADER_BYTES];
    let mut filled = 0usize;

    let header_end = loop {
        if filled == header_buf.len() {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\n\r\n",
                )
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
    let Ok(header_str) = std::str::from_utf8(header_section) else {
        write_error(&mut stream, 400, "malformed request header").await;
        return;
    };

    let mut lines = header_str.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    // Only GET is supported by this family.
    if method != "GET" {
        write_error(
            &mut stream,
            405,
            "method not allowed; this route family is GET-only",
        )
        .await;
        return;
    }

    let (path, query) = split_target(target);

    if path == "/openapi.json" {
        write_json_str_200(&mut stream, OPENAPI_JSON).await;
        return;
    }

    let Some(route) = path.strip_prefix(API_PREFIX) else {
        write_error(&mut stream, 404, "not a catalog route; use /api/v1/<route>").await;
        return;
    };
    if route.is_empty() {
        write_error(&mut stream, 404, "empty catalog route").await;
        return;
    }

    let params = parse_query_params(query);

    match handler.fetch_route(route, params).await {
        Ok(body) => write_json_value_200(&mut stream, &body).await,
        // Unknown route and invalid params are both caller errors -> 400.
        Err(RestError::UnknownRoute(message) | RestError::InvalidParams(message)) => {
            write_error(&mut stream, 400, &message).await;
        }
        Err(RestError::Provider(message)) => write_error(&mut stream, 502, &message).await,
    }
}

/// Split an HTTP request target into `(path, query)` (query excludes the `?`).
fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').map_or((target, ""), |(p, q)| (p, q))
}

/// Parse a URL query string into a JSON object.
///
/// Each `key=value` pair is percent-decoded; the value is parsed as a JSON
/// scalar when it cleanly is one (`true`/`false`/number), otherwise kept as a
/// string. Repeated keys collapse to the last value. Keys with no `=` map to an
/// empty string. The `QueryParams` validation in the handler coerces the rest.
fn parse_query_params(query: &str) -> Value {
    let mut map = Map::new();
    if query.is_empty() {
        return Value::Object(map);
    }
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(raw_key);
        if key.is_empty() {
            continue;
        }
        let value = percent_decode(raw_value);
        map.insert(key, coerce_scalar(&value));
    }
    Value::Object(map)
}

/// Coerce a decoded query value to a JSON scalar when it is unambiguously one,
/// else keep it as a JSON string.
///
/// Bools and numbers parse as their JSON scalar form so a route's typed params
/// (e.g. a numeric `limit`) deserialize without a per-route hint; everything
/// else stays a string (the common case for symbols, dates, providers).
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

/// Minimal `application/x-www-form-urlencoded` percent-decoder (`%XX` + `+`→space).
///
/// No external dependency; tolerant of malformed escapes (a stray `%` or a
/// non-hex pair is kept verbatim) so a bad escape never drops the rest of the
/// value.
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

/// Write a `200 OK` response with a JSON body from a `serde_json::Value`.
async fn write_json_value_200(stream: &mut TcpStream, value: &Value) {
    let Ok(body) = serde_json::to_vec(value) else {
        let _ = stream
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
            .await;
        return;
    };
    write_json_bytes_200(stream, &body).await;
}

/// Write a `200 OK` response with a pre-rendered JSON string body (e.g. the
/// embedded `OpenAPI` document).
async fn write_json_str_200(stream: &mut TcpStream, body: &str) {
    write_json_bytes_200(stream, body.as_bytes()).await;
}

async fn write_json_bytes_200(stream: &mut TcpStream, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

/// Write a `{status} ...` response with a `{ "error": message }` body.
async fn write_error(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        _ => "Internal Server Error",
    };
    let body = serde_json::json!({ "error": message });
    let body_bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body_bytes.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body_bytes).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_target_separates_path_and_query() {
        assert_eq!(
            split_target("/api/v1/equity/price/historical"),
            ("/api/v1/equity/price/historical", "")
        );
        assert_eq!(
            split_target("/api/v1/equity/price/historical?symbol=AAPL&provider=yahoo"),
            (
                "/api/v1/equity/price/historical",
                "symbol=AAPL&provider=yahoo"
            )
        );
    }

    #[test]
    fn query_params_parse_into_json_object_with_scalar_coercion() {
        let value = parse_query_params("symbol=AAPL&provider=yahoo&limit=10&active=true");
        assert_eq!(value["symbol"], "AAPL");
        assert_eq!(value["provider"], "yahoo");
        assert_eq!(value["limit"], 10);
        assert_eq!(value["active"], true);
    }

    #[test]
    fn query_params_empty_is_empty_object() {
        assert_eq!(parse_query_params(""), Value::Object(Map::new()));
    }

    #[test]
    fn percent_decoding_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("hello+world"), "hello world");
        // Malformed escape kept verbatim rather than dropping the rest.
        assert_eq!(percent_decode("50%"), "50%");
    }

    #[test]
    fn coerce_scalar_keeps_ambiguous_strings() {
        assert_eq!(coerce_scalar("AAPL"), Value::String("AAPL".to_string()));
        // A date-like string is NOT a number, stays a string.
        assert_eq!(
            coerce_scalar("2025-01-01"),
            Value::String("2025-01-01".to_string())
        );
        assert_eq!(coerce_scalar("42"), Value::from(42));
        assert_eq!(coerce_scalar("true"), Value::Bool(true));
    }

    #[test]
    fn embedded_openapi_doc_parses_as_json() {
        let doc: Value = serde_json::from_str(OPENAPI_JSON).expect("embedded openapi.json parses");
        assert_eq!(doc["openapi"], "3.1.0");
        assert!(doc["paths"].as_object().is_some_and(|p| !p.is_empty()));
    }
}
