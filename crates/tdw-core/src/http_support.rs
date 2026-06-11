//! Shared HTTP fetcher scaffolding for `tdw-provider-*` crates.
//!
//! Gated by the `http` feature. Provides:
//!
//! * [`prelude`] — a glob re-export of the import preamble shared by every
//!   provider `http_fetcher.rs` (`async_trait`, [`Bytes`], reqwest [`Client`],
//!   serde_json [`Value`], plus the `tdw_core` fetcher symbols). Migrated
//!   providers replace their multi-line `use` preamble with a single
//!   `use tdw_core::http_support::prelude::*;`.
//! * [`build_client`] / [`read_required_key`] / [`read_optional_key`] — the
//!   shared credential helpers used by every provider. All key reads go through
//!   one of these two functions; no provider rolls its own `env::var` loop.
//!
//! ## Credential precedence (G008 CORE2)
//!
//! **Today's implemented order** (config-file sourcing is not yet wired):
//!
//! 1. **Environment variable** — the provider-declared `env` name.
//! 2. **Hard error** (required keys) / **`None`** (optional keys).
//!
//! Config-value override is reserved for a future extension; no provider
//! currently passes a config value here. When that path is wired, this list
//! will gain a step 0 above env-var lookup. Do not document it as "already
//! implemented" until it is.
//!
//! Silent fallbacks ("warn and continue without a key") are banned by the
//! workspace policy established by B6's `[knowledge.embedding]` precedent.
//! A missing required key is always a hard `Error::Provider`; a missing
//! optional key returns `None` and the provider degrades gracefully (e.g.
//! BLS unauthenticated tier, CoinGecko free tier).
//!
//! ## Architecture layering (G008 CORE2)
//!
//! * **`tdw-config::credential_registry`** — sole declaration surface. Owns
//!   the mapping of provider name → env-var name → optional config-file key.
//!   This is the authoritative reference for "which env var does provider X
//!   use?". `tdw-config` has no dependency on `tdw-core`.
//! * **This module (`tdw-core::http_support`)** — resolution logic only. The
//!   `read_required_key` / `read_optional_key` functions accept an env-var
//!   name as a `&str` parameter; they have no knowledge of provider names.
//!   Provider crates supply the name via their own `*_ENV` / `TOKEN_ENVS`
//!   constant, which must match the `env_var` field in the registry.
//!
//! The two layers serve different callers and are intentionally separate:
//! the registry is consumed by the REST / MCP credential-listing surface; the
//! resolution helpers are consumed by individual provider `http_fetcher.rs`
//! files at fetch time. Neither layer duplicates the other's concerns.
//!
//! ## Error-value collapse
//!
//! Both `read_required_key` and `read_optional_key` call
//! `std::env::var(env).ok()`, which collapses `VarError::NotPresent` and
//! `VarError::NotUnicode` into a single `None`. A non-UTF-8 env-var value is
//! therefore treated identically to an absent variable: required keys error
//! with the standard "must be set" message; optional keys return `None`. This
//! is intentional — a garbled key cannot authenticate successfully anyway.

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
        .map_err(|error| Error::Provider(format!("{ctx}: {error}")))
}

/// Resolve a raw env-var value (already fetched or synthesised) into a
/// trimmed, non-empty string, or return a hard error.
///
/// This is the shared inner logic for [`read_required_key`]; it is also
/// directly testable without touching process environment (which is `unsafe`
/// in Rust 2024 edition with `unsafe_code = "forbid"`).
///
/// # Errors
///
/// Returns [`Error::Provider`] when `raw` is `None` or blank after trimming.
fn resolve_required(raw: Option<String>, env: &str, ctx: &str) -> Result<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Provider(format!("{ctx} api key env {env} must be set")))
}

/// Resolve a raw env-var value (already fetched or synthesised) into a
/// trimmed, non-empty string, or `None`.
///
/// This is the shared inner logic for [`read_optional_key`]; directly
/// testable without process-environment mutation.
fn resolve_optional(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Read a **required** credential from the `env` environment variable.
///
/// Trims whitespace and rejects empty values. On absence or emptiness returns
/// [`Error::Provider`] with the canonical text
/// `"{ctx} api key env {env} must be set"` — never a silent fallback.
///
/// This is the single authoritative implementation for all providers that
/// require a key; see the module-level credential precedence note.
///
/// # Errors
///
/// Returns [`Error::Provider`] when the variable is absent or blank.
pub fn read_required_key(env: &str, ctx: &str) -> Result<String> {
    resolve_required(std::env::var(env).ok(), env, ctx)
}

/// Read an **optional** credential from the `env` environment variable.
///
/// Returns `Some(key)` when the variable is set and non-empty after trimming,
/// `None` otherwise. Never errors — the caller decides how to degrade when the
/// key is absent (e.g. use the unauthenticated API tier).
///
/// This is the single authoritative implementation for providers where the key
/// is optional (BLS unauthenticated tier, CoinGecko free tier).
pub fn read_optional_key(env: &str) -> Option<String> {
    resolve_optional(std::env::var(env).ok())
}

#[cfg(test)]
mod tests {
    use super::{resolve_optional, resolve_required};

    // All tests call the inner `resolve_*` functions directly so they never
    // mutate the process environment — `set_var`/`remove_var` are `unsafe` in
    // Rust 2024 edition and forbidden by the workspace `unsafe_code = "forbid"`
    // lint.  The absent-env path is also exercised by `read_required_key` and
    // `read_optional_key` tests in provider integration suites.

    const ENV_NAME: &str = "TDW_TEST_CRED_RESOLVER_KEY";
    const CTX: &str = "testprovider";

    // ---------------------------------------------------------------------------
    // resolve_required — precedence: present → trimmed value; absent/blank → error
    // ---------------------------------------------------------------------------

    #[test]
    fn required_present_returns_trimmed_value() {
        let result = resolve_required(Some("  my-secret  ".to_string()), ENV_NAME, CTX);
        assert_eq!(result.expect("expected Ok"), "my-secret");
    }

    #[test]
    fn required_absent_is_hard_error() {
        let err = resolve_required(None, ENV_NAME, CTX).expect_err("expected Err");
        let msg = err.to_string();
        assert!(
            msg.contains("testprovider api key env"),
            "unexpected error text: {msg}"
        );
        assert!(msg.contains(ENV_NAME), "missing env name in error: {msg}");
        assert!(
            msg.contains("must be set"),
            "missing sentinel in error: {msg}"
        );
    }

    #[test]
    fn required_blank_is_hard_error() {
        let err =
            resolve_required(Some("   ".to_string()), ENV_NAME, CTX).expect_err("expected Err");
        let msg = err.to_string();
        assert!(msg.contains("must be set"), "unexpected error text: {msg}");
    }

    // ---------------------------------------------------------------------------
    // resolve_optional — present → Some(trimmed); absent/blank → None
    // ---------------------------------------------------------------------------

    #[test]
    fn optional_present_returns_some_trimmed() {
        assert_eq!(
            resolve_optional(Some("  opt-key  ".to_string())),
            Some("opt-key".to_string())
        );
    }

    #[test]
    fn optional_absent_returns_none() {
        assert_eq!(resolve_optional(None), None);
    }

    #[test]
    fn optional_blank_returns_none() {
        assert_eq!(resolve_optional(Some("   ".to_string())), None);
    }
}
