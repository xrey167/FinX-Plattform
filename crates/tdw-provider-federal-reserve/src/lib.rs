#![forbid(unsafe_code)]
//! Offline types and validation for the Federal Reserve data provider
//! (gap-matrix item **L3.1**).
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access. The Fed data portals are keyless.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{FedFomcDocumentsHttpFetcher, FedMacroSeriesHttpFetcher};

pub use catalog::{ENDPOINTS, FedEndpoint, FedModel};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the Federal Reserve Board data portal.
pub const BASE_URL: &str = "https://www.federalreserve.gov";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "federal_reserve";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, FedProviderError>;

/// Query for a standardized catalog-backed Federal Reserve endpoint.
///
/// The caller supplies an `OpenBB` `command` path (e.g.
/// `"economy/money_measures"`); the query resolves it against [`catalog`] and
/// carries the shared `start_date`/`end_date`/`limit` normalization. Used by
/// both the macro-series fetcher and the FOMC document-index fetcher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FedCatalogQuery {
    /// The resolved `OpenBB` command path.
    pub command: String,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl FedCatalogQuery {
    /// Resolve a catalog command to a query.
    ///
    /// # Errors
    ///
    /// Returns [`FedProviderError::UnknownCommand`] when `command` is not in the
    /// standardized [`catalog::ENDPOINTS`].
    pub fn new(command: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| FedProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog and normalizing the shared `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing,
    /// unknown, or the shared parameters fail validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery(
                    "federal_reserve command must be a string".to_string(),
                )
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                FedProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        Ok(Self {
            command: entry.command.to_string(),
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`FedCatalogQuery::new`] or
    /// [`FedCatalogQuery::from_value`], since both validate `command` against the
    /// catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static FedEndpoint {
        catalog::resolve(&self.command)
            .expect("FedCatalogQuery::command is validated against the catalog at construction")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FedProviderError {
    #[error("federal_reserve command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_query_resolves_known_commands() {
        let money = FedCatalogQuery::new("economy/money_measures")
            .unwrap_or_else(|error| panic!("money query should build: {error}"));
        assert_eq!(money.endpoint().model, FedModel::Macro);

        let fomc = FedCatalogQuery::new("regulators/fed/fomc_documents")
            .unwrap_or_else(|error| panic!("fomc query should build: {error}"));
        assert_eq!(fomc.endpoint().model, FedModel::FomcDocuments);

        assert_eq!(
            FedCatalogQuery::new("not/a/command"),
            Err(FedProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = FedCatalogQuery::from_value(&serde_json::json!({
            "command": "fixedincome/government/dealer_stats",
            "limit": 7
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.command, "fixedincome/government/dealer_stats");
        assert_eq!(query.params.limit, Some(7));

        assert!(FedCatalogQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(FedCatalogQuery::from_value(&serde_json::json!({})).is_err());
    }
}
