#![forbid(unsafe_code)]
#![cfg(feature = "http")]

//! Shared generic HTTP fetcher scaffolding for `tdw-provider-*` crates.
//!
//! Each provider supplies a zero-sized [`ProviderSpec`] type that declares the
//! provider/endpoint/user-agent/base-url constants, the error-message
//! fragments, plus the `transform_query`/`build_request`/`transform_data`
//! hooks. The generic [`HttpFetcher`] owns the `{ base_url }` state, `Default`,
//! `with_base_url`, `registry_entry`, and the blanket [`tdw_core::Fetcher`]
//! implementation (client build + send + status-check + bytes), collapsing the
//! otherwise identical per-provider scaffolding into a single definition.
//!
//! The client build is delegated to [`tdw_core::http_support::build_client`] so
//! the already-shared builder helper is reused (not re-inlined).
//!
//! Observable behavior is unchanged: [`ProviderSpec::build_request`] returns a
//! fully-built [`reqwest::RequestBuilder`] (method + url + query + headers +
//! auth), and the error-message fragment consts reproduce each provider's
//! existing error strings byte-for-byte.

use core::marker::PhantomData;

use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

/// Per-provider specification consumed by the generic [`HttpFetcher`].
///
/// Implementors are zero-sized marker types. The associated consts and hooks
/// fully describe the provider's request/response behavior, including the exact
/// error-message fragments so observable error strings are unchanged.
pub trait ProviderSpec: Send + Sync + 'static {
    /// Canonical provider name (e.g. `"glassnode"`).
    const PROVIDER: &'static str;
    /// Endpoint identifier within the provider (e.g. `"metric"`).
    const ENDPOINT: &'static str;
    /// `User-Agent` header sent on every request.
    const USER_AGENT: &'static str;
    /// Default base URL used by [`HttpFetcher::default`].
    const DEFAULT_BASE_URL: &'static str;

    /// Error-message prefix used when the reqwest client fails to build
    /// (formatted as `"{CLIENT_ERR}: {e}"`).
    const CLIENT_ERR: &'static str;
    /// Error-message prefix used when sending the request fails
    /// (formatted as `"{SEND_ERR}: {e}"`).
    const SEND_ERR: &'static str;
    /// Error-message prefix used on a non-success status
    /// (formatted as `"{RETURNED_ERR} {status}: {body}"`).
    const RETURNED_ERR: &'static str;
    /// Error-message prefix used when reading the body fails
    /// (formatted as `"{READ_BODY_ERR}: {e}"`).
    const READ_BODY_ERR: &'static str;
    /// Placeholder rendered when an error-response body is unreadable.
    ///
    /// * `None` => `response.text().await.unwrap_or_default()` (empty string).
    /// * `Some(s)` => `response.text().await.unwrap_or_else(|_| s.to_string())`.
    ///
    /// This reproduces the three observed families byte-identically: empty,
    /// `"<unreadable body>"`, and `"<unreadable>"`.
    const UNREADABLE_BODY: Option<&'static str> = None;

    /// Validated query parameter type.
    type Query: tdw_core::QueryParams;
    /// Parsed output row type.
    type Data: tdw_core::DataModel;

    /// Validate and build a typed query from raw JSON params.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the params are invalid.
    fn transform_query(params: Value) -> Result<Self::Query>;

    /// Build the fully-configured HTTP request (url + query + headers + auth)
    /// for `query`, ready to `.send()`.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the request cannot be constructed (e.g. a
    /// missing API key or an invalid request path).
    fn build_request(
        base_url: &str,
        query: &Self::Query,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder>;

    /// Parse the raw response bytes into output rows.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the payload cannot be parsed.
    fn transform_data(query: &Self::Query, raw: Bytes) -> Result<Vec<Self::Data>>;
}

/// Generic provider HTTP fetcher parameterised over a [`ProviderSpec`].
#[derive(Clone, Debug)]
pub struct HttpFetcher<P: ProviderSpec> {
    base_url: String,
    _spec: PhantomData<fn() -> P>,
}

impl<P: ProviderSpec> Default for HttpFetcher<P> {
    fn default() -> Self {
        Self {
            base_url: P::DEFAULT_BASE_URL.to_string(),
            _spec: PhantomData,
        }
    }
}

impl<P: ProviderSpec> HttpFetcher<P> {
    /// Create a new fetcher pointing at the spec's canonical base URL.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the base URL (useful for testing against a mock server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the spec's canonical provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(P::PROVIDER, P::ENDPOINT)
    }
}

/// Send `req`, enforce a success status, and return the response body bytes.
///
/// Reproduces each provider's existing extract_data core (send + status-check +
/// bytes) using the spec's error-message fragments so error strings remain
/// byte-identical.
async fn send_expect_bytes<P: ProviderSpec>(req: reqwest::RequestBuilder) -> Result<Bytes> {
    let response = req
        .send()
        .await
        .map_err(|e| Error::Provider(format!("{}: {e}", P::SEND_ERR)))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = match P::UNREADABLE_BODY {
            Some(placeholder) => response
                .text()
                .await
                .unwrap_or_else(|_| placeholder.to_string()),
            None => response.text().await.unwrap_or_default(),
        };
        return Err(Error::Provider(format!(
            "{} {status}: {body}",
            P::RETURNED_ERR
        )));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("{}: {e}", P::READ_BODY_ERR)))
}

#[async_trait::async_trait]
impl<P: ProviderSpec> Fetcher<P::Query, P::Data> for HttpFetcher<P> {
    const PROVIDER: &'static str = P::PROVIDER;
    const ENDPOINT: &'static str = P::ENDPOINT;

    fn transform_query(params: Value) -> Result<P::Query> {
        P::transform_query(params)
    }

    async fn extract_data(&self, query: &P::Query, _creds: &Credentials) -> Result<Bytes> {
        let client = tdw_core::http_support::build_client(P::USER_AGENT, P::CLIENT_ERR)?;
        let req = P::build_request(&self.base_url, query, &client)?;
        send_expect_bytes::<P>(req).await
    }

    fn transform_data(&self, query: &P::Query, raw: Bytes) -> Result<Vec<P::Data>> {
        P::transform_data(query, raw)
    }
}
