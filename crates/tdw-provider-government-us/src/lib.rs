#![forbid(unsafe_code)]
//! Offline types and validation for the US Treasury `FiscalData` provider
//! (gap-matrix item **L3.2**).
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access. The `FiscalData` API is keyless.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{GovUsTreasuryAuctionsHttpFetcher, GovUsTreasuryPricesHttpFetcher};

pub use catalog::{ENDPOINTS, GovUsEndpoint, GovUsModel};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the US Treasury `FiscalData` API.
pub const BASE_URL: &str = "https://api.fiscaldata.treasury.gov/services/api/fiscal_service";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "government_us";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, GovUsProviderError>;

/// Query for a standardized catalog-backed Treasury endpoint.
///
/// The caller supplies an `OpenBB` `command` path (e.g.
/// `"fixedincome/government/treasury_auctions"`); the query resolves it against
/// [`catalog`] to the concrete `FiscalData` dataset path and carries the shared
/// `start_date`/`end_date`/`limit` normalization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GovUsCatalogQuery {
    /// The resolved `OpenBB` command path.
    pub command: String,
    /// The `FiscalData` dataset path the command resolves to.
    pub dataset: String,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl GovUsCatalogQuery {
    /// Resolve a catalog command to a query.
    ///
    /// # Errors
    ///
    /// Returns [`GovUsProviderError::UnknownCommand`] when `command` is not in
    /// the standardized [`catalog::ENDPOINTS`].
    pub fn new(command: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| GovUsProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            dataset: entry.dataset.to_string(),
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
                tdw_core::Error::InvalidQuery("government_us command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                GovUsProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        Ok(Self {
            command: entry.command.to_string(),
            dataset: entry.dataset.to_string(),
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`GovUsCatalogQuery::new`] or
    /// [`GovUsCatalogQuery::from_value`], since both validate `command` against
    /// the catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static GovUsEndpoint {
        catalog::resolve(&self.command)
            .expect("GovUsCatalogQuery::command is validated against the catalog at construction")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GovUsProviderError {
    #[error("government_us command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_query_resolves_command_to_dataset() {
        let query = GovUsCatalogQuery::new("fixedincome/government/treasury_auctions")
            .unwrap_or_else(|error| panic!("auctions query should build: {error}"));
        assert_eq!(query.dataset, "v1/accounting/od/auctions_query");
        assert_eq!(query.endpoint().model, GovUsModel::Auction);

        assert_eq!(
            GovUsCatalogQuery::new("not/a/command"),
            Err(GovUsProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = GovUsCatalogQuery::from_value(&serde_json::json!({
            "command": "fixedincome/government/treasury_prices",
            "start_date": "2024-01-01",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.dataset, "v1/accounting/od/securities_sales");
        assert_eq!(query.params.limit, Some(10));

        assert!(GovUsCatalogQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(GovUsCatalogQuery::from_value(&serde_json::json!({})).is_err());
    }
}
