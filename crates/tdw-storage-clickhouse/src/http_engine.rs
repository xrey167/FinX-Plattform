//! Real `ClickHouse` backend for `tdw_core::OlapEngine`.
//!
//! Gated by the `clickhouse` feature. Talks to `ClickHouse` over its
//! native HTTP interface (port 8123 by default), so no SDK crate is
//! required — just `reqwest`. Works with any ClickHouse-compatible
//! HTTP endpoint (`ClickHouse` Cloud, single-node, `ClickHouse` keeper +
//! distributed table cluster, etc.).
//!
//! Authentication uses HTTP basic auth via the constructor; `ClickHouse`
//! accepts credentials this way for the standard HTTP interface.
//!
//! Parameter binding is not supported in this slice. `ClickHouse`'s HTTP
//! interface supports server-side params via `param_<name>` query
//! string keys, which is a different binding shape than sqlx-style
//! positional `$N`. Adding that surface is a follow-up; the engine
//! rejects non-null `params` with a clear error so callers know to
//! extend the binding surface deliberately.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde_json::Value;
use tdw_core::{Error, OlapEngine, Result};

/// Cap on how long establishing a connection may take. A stalled/black-holed
/// endpoint fails fast here instead of hanging the calling op (CHH1).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Generous overall request timeout — analytical queries can legitimately run
/// for minutes, so this is only a backstop against a truly wedged connection,
/// not a query-latency budget (CHH1).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Default cap on a `query_json` response body (256 MiB). Bounds memory so a
/// huge result set cannot OOM the process; tune via
/// [`ClickHouseHttpEngine::with_max_response_bytes`] (CHH2).
const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024 * 1024;

/// Production `ClickHouse` backend. Construct via
/// [`ClickHouseHttpEngine::new`].
#[derive(Clone, Debug)]
pub struct ClickHouseHttpEngine {
    client: Client,
    base_url: Url,
    user: Option<String>,
    password: Option<String>,
    max_response_bytes: usize,
}

impl ClickHouseHttpEngine {
    /// Build a `ClickHouse` HTTP client. `endpoint` is the base URL
    /// (e.g. `http://127.0.0.1:8123`). `user` / `password` are
    /// optional; pass `None` for the default `default` user on a
    /// locally-running `ClickHouse` with no auth.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(endpoint: &str, user: Option<String>, password: Option<String>) -> Result<Self> {
        let base_url = Url::parse(endpoint)
            .map_err(|error| Error::Storage(format!("clickhouse endpoint: {error}")))?;
        let client = Client::builder()
            .user_agent("tdw-storage-clickhouse/0.1")
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
            .build()
            .map_err(|error| Error::Storage(format!("clickhouse client: {error}")))?;
        Ok(Self {
            client,
            base_url,
            user,
            password,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        })
    }

    /// Override the maximum `query_json` response size (default 256 MiB). A
    /// response exceeding this is rejected rather than buffered, so a large
    /// result set cannot OOM the process.
    #[must_use]
    pub const fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }

    fn request(&self, query: &str) -> reqwest::RequestBuilder {
        // Send the SQL in the request body (the idiomatic ClickHouse HTTP
        // pattern). reqwest sets `Content-Length` automatically for known-
        // length bodies; ClickHouse 25.5+ rejects POSTs lacking
        // `Content-Length` (or `Transfer-Encoding: chunked`) with
        // `411 Length Required` (Code 381).
        //
        // Always send basic_auth: when no user is configured, fall back to the
        // well-known `default` user with no password. ClickHouse 25.5+ returns
        // `401 Unauthorized` (Code 194) for body-style POSTs that lack
        // explicit credentials, even though the legacy URL-parameter form was
        // anonymous-by-default.
        let user = self.user.as_deref().unwrap_or("default");
        self.client
            .post(self.base_url.clone())
            .body(query.to_owned())
            .basic_auth(user, self.password.as_deref())
    }
}

#[async_trait]
impl OlapEngine for ClickHouseHttpEngine {
    async fn execute(&self, ddl: &str) -> Result<()> {
        if ddl.trim().is_empty() {
            return Err(Error::Storage(
                "clickhouse sql must not be empty".to_string(),
            ));
        }
        let response = self
            .request(ddl)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("clickhouse execute: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "clickhouse execute returned {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn query_json(&self, sql: &str, params: Value) -> Result<Value> {
        if sql.trim().is_empty() {
            return Err(Error::Storage(
                "clickhouse sql must not be empty".to_string(),
            ));
        }
        if !matches!(params, Value::Null) {
            return Err(Error::Storage(
                "clickhouse param binding is not supported in this slice".to_string(),
            ));
        }
        // Append FORMAT JSON so ClickHouse returns a JSON object with
        // `meta` (schema) and `data` (rows). The default response is
        // tab-separated text and would need a separate parser.
        let formatted = format!("{sql} FORMAT JSON");
        let mut response = self
            .request(&formatted)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("clickhouse query_json: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "clickhouse query_json returned {status}: {body}"
            )));
        }
        // Read the body incrementally with a hard cap so an unexpectedly large
        // OLAP result set cannot OOM the process (CHH2). `text()` would buffer
        // the whole response unconditionally.
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| Error::Storage(format!("clickhouse read body: {error}")))?
        {
            if would_exceed_cap(body.len(), chunk.len(), self.max_response_bytes) {
                return Err(Error::Storage(format!(
                    "clickhouse query_json result exceeds the {}-byte cap",
                    self.max_response_bytes
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(body)
            .map_err(|error| Error::Storage(format!("clickhouse body utf8: {error}")))?;
        serde_json::from_str(&text)
            .map_err(|error| Error::Storage(format!("clickhouse parse_json: {error}")))
    }
}

/// Whether appending `incoming` bytes to an already-buffered `current` would
/// exceed `max`. Uses a saturating add so a malicious/huge `incoming` cannot
/// wrap and bypass the cap.
fn would_exceed_cap(current: usize, incoming: usize, max: usize) -> bool {
    current.saturating_add(incoming) > max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn would_exceed_cap_guards_the_boundary() {
        // Exactly at the cap is allowed; one over is rejected.
        assert!(!would_exceed_cap(90, 10, 100));
        assert!(would_exceed_cap(90, 11, 100));
        // A huge incoming chunk cannot wrap past the cap.
        assert!(would_exceed_cap(1, usize::MAX, 100));
        // Empty additions never exceed.
        assert!(!would_exceed_cap(100, 0, 100));
    }

    #[test]
    fn new_sets_a_default_response_cap() {
        let engine =
            ClickHouseHttpEngine::new("http://127.0.0.1:8123", None, None).expect("engine builds");
        assert_eq!(engine.max_response_bytes, DEFAULT_MAX_RESPONSE_BYTES);
        let tuned = engine.with_max_response_bytes(1_024);
        assert_eq!(tuned.max_response_bytes, 1_024);
    }
}
