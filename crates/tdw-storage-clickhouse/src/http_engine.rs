//! Real ClickHouse backend for `tdw_core::OlapEngine`.
//!
//! Gated by the `clickhouse` feature. Talks to ClickHouse over its
//! native HTTP interface (port 8123 by default), so no SDK crate is
//! required — just `reqwest`. Works with any ClickHouse-compatible
//! HTTP endpoint (ClickHouse Cloud, single-node, ClickHouse keeper +
//! distributed table cluster, etc.).
//!
//! Authentication uses HTTP basic auth via the constructor; ClickHouse
//! accepts credentials this way for the standard HTTP interface.
//!
//! Parameter binding is not supported in this slice. ClickHouse's HTTP
//! interface supports server-side params via `param_<name>` query
//! string keys, which is a different binding shape than sqlx-style
//! positional `$N`. Adding that surface is a follow-up; the engine
//! rejects non-null `params` with a clear error so callers know to
//! extend the binding surface deliberately.

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde_json::Value;
use tdw_core::{Error, OlapEngine, Result};

/// Production ClickHouse backend. Construct via
/// [`ClickHouseHttpEngine::new`].
#[derive(Clone, Debug)]
pub struct ClickHouseHttpEngine {
    client: Client,
    base_url: Url,
    user: Option<String>,
    password: Option<String>,
}

impl ClickHouseHttpEngine {
    /// Build a ClickHouse HTTP client. `endpoint` is the base URL
    /// (e.g. `http://127.0.0.1:8123`). `user` / `password` are
    /// optional; pass `None` for the default `default` user on a
    /// locally-running ClickHouse with no auth.
    pub fn new(endpoint: &str, user: Option<String>, password: Option<String>) -> Result<Self> {
        let base_url = Url::parse(endpoint)
            .map_err(|error| Error::Storage(format!("clickhouse endpoint: {error}")))?;
        let client = Client::builder()
            .user_agent("tdw-storage-clickhouse/0.1")
            .build()
            .map_err(|error| Error::Storage(format!("clickhouse client: {error}")))?;
        Ok(Self {
            client,
            base_url,
            user,
            password,
        })
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
        let response = self
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
        let body = response
            .text()
            .await
            .map_err(|error| Error::Storage(format!("clickhouse read body: {error}")))?;
        serde_json::from_str(&body)
            .map_err(|error| Error::Storage(format!("clickhouse parse_json: {error}")))
    }
}
