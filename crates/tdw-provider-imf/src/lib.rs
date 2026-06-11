#![forbid(unsafe_code)]
//! Offline types and validation for the IMF macro time-series provider
//! (OpenBB-parity item **P3W3**).
//!
//! The real HTTP fetcher lives in [`http_fetcher`] and is only compiled when the
//! `http` feature is enabled. Everything in this module works without any
//! network access. The IMF Data Services SDMX-JSON API is keyless; live calls
//! are additionally gated by `TDW_IMF_LIVE=1` in the integration tests.
//!
//! Each `economy/imf/*` command resolves (via [`catalog`]) to an IMF SDMX
//! `CompactData` database id; the caller supplies the dot-separated SDMX `key`
//! (`Frequency.RefArea.Indicator`, e.g. `M.US.PMP_IX`) that selects the concrete
//! series within that database. Observations are normalized to
//! [`tdw_domain::MacroSeries`] at the fetcher boundary — no raw SDMX shapes leak.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::ImfHttpMacroSeriesFetcher;

pub use catalog::{ENDPOINTS, ImfEndpoint};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the IMF Data Services SDMX-JSON REST API.
///
/// Uses TLS (`https`): the caller-supplied SDMX `key` and the requested series
/// travel over the wire, so the transport is encrypted even though the API is
/// keyless. The shared `build_client` enforces certificate validation.
pub const BASE_URL: &str = "https://dataservices.imf.org/REST/SDMX_JSON.svc";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "imf";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, ImfProviderError>;

/// Query for a standardized catalog-backed IMF `CompactData` series.
///
/// The caller supplies an `economy/imf/*` `command` path (e.g.
/// `"economy/imf/international_financial_statistics"`); the query resolves it
/// against [`catalog`] to the concrete SDMX database id and carries the
/// dot-separated SDMX `key` (`Frequency.RefArea.Indicator`) plus the shared
/// `start_date`/`end_date`/`limit` normalization (the date years drive the
/// `startPeriod`/`endPeriod` request parameters).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ImfCompactDataQuery {
    /// The resolved `economy/imf/*` command path.
    pub command: String,
    /// The IMF SDMX `CompactData` database id the command resolves to.
    pub database: String,
    /// Dot-separated SDMX dimension key (`Frequency.RefArea.Indicator`).
    pub key: String,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl ImfCompactDataQuery {
    /// Resolve a catalog command to a query for a given SDMX `key`.
    ///
    /// # Errors
    ///
    /// Returns [`ImfProviderError::UnknownCommand`] when `command` is not in the
    /// standardized [`catalog::ENDPOINTS`], or [`ImfProviderError::EmptyKey`] /
    /// [`ImfProviderError::InvalidKey`] when `key` is blank or contains
    /// characters outside the SDMX key grammar.
    pub fn new(command: &str, key: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| ImfProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            database: entry.database.to_string(),
            key: validate_key(key)?,
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog, reading the required `key` filter, and normalizing the shared
    /// `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing,
    /// unknown, `key` is missing/blank/invalid, or the shared parameters fail
    /// validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("imf command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                ImfProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        let key = params
            .get("key")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tdw_core::Error::InvalidQuery("imf key must be a string".to_string()))?;
        let key =
            validate_key(key).map_err(|error| tdw_core::Error::InvalidQuery(error.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            database: entry.database.to_string(),
            key,
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`ImfCompactDataQuery::new`] or
    /// [`ImfCompactDataQuery::from_value`], since both validate `command`
    /// against the catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static ImfEndpoint {
        catalog::resolve(&self.command)
            .expect("ImfCompactDataQuery::command is validated against the catalog at construction")
    }
}

/// Validate an SDMX dimension `key` such as `M.US.PMP_IX`.
///
/// The key sits on the request path between the database id and the query
/// string, so it is constrained to the SDMX key grammar — ASCII alphanumerics
/// plus the structural characters IMF uses: `.` (dimension separator), `+` (OR
/// within a dimension), `_`, and `-`. This excludes `/`, `?`, `&`, and `#`,
/// closing a path/query-injection gap.
fn validate_key(key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        return Err(ImfProviderError::EmptyKey);
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
    {
        return Err(ImfProviderError::InvalidKey);
    }
    Ok(key.to_string())
}

/// Errors produced by `tdw-provider-imf`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImfProviderError {
    /// The supplied `command` is not in the standardized endpoint catalog.
    #[error("imf command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
    /// The SDMX `key` was blank.
    #[error("imf key must not be empty")]
    EmptyKey,
    /// The SDMX `key` contained characters outside the SDMX key grammar.
    #[error("imf key contains characters outside the SDMX key grammar")]
    InvalidKey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_uses_tls() {
        // The caller-supplied SDMX key travels on the request path; the
        // transport must be encrypted. Guards against a plaintext regression.
        assert!(
            BASE_URL.starts_with("https://"),
            "IMF BASE_URL must use TLS, got {BASE_URL:?}"
        );
    }

    #[test]
    fn catalog_query_resolves_command_to_database() {
        let query = ImfCompactDataQuery::new(
            "economy/imf/international_financial_statistics",
            "M.US.PMP_IX",
        )
        .unwrap_or_else(|error| panic!("ifs query should build: {error}"));
        assert_eq!(query.database, "IFS");
        assert_eq!(query.key, "M.US.PMP_IX");
        assert_eq!(
            query.endpoint().command,
            "economy/imf/international_financial_statistics"
        );

        assert_eq!(
            ImfCompactDataQuery::new("not/a/command", "M.US.PMP_IX"),
            Err(ImfProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn new_rejects_blank_and_invalid_keys() {
        assert_eq!(
            ImfCompactDataQuery::new("economy/imf/direction_of_trade", "  "),
            Err(ImfProviderError::EmptyKey)
        );
        assert_eq!(
            ImfCompactDataQuery::new("economy/imf/direction_of_trade", "M.US/../x"),
            Err(ImfProviderError::InvalidKey)
        );
        // Legitimate SDMX dimension-key notation parses (and trims).
        let query =
            ImfCompactDataQuery::new("economy/imf/direction_of_trade", "  A.US+GB.TXG_FOB_USD  ")
                .unwrap_or_else(|error| panic!("valid key should build: {error}"));
        assert_eq!(query.key, "A.US+GB.TXG_FOB_USD");
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = ImfCompactDataQuery::from_value(&serde_json::json!({
            "command": "economy/imf/balance_of_payments",
            "key": "Q.US.BCA_BP6_USD",
            "start_date": "2020-01-01",
            "end_date": "2024-12-31",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.database, "BOP");
        assert_eq!(query.key, "Q.US.BCA_BP6_USD");
        assert_eq!(query.params.limit, Some(10));

        // Missing command / key / unknown command all error.
        assert!(
            ImfCompactDataQuery::from_value(&serde_json::json!({ "key": "M.US.PMP_IX" })).is_err()
        );
        assert!(
            ImfCompactDataQuery::from_value(&serde_json::json!({
                "command": "economy/imf/balance_of_payments"
            }))
            .is_err()
        );
        assert!(
            ImfCompactDataQuery::from_value(&serde_json::json!({
                "command": "bogus",
                "key": "M.US.PMP_IX"
            }))
            .is_err()
        );
        // Inverted date range is rejected by the shared params validation.
        assert!(
            ImfCompactDataQuery::from_value(&serde_json::json!({
                "command": "economy/imf/balance_of_payments",
                "key": "Q.US.BCA_BP6_USD",
                "start_date": "2024-01-01",
                "end_date": "2020-01-01"
            }))
            .is_err()
        );
    }
}
