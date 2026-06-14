#![forbid(unsafe_code)]
//! Offline types and validation for the keyed `Intrinio` provider
//! (`OpenBB`-parity total wave **G002**).
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access. `Intrinio` is a PAID API: every live request carries the
//! `api_key` query parameter read from `INTRINIO_API_KEY`. The crate ships and
//! compiles structurally without a key; the live integration tests skip when the
//! key is absent, so this is true code-level parity for the paid block — the
//! data lights up when a key is supplied.
//!
//! Clean-room: every route template and field is taken from `Intrinio`'s OWN
//! public API docs at <https://docs.intrinio.com> / <https://api-v2.intrinio.com>.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    IntrinioHttpForwardPeFetcher, IntrinioHttpForwardSalesFetcher,
    IntrinioHttpHistoricalAttributesFetcher, IntrinioHttpLatestAttributesFetcher,
    IntrinioHttpOptionsSnapshotsFetcher, IntrinioHttpOptionsSurfaceFetcher,
    IntrinioHttpOptionsUnusualFetcher, IntrinioHttpReportedFinancialsFetcher,
    IntrinioHttpSearchAttributesFetcher,
};

pub use catalog::{ENDPOINTS, IntrinioEndpoint, IntrinioModel};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the `Intrinio` v2 REST API.
pub const BASE_URL: &str = "https://api-v2.intrinio.com";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "intrinio";
/// Environment variable carrying the PAID `Intrinio` `api_key`.
pub const API_KEY_ENV: &str = "INTRINIO_API_KEY";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, IntrinioProviderError>;

/// Query for a standardized catalog-backed `Intrinio` endpoint.
///
/// The caller supplies an `OpenBB`-style `command` path (e.g.
/// `"derivatives/options/unusual"`); the query resolves it against [`catalog`]
/// and carries the optional `identifier` (a company ticker / `Intrinio` id or an
/// underlying option symbol), the optional `tag` (a standardized data-point tag
/// or a fundamental id, per route), and the shared
/// `start_date`/`end_date`/`limit` normalization. Which fields a route requires
/// is enforced by that route's fetcher, keeping one query shape across the
/// provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IntrinioQuery {
    /// The resolved `OpenBB`-style command path.
    pub command: String,
    /// The short dispatch endpoint key the command resolves to.
    pub endpoint: String,
    /// Company ticker / `Intrinio` identifier, underlying option symbol, or a
    /// fundamental id — per route. Required by most routes; unused by the
    /// metadata `search_attributes` route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    /// Standardized data-point tag, expiration date, or free-text search query —
    /// per route. Required by the attribute routes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl IntrinioQuery {
    /// Resolve a catalog command to a query with no extra filters.
    ///
    /// # Errors
    ///
    /// Returns [`IntrinioProviderError::UnknownCommand`] when `command` is not in
    /// the standardized [`catalog::ENDPOINTS`].
    pub fn new(command: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| IntrinioProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            endpoint: entry.endpoint.to_string(),
            identifier: None,
            tag: None,
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog, reading the optional `identifier`/`tag` filters (also accepting
    /// the aliases `symbol` and `query`), and normalizing the shared
    /// `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing,
    /// unknown, `identifier`/`tag` is not a string, or the shared parameters fail
    /// validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("intrinio command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                IntrinioProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        let identifier = optional_string(params, "identifier", "symbol")?;
        let tag = optional_string(params, "tag", "query")?;
        Ok(Self {
            command: entry.command.to_string(),
            endpoint: entry.endpoint.to_string(),
            identifier,
            tag,
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`IntrinioQuery::new`] or
    /// [`IntrinioQuery::from_value`], since both validate `command` against the
    /// catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static IntrinioEndpoint {
        catalog::resolve(&self.command)
            .expect("IntrinioQuery::command is validated against the catalog at construction")
    }
}

/// Read an optional string field under `key`, falling back to `alias`, treating
/// an absent / `null` / blank value as `None` and a non-string as an error.
fn optional_string(
    params: &serde_json::Value,
    key: &str,
    alias: &str,
) -> std::result::Result<Option<String>, tdw_core::Error> {
    for name in [key, alias] {
        match params.get(name) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(value)) => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
            Some(_) => {
                return Err(tdw_core::Error::InvalidQuery(format!(
                    "intrinio {key} must be a string"
                )));
            }
        }
    }
    Ok(None)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IntrinioProviderError {
    #[error("intrinio command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_uses_tls() {
        assert!(
            BASE_URL.starts_with("https://"),
            "intrinio base URL must use TLS: {BASE_URL}"
        );
    }

    #[test]
    fn catalog_query_resolves_command_to_endpoint() {
        let query = IntrinioQuery::new("derivatives/options/unusual")
            .unwrap_or_else(|error| panic!("unusual query should build: {error}"));
        assert_eq!(query.endpoint, "derivatives_options_unusual");
        assert_eq!(query.endpoint().model, IntrinioModel::OptionContract);

        assert_eq!(
            IntrinioQuery::new("not/a/command"),
            Err(IntrinioProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = IntrinioQuery::from_value(&serde_json::json!({
            "command": "equity/fundamental/historical_attributes",
            "symbol": "AAPL",
            "tag": "marketcap",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.identifier.as_deref(), Some("AAPL"));
        assert_eq!(query.tag.as_deref(), Some("marketcap"));
        assert_eq!(query.params.limit, Some(10));
        assert_eq!(
            query.endpoint().model,
            IntrinioModel::CompanyAttribute,
            "historical_attributes standardizes to CompanyAttribute"
        );

        assert!(IntrinioQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(IntrinioQuery::from_value(&serde_json::json!({})).is_err());
        assert!(
            IntrinioQuery::from_value(&serde_json::json!({
                "command": "equity/fundamental/latest_attributes",
                "identifier": 7
            }))
            .is_err()
        );
    }

    #[test]
    fn blank_filters_normalize_to_none_and_query_alias_maps_to_tag() {
        let query = IntrinioQuery::from_value(&serde_json::json!({
            "command": "equity/fundamental/search_attributes",
            "identifier": "  ",
            "query": "  pe  "
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert!(query.identifier.is_none());
        assert_eq!(query.tag.as_deref(), Some("pe"));
        assert_eq!(query.endpoint().model, IntrinioModel::CompanyAttribute);
    }

    #[test]
    fn provider_id_and_key_env_are_stable() {
        assert_eq!(PROVIDER_ID, "intrinio");
        assert_eq!(API_KEY_ENV, "INTRINIO_API_KEY");
    }
}
