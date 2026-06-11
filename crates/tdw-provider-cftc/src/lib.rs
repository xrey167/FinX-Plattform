#![forbid(unsafe_code)]
//! Offline types and validation for the CFTC Commitments of Traders provider
//! (OpenBB-parity item **P2W5**).
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access. The CFTC Socrata API is keyless; an optional
//! `X-App-Token` (read from `TDW_CFTC_APP_TOKEN`) raises the rate limit.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{CftcHttpCotFetcher, CftcHttpCotSearchFetcher};

pub use catalog::{CftcEndpoint, CftcModel, ENDPOINTS};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the CFTC public-reporting Socrata API.
pub const BASE_URL: &str = "https://publicreporting.cftc.gov";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "cftc";
/// Environment variable carrying the optional Socrata `X-App-Token`.
pub const APP_TOKEN_ENV: &str = "TDW_CFTC_APP_TOKEN";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, CftcProviderError>;

/// Query for a standardized catalog-backed CFTC endpoint.
///
/// The caller supplies an `OpenBB` `command` path (e.g. `"regulators/cftc/cot"`);
/// the query resolves it against [`catalog`] and carries the COT `market` filter
/// plus the shared `start_date`/`end_date`/`limit` normalization. The
/// `market` is required for the COT series and ignored by the market-discovery
/// (`cot_search`) endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CftcCotQuery {
    /// The resolved `OpenBB` command path.
    pub command: String,
    /// The Socrata dataset resource path the command resolves to.
    pub dataset: String,
    /// `market_and_exchange_names` filter for the COT series. Required by the
    /// `regulators/cftc/cot` series fetch; unused by `cot_search`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl CftcCotQuery {
    /// Resolve a catalog command to a query.
    ///
    /// # Errors
    ///
    /// Returns [`CftcProviderError::UnknownCommand`] when `command` is not in the
    /// standardized [`catalog::ENDPOINTS`].
    pub fn new(command: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| CftcProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            dataset: entry.dataset.to_string(),
            market: None,
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog, reading the optional `market` filter, and normalizing the shared
    /// `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing,
    /// unknown, `market` is not a string, or the shared parameters fail
    /// validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("cftc command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                CftcProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        let market = match params.get("market") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(value)) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            Some(_) => {
                return Err(tdw_core::Error::InvalidQuery(
                    "cftc market must be a string".to_string(),
                ));
            }
        };
        Ok(Self {
            command: entry.command.to_string(),
            dataset: entry.dataset.to_string(),
            market,
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`CftcCotQuery::new`] or
    /// [`CftcCotQuery::from_value`], since both validate `command` against the
    /// catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static CftcEndpoint {
        catalog::resolve(&self.command)
            .expect("CftcCotQuery::command is validated against the catalog at construction")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CftcProviderError {
    #[error("cftc command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_query_resolves_command_to_dataset() {
        let query = CftcCotQuery::new("regulators/cftc/cot")
            .unwrap_or_else(|error| panic!("cot query should build: {error}"));
        assert_eq!(query.dataset, "resource/6dca-aqww.json");
        assert_eq!(query.endpoint().model, CftcModel::Cot);

        assert_eq!(
            CftcCotQuery::new("not/a/command"),
            Err(CftcProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = CftcCotQuery::from_value(&serde_json::json!({
            "command": "regulators/cftc/cot",
            "market": "WHEAT-SRW - CHICAGO BOARD OF TRADE",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.dataset, "resource/6dca-aqww.json");
        assert_eq!(
            query.market.as_deref(),
            Some("WHEAT-SRW - CHICAGO BOARD OF TRADE")
        );
        assert_eq!(query.params.limit, Some(10));

        assert!(CftcCotQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(CftcCotQuery::from_value(&serde_json::json!({})).is_err());
        assert!(
            CftcCotQuery::from_value(&serde_json::json!({
                "command": "regulators/cftc/cot",
                "market": 7
            }))
            .is_err()
        );
    }

    #[test]
    fn blank_market_normalizes_to_none() {
        let query = CftcCotQuery::from_value(&serde_json::json!({
            "command": "regulators/cftc/cot_search",
            "market": "  "
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert!(query.market.is_none());
        assert_eq!(query.endpoint().model, CftcModel::CotSearch);
    }
}
