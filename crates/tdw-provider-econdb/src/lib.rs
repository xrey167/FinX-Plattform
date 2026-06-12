#![forbid(unsafe_code)]
//! Offline types and validation for the `EconDB` economic time-series provider
//! (`OpenBB`-parity item **P3W4**).
//!
//! The real HTTP fetcher lives in [`http_fetcher`] and is only compiled when the
//! `http` feature is enabled. Everything in this module works without any
//! network access. The `EconDB` public series API is keyless for public series;
//! an optional free token (read from `TDW_ECONDB_API_KEY`) raises limits /
//! unlocks series. Live calls are additionally gated by `TDW_ECONDB_LIVE=1` in
//! the integration tests.
//!
//! The single `economy/econdb/series` command resolves (via [`catalog`]) to the
//! `EconDB` series endpoint; the caller supplies the `EconDB` `ticker` (e.g.
//! `RGDPUS` = real GDP US) that selects the concrete series. Observations are
//! normalized to [`tdw_domain::MacroSeries`] at the fetcher boundary — no raw
//! `EconDB` shapes leak.

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::EcondbHttpMacroSeriesFetcher;

pub use catalog::{ENDPOINTS, EcondbEndpoint};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

/// Public base URL for the `EconDB` public API.
///
/// Uses TLS (`https`): the caller-supplied `ticker` and (optional) token travel
/// over the wire, so the transport is encrypted even though public series are
/// keyless. The shared `build_client` enforces certificate validation.
pub const BASE_URL: &str = "https://www.econdb.com/api";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "econdb";
/// Environment variable carrying the optional `EconDB` API token.
pub const API_KEY_ENV: &str = "TDW_ECONDB_API_KEY";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, EcondbProviderError>;

/// Query for a standardized catalog-backed `EconDB` series.
///
/// The caller supplies an `economy/econdb/*` `command` path (currently only
/// `"economy/econdb/series"`); the query resolves it against [`catalog`] and
/// carries the `EconDB` `ticker` plus the shared `start_date`/`end_date`/`limit`
/// normalization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EcondbSeriesQuery {
    /// The resolved `economy/econdb/*` command path.
    pub command: String,
    /// The `EconDB` series ticker (e.g. `RGDPUS`).
    pub ticker: String,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl EcondbSeriesQuery {
    /// Resolve a catalog command to a query for a given `EconDB` `ticker`.
    ///
    /// # Errors
    ///
    /// Returns [`EcondbProviderError::UnknownCommand`] when `command` is not in
    /// the standardized [`catalog::ENDPOINTS`], or
    /// [`EcondbProviderError::EmptyTicker`] / [`EcondbProviderError::InvalidTicker`]
    /// when `ticker` is blank or contains characters outside the ticker grammar.
    pub fn new(command: &str, ticker: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| EcondbProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            ticker: validate_ticker(ticker)?,
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, resolving `command` against the
    /// catalog, reading the required `ticker` filter, and normalizing the shared
    /// `start_date`/`end_date`/`limit`.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when `command` is missing,
    /// unknown, `ticker` is missing/blank/invalid, or the shared parameters fail
    /// validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let command = params
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("econdb command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                EcondbProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        let ticker = params
            .get("ticker")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("econdb ticker must be a string".to_string())
            })?;
        let ticker = validate_ticker(ticker)
            .map_err(|error| tdw_core::Error::InvalidQuery(error.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            ticker,
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`EcondbSeriesQuery::new`] or
    /// [`EcondbSeriesQuery::from_value`], since both validate `command` against
    /// the catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static EcondbEndpoint {
        catalog::resolve(&self.command)
            .expect("EcondbSeriesQuery::command is validated against the catalog at construction")
    }
}

/// Validate an `EconDB` `ticker` such as `RGDPUS`.
///
/// The ticker sits on the request path between the API base and the query
/// string (`series/{TICKER}/`), so it is constrained to a safe grammar — ASCII
/// alphanumerics plus the structural characters `EconDB` tickers use: `_`, `-`,
/// and `.`. This excludes `/`, `?`, `&`, and `#`, closing a path/query-injection
/// gap on the `{TICKER}` path segment.
fn validate_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(EcondbProviderError::EmptyTicker);
    }
    if !ticker
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(EcondbProviderError::InvalidTicker);
    }
    Ok(ticker.to_string())
}

/// Errors produced by `tdw-provider-econdb`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EcondbProviderError {
    /// The supplied `command` is not in the standardized endpoint catalog.
    #[error("econdb command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
    /// The `EconDB` `ticker` was blank.
    #[error("econdb ticker must not be empty")]
    EmptyTicker,
    /// The `EconDB` `ticker` contained characters outside the ticker grammar.
    #[error("econdb ticker contains characters outside the ticker grammar")]
    InvalidTicker,
}

#[cfg(test)]
mod tests {
    use super::{BASE_URL, EcondbProviderError, EcondbSeriesQuery};

    #[test]
    fn base_url_uses_tls() {
        // The caller-supplied ticker travels on the request path; the transport
        // must be encrypted. Guards against a plaintext regression.
        assert!(
            BASE_URL.starts_with("https://"),
            "EconDB BASE_URL must use TLS, got {BASE_URL:?}"
        );
    }

    #[test]
    fn catalog_query_resolves_command() {
        let query = EcondbSeriesQuery::new("economy/econdb/series", "RGDPUS")
            .unwrap_or_else(|error| panic!("series query should build: {error}"));
        assert_eq!(query.ticker, "RGDPUS");
        assert_eq!(query.endpoint().command, "economy/econdb/series");

        assert_eq!(
            EcondbSeriesQuery::new("not/a/command", "RGDPUS"),
            Err(EcondbProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn new_rejects_blank_and_invalid_tickers() {
        assert_eq!(
            EcondbSeriesQuery::new("economy/econdb/series", "  "),
            Err(EcondbProviderError::EmptyTicker)
        );
        assert_eq!(
            EcondbSeriesQuery::new("economy/econdb/series", "RGDP/../x"),
            Err(EcondbProviderError::InvalidTicker)
        );
        // Legitimate ticker notation parses (and trims).
        let query = EcondbSeriesQuery::new("economy/econdb/series", "  GDP.US-2024_Q1  ")
            .unwrap_or_else(|error| panic!("valid ticker should build: {error}"));
        assert_eq!(query.ticker, "GDP.US-2024_Q1");
    }

    #[test]
    fn from_value_resolves_and_normalizes_params() {
        let query = EcondbSeriesQuery::from_value(&serde_json::json!({
            "command": "economy/econdb/series",
            "ticker": "RGDPUS",
            "start_date": "2020-01-01",
            "end_date": "2024-12-31",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));
        assert_eq!(query.ticker, "RGDPUS");
        assert_eq!(query.params.limit, Some(10));

        // Missing command / ticker / unknown command all error.
        assert!(EcondbSeriesQuery::from_value(&serde_json::json!({ "ticker": "RGDPUS" })).is_err());
        assert!(
            EcondbSeriesQuery::from_value(&serde_json::json!({
                "command": "economy/econdb/series"
            }))
            .is_err()
        );
        assert!(
            EcondbSeriesQuery::from_value(&serde_json::json!({
                "command": "bogus",
                "ticker": "RGDPUS"
            }))
            .is_err()
        );
        // Inverted date range is rejected by the shared params validation.
        assert!(
            EcondbSeriesQuery::from_value(&serde_json::json!({
                "command": "economy/econdb/series",
                "ticker": "RGDPUS",
                "start_date": "2024-01-01",
                "end_date": "2020-01-01"
            }))
            .is_err()
        );
    }
}
