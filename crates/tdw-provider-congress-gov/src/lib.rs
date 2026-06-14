#![forbid(unsafe_code)]
//! Offline types and validation for the US Congress legislative-data provider
//! (`congress.gov`), OpenBB-parity item **P4W12**.
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled when
//! the `http` feature is enabled. Everything in this module works without any
//! network access. The `congress.gov` v3 API requires a free api.data.gov key
//! (read from `CONGRESS_GOV_API_KEY`); requests pass it as the `api_key` query
//! parameter alongside `format=json`.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    CongressGovBillInfoFetcher, CongressGovBillTextFetcher, CongressGovBillsFetcher,
};

pub use catalog::{CongressEndpoint, CongressModel, ENDPOINTS};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the `congress.gov` v3 API.
pub const BASE_URL: &str = "https://api.congress.gov/v3";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "congress-gov";
/// Environment variable carrying the free api.data.gov API key.
pub const API_KEY_ENV: &str = "CONGRESS_GOV_API_KEY";

/// The bill types accepted by the `congress.gov` `/bill/{congress}/{billType}`
/// route, lowercased as the API expects them on the path.
pub const BILL_TYPES: &[&str] = &[
    "hr", "s", "hjres", "sjres", "hconres", "sconres", "hres", "sres",
];

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, CongressGovProviderError>;

/// Errors produced by the `congress.gov` provider's offline layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CongressGovProviderError {
    /// The supplied `command` is not in the standardized endpoint catalog.
    #[error("congress-gov command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
    /// A required path parameter was missing or invalid.
    #[error("congress-gov {0}")]
    InvalidQuery(String),
}

/// Query for a standardized catalog-backed `congress.gov` endpoint.
///
/// The caller supplies an `OpenBB` `command` path (e.g. `"uscongress/bills"`);
/// the query resolves it against [`catalog`] and carries the `congress`,
/// `bill_type`, and `bill_number` path parameters plus the shared
/// `start_date`/`end_date`/`limit` normalization. `bill_type` is required for
/// every route; `bill_number` is required by `bill_info` and `bill_text_urls`
/// and ignored by the `bills` list route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CongressBillQuery {
    /// The resolved `OpenBB` command path.
    pub command: String,
    /// Congress number, e.g. `118`.
    pub congress: u32,
    /// Bill type on the path, e.g. `"hr"`, `"s"` (lowercased and validated).
    pub bill_type: String,
    /// Bill number for the detail/text routes; `None` for the list route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bill_number: Option<String>,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl CongressBillQuery {
    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog, reading the `congress`/`bill_type`/`bill_number` path params,
    /// and normalizing the shared `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing or
    /// unknown, `congress` is missing/non-numeric, `bill_type` is missing or
    /// unsupported, the route requires `bill_number` and it is absent, or the
    /// shared parameters fail validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("congress-gov command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                CongressGovProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;

        let congress = read_congress(params)?;
        let bill_type = read_bill_type(params)?;
        let bill_number = read_bill_number(params)?;
        if entry.requires_bill_number && bill_number.is_none() {
            return Err(tdw_core::Error::InvalidQuery(format!(
                "congress-gov command {command:?} requires a bill_number"
            )));
        }

        Ok(Self {
            command: entry.command.to_string(),
            congress,
            bill_type,
            bill_number,
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`CongressBillQuery::from_value`],
    /// which validates `command` against the catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static CongressEndpoint {
        catalog::resolve(&self.command)
            .expect("CongressBillQuery::command is validated against the catalog at construction")
    }
}

fn read_congress(params: &serde_json::Value) -> std::result::Result<u32, tdw_core::Error> {
    match params.get("congress") {
        Some(serde_json::Value::Number(number)) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("congress-gov congress out of range".to_string())
            }),
        Some(serde_json::Value::String(text)) => text.trim().parse::<u32>().map_err(|_| {
            tdw_core::Error::InvalidQuery("congress-gov congress must be a number".to_string())
        }),
        _ => Err(tdw_core::Error::InvalidQuery(
            "congress-gov congress must be a number".to_string(),
        )),
    }
}

fn read_bill_type(params: &serde_json::Value) -> std::result::Result<String, tdw_core::Error> {
    let raw = params
        .get("bill_type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            tdw_core::Error::InvalidQuery("congress-gov bill_type must be a string".to_string())
        })?;
    let normalized = raw.trim().to_ascii_lowercase();
    if BILL_TYPES.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(tdw_core::Error::InvalidQuery(format!(
            "congress-gov bill_type {raw:?} is not one of {BILL_TYPES:?}"
        )))
    }
}

fn read_bill_number(
    params: &serde_json::Value,
) -> std::result::Result<Option<String>, tdw_core::Error> {
    match params.get("bill_number") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else if trimmed.chars().all(|c| c.is_ascii_digit()) {
                Ok(Some(trimmed.to_string()))
            } else {
                Err(tdw_core::Error::InvalidQuery(
                    "congress-gov bill_number must be numeric".to_string(),
                ))
            }
        }
        Some(serde_json::Value::Number(number)) => Ok(Some(number.to_string())),
        Some(_) => Err(tdw_core::Error::InvalidQuery(
            "congress-gov bill_number must be a string or number".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bills_query_resolves_and_normalizes() {
        let query = CongressBillQuery::from_value(&serde_json::json!({
            "command": "uscongress/bills",
            "congress": 118,
            "bill_type": "HR",
            "limit": 5
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.command, "uscongress/bills");
        assert_eq!(query.congress, 118);
        // bill_type is lowercased for the API path.
        assert_eq!(query.bill_type, "hr");
        assert!(query.bill_number.is_none());
        assert_eq!(query.params.limit, Some(5));
        assert_eq!(query.endpoint().model, CongressModel::Bill);
    }

    #[test]
    fn bill_info_requires_bill_number() {
        assert!(
            CongressBillQuery::from_value(&serde_json::json!({
                "command": "uscongress/bill_info",
                "congress": 118,
                "bill_type": "hr"
            }))
            .is_err()
        );
        let query = CongressBillQuery::from_value(&serde_json::json!({
            "command": "uscongress/bill_info",
            "congress": 118,
            "bill_type": "hr",
            "bill_number": "3076"
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.bill_number.as_deref(), Some("3076"));
        assert_eq!(query.endpoint().model, CongressModel::Bill);
    }

    #[test]
    fn bill_text_urls_resolves_to_text_model() {
        let query = CongressBillQuery::from_value(&serde_json::json!({
            "command": "uscongress/bill_text_urls",
            "congress": 117,
            "bill_type": "s",
            "bill_number": "1"
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.endpoint().model, CongressModel::TextUrl);
    }

    #[test]
    fn rejects_unknown_command_and_bad_inputs() {
        assert!(CongressBillQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(CongressBillQuery::from_value(&serde_json::json!({})).is_err());
        // Unsupported bill type.
        assert!(
            CongressBillQuery::from_value(&serde_json::json!({
                "command": "uscongress/bills",
                "congress": 118,
                "bill_type": "xyz"
            }))
            .is_err()
        );
        // Non-numeric bill number.
        assert!(
            CongressBillQuery::from_value(&serde_json::json!({
                "command": "uscongress/bill_info",
                "congress": 118,
                "bill_type": "hr",
                "bill_number": "abc"
            }))
            .is_err()
        );
    }
}
