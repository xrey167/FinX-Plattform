//! Shared HTTP fetcher scaffolding for `tdw-provider-*` crates.
//!
//! Gated by the `http` feature. Provides:
//!
//! * [`prelude`] — a glob re-export of the import preamble shared by every
//!   provider `http_fetcher.rs` (`async_trait`, [`Bytes`], reqwest [`Client`],
//!   serde_json [`Value`], plus the `tdw_core` fetcher symbols). Migrated
//!   providers replace their multi-line `use` preamble with a single
//!   `use tdw_core::http_support::prelude::*;`.
//! * [`build_client`] / [`read_required_key`] — the two boilerplate helper
//!   functions previously hand-copied across providers. Both take a `ctx`
//!   string so the resulting error text stays byte-identical per provider.

use crate::{Error, Result};
use reqwest::Client;

/// Re-export of the import preamble shared across provider `http_fetcher.rs`
/// files. Importing `use tdw_core::http_support::prelude::*;` collapses the
/// recurring multi-line `use` block to a single line.
///
/// `serde::Deserialize` is intentionally NOT re-exported here: it is a derive
/// macro used by only some providers, so each migrated file keeps its own
/// `use serde::Deserialize;` line when needed.
pub mod prelude {
    pub use crate::{Credentials, Error, Fetcher, RegistryEntry, Result};
    pub use async_trait::async_trait;
    pub use bytes::Bytes;
    pub use reqwest::Client;
    pub use serde_json::Value;
}

/// Build a `reqwest` [`Client`] with the given `user_agent`, mapping any
/// builder error to [`Error::Provider`] using `ctx` as the message prefix.
///
/// Reproduces the per-provider error text `"{ctx}: {error}"` byte-identically.
///
/// All provider HTTP traffic through this client is bounded: 10s connect
/// timeout, 30s total request timeout (the G005 bounded-I/O policy), so a
/// stalled upstream cannot hang a dispatcher worker indefinitely.
pub fn build_client(user_agent: &str, ctx: &str) -> Result<Client> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(user_agent)
        .build()
        .map_err(|source| Error::HttpClient {
            message: format!("{ctx}: {source}"),
            source,
        })
}

/// Read a required API key from the `env` environment variable, trimming
/// whitespace and rejecting empty values. On absence, returns
/// [`Error::Provider`] with the byte-identical text
/// `"{ctx} api key env {env} must be set"`.
pub fn read_required_key(env: &str, ctx: &str) -> Result<String> {
    std::env::var(env)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Provider(format!("{ctx} api key env {env} must be set")))
}
