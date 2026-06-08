#![forbid(unsafe_code)]

pub mod catalog;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    FredHttpMacroSeriesFetcher, FredHttpRateObservationFetcher, FredHttpSeriesObservationsFetcher,
};

pub use catalog::{ENDPOINTS, FredEndpoint, FredModel};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::query_params::StandardParams;
use thiserror::Error;

pub const PROVIDER_ID: &str = "fred";
pub const BASE_URL: &str = "https://api.stlouisfed.org/fred";
pub const API_KEY_PARAM: &str = "api_key";

pub type Result<T> = std::result::Result<T, FredProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub credential_param: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_param: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FredSeriesObservationsQuery {
    pub series_id: String,
    /// Shared date/limit normalization parsed from the same caller payload as
    /// `series_id`. FRED maps `start_date`/`end_date`/`limit` onto its
    /// `observation_start`/`observation_end`/`limit` request parameters.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl FredSeriesObservationsQuery {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(series_id: &str) -> Result<Self> {
        Ok(Self {
            series_id: normalize_series_id(series_id)?,
            params: StandardParams::default(),
        })
    }

    /// Build a query from a raw caller payload, normalizing both the
    /// `series_id` and the shared `start_date`/`end_date`/`limit` parameters.
    ///
    /// # Errors
    ///
    /// Returns an error variant when the `series_id` is missing/invalid or the
    /// shared parameters fail validation.
    pub fn from_value(params: &serde_json::Value) -> std::result::Result<Self, tdw_core::Error> {
        let series_id = params
            .get("series_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                tdw_core::Error::InvalidQuery("fred series_id must be a string".to_string())
            })?;
        let series_id = normalize_series_id(series_id)
            .map_err(|error| tdw_core::Error::InvalidQuery(error.to_string()))?;
        Ok(Self {
            series_id,
            params: StandardParams::from_value(params)?,
        })
    }
}

/// Query for a standardized catalog-backed endpoint (macro or rate cluster).
///
/// The caller supplies an `OpenBB` `command` path (e.g. `"economy/cpi"` or
/// `"fixedincome/rate/sofr"`); the query resolves it against [`catalog`] to the
/// concrete FRED `series_id` and carries the shared `start_date`/`end_date`/
/// `limit` normalization. Used by both [`http_fetcher::FredHttpMacroSeriesFetcher`]
/// and [`http_fetcher::FredHttpRateObservationFetcher`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FredCatalogQuery {
    /// The resolved OpenBB command path.
    pub command: String,
    /// The FRED series id the command resolves to.
    pub series_id: String,
    /// Shared date/limit normalization parsed from the caller payload.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl FredCatalogQuery {
    /// Resolve a catalog command to a query.
    ///
    /// # Errors
    ///
    /// Returns [`FredProviderError::UnknownCommand`] when `command` is not in
    /// the standardized [`catalog::ENDPOINTS`].
    pub fn new(command: &str) -> Result<Self> {
        let entry = catalog::resolve(command)
            .ok_or_else(|| FredProviderError::UnknownCommand(command.to_string()))?;
        Ok(Self {
            command: entry.command.to_string(),
            series_id: entry.series_id.to_string(),
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
                tdw_core::Error::InvalidQuery("fred command must be a string".to_string())
            })?;
        let entry = catalog::resolve(command).ok_or_else(|| {
            tdw_core::Error::InvalidQuery(
                FredProviderError::UnknownCommand(command.to_string()).to_string(),
            )
        })?;
        Ok(Self {
            command: entry.command.to_string(),
            series_id: entry.series_id.to_string(),
            params: StandardParams::from_value(params)?,
        })
    }

    /// The resolved catalog entry for this query.
    ///
    /// # Panics
    ///
    /// Never panics for a query built via [`FredCatalogQuery::new`] or
    /// [`FredCatalogQuery::from_value`], since both validate `command` against
    /// the catalog at construction time.
    #[must_use]
    pub fn endpoint(&self) -> &'static FredEndpoint {
        catalog::resolve(&self.command)
            .expect("FredCatalogQuery::command is validated against the catalog at construction")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FredObservation {
    pub series_id: String,
    pub date: String,
    pub value: f64,
    pub realtime_start: String,
    pub realtime_end: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FredProviderError {
    #[error("fred series id must not be empty")]
    EmptySeriesId,
    #[error("fred series id contains unsupported characters")]
    InvalidSeriesId,
    #[error("fred api key must be supplied by the caller")]
    MissingApiKey,
    #[error("fred command {0:?} is not in the standardized endpoint catalog")]
    UnknownCommand(String),
    #[error("fred search text must not be empty")]
    EmptySearchText,
}

#[must_use]
pub const fn endpoints() -> &'static [ProviderEndpoint] {
    const fn entry(name: &'static str) -> ProviderEndpoint {
        ProviderEndpoint {
            provider: PROVIDER_ID,
            name,
            base_url: BASE_URL,
            credential_param: API_KEY_PARAM,
        }
    }
    const PROVIDER_ENDPOINTS: &[ProviderEndpoint] = &[
        entry("series_observations"),
        entry("macro_series"),
        entry("rate_observation"),
        entry("fred_search"),
        entry("fred_release"),
        entry("fred_regional"),
    ];
    PROVIDER_ENDPOINTS
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn series_observations_request(
    series_id: &str,
    api_key_present: bool,
) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(FredProviderError::MissingApiKey);
    }
    let series_id = normalize_series_id(series_id)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "series_observations",
        path: format!("/series/observations?series_id={series_id}&file_type=json"),
        credential_param: API_KEY_PARAM,
    })
}

/// Build the request for the FRED `series/search` metadata endpoint
/// (standardizes `economy/fred_search`).
///
/// # Errors
///
/// Returns [`FredProviderError::MissingApiKey`] when no key is present, or
/// [`FredProviderError::EmptySearchText`] when `search_text` is blank.
pub fn fred_search_request(search_text: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(FredProviderError::MissingApiKey);
    }
    let search_text = search_text.trim();
    if search_text.is_empty() {
        return Err(FredProviderError::EmptySearchText);
    }
    let encoded = encode_query_component(search_text);
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "fred_search",
        path: format!("/series/search?search_text={encoded}&file_type=json"),
        credential_param: API_KEY_PARAM,
    })
}

/// Build the request for the FRED `release/series` table endpoint
/// (standardizes `economy/fred_release_table`).
///
/// # Errors
///
/// Returns [`FredProviderError::MissingApiKey`] when no key is present.
pub fn fred_release_request(release_id: u32, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(FredProviderError::MissingApiKey);
    }
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "fred_release",
        path: format!("/release/series?release_id={release_id}&file_type=json"),
        credential_param: API_KEY_PARAM,
    })
}

/// Build the request for the FRED regional `series/observations` endpoint
/// (standardizes `economy/fred_regional`). Regional series are queried by the
/// same observation path as any other series id.
///
/// # Errors
///
/// Returns [`FredProviderError::MissingApiKey`] when no key is present, or an
/// id-validation error from [`normalize_series_id`].
pub fn fred_regional_request(series_id: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(FredProviderError::MissingApiKey);
    }
    let series_id = normalize_series_id(series_id)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "fred_regional",
        path: format!("/series/observations?series_id={series_id}&file_type=json"),
        credential_param: API_KEY_PARAM,
    })
}

/// Percent-encode the characters that would otherwise break out of a query
/// component (`&`, `=`, `?`, `#`, `+`, space). Kept dependency-free in keeping
/// with the crate's no-`url`-crate stance.
fn encode_query_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            other => {
                encoded.push('%');
                encoded.push(hex_digit(other >> 4));
                encoded.push(hex_digit(other & 0x0f));
            }
        }
    }
    encoded
}

const fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

fn normalize_series_id(series_id: &str) -> Result<String> {
    let series_id = series_id.trim();
    if series_id.is_empty() {
        return Err(FredProviderError::EmptySeriesId);
    }
    if !series_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.'))
    {
        return Err(FredProviderError::InvalidSeriesId);
    }
    Ok(series_id.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_series_observation_request_contract() {
        let request = series_observations_request("gdp", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(endpoints()[0].provider, "fred");
        assert!(request.path.contains("series_id=GDP"));
        assert_eq!(request.credential_param, API_KEY_PARAM);
        assert!(series_observations_request("GDP", false).is_err());
        assert!(series_observations_request("", true).is_err());
        assert!(series_observations_request("GDP&file_type=xml", true).is_err());
    }

    #[test]
    fn builds_series_observation_query_model() {
        let query = FredSeriesObservationsQuery::new("unrate")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.series_id, "UNRATE");
        assert_eq!(query.params, StandardParams::default());
        assert!(FredSeriesObservationsQuery::new(" ").is_err());
        assert!(FredSeriesObservationsQuery::new("GDP&limit=1").is_err());
    }

    #[test]
    fn from_value_normalizes_series_id_and_shared_params() {
        let query = FredSeriesObservationsQuery::from_value(&serde_json::json!({
            "series_id": "gdp",
            "start_date": "2020-01-01",
            "end_date": "2024-12-31",
            "limit": 25
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.series_id, "GDP");
        assert_eq!(
            query.params.start_date.map(|d| d.to_string()).as_deref(),
            Some("2020-01-01")
        );
        assert_eq!(
            query.params.end_date.map(|d| d.to_string()).as_deref(),
            Some("2024-12-31")
        );
        assert_eq!(query.params.limit, Some(25));

        // Shared validation rejects an inverted date range.
        assert!(
            FredSeriesObservationsQuery::from_value(&serde_json::json!({
                "series_id": "gdp",
                "start_date": "2024-01-01",
                "end_date": "2020-01-01"
            }))
            .is_err()
        );
        // Missing series_id is still an error.
        assert!(FredSeriesObservationsQuery::from_value(&serde_json::json!({})).is_err());
    }

    #[test]
    fn exposes_standardized_catalog_endpoints() {
        let names: Vec<&str> = endpoints().iter().map(|e| e.name).collect();
        assert!(names.contains(&"macro_series"));
        assert!(names.contains(&"rate_observation"));
        assert!(names.contains(&"fred_search"));
        assert!(names.contains(&"fred_release"));
        assert!(names.contains(&"fred_regional"));
    }

    #[test]
    fn catalog_query_resolves_command_to_series_id() {
        let query = FredCatalogQuery::new("economy/cpi")
            .unwrap_or_else(|error| panic!("cpi query should build: {error}"));
        assert_eq!(query.series_id, "CPIAUCSL");
        assert_eq!(query.endpoint().model, FredModel::Macro);

        let sofr = FredCatalogQuery::new("fixedincome/rate/sofr")
            .unwrap_or_else(|error| panic!("sofr query should build: {error}"));
        assert_eq!(sofr.series_id, "SOFR");
        assert_eq!(sofr.endpoint().model, FredModel::Rate);

        assert_eq!(
            FredCatalogQuery::new("not/a/command"),
            Err(FredProviderError::UnknownCommand(
                "not/a/command".to_string()
            ))
        );
    }

    #[test]
    fn catalog_query_from_value_resolves_and_normalizes_params() {
        let query = FredCatalogQuery::from_value(&serde_json::json!({
            "command": "fixedincome/spreads/tcm/10y2y",
            "start_date": "2020-01-01",
            "limit": 10
        }))
        .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.command, "fixedincome/spreads/tcm/10y2y");
        assert_eq!(query.series_id, "T10Y2Y");
        assert_eq!(query.params.limit, Some(10));

        // Unknown command and missing command both error.
        assert!(FredCatalogQuery::from_value(&serde_json::json!({ "command": "bogus" })).is_err());
        assert!(FredCatalogQuery::from_value(&serde_json::json!({})).is_err());
    }

    #[test]
    fn fred_search_request_encodes_and_guards_api_key() {
        let request = fred_search_request("housing starts", true)
            .unwrap_or_else(|error| panic!("search request should build: {error}"));
        assert_eq!(request.endpoint, "fred_search");
        assert!(request.path.contains("search_text=housing%20starts"));
        assert!(fred_search_request("housing", false).is_err());
        assert!(fred_search_request("   ", true).is_err());
        // Injection characters are percent-encoded, not passed through raw.
        let injected = fred_search_request("a&file_type=xml", true)
            .unwrap_or_else(|error| panic!("should build: {error}"));
        assert!(!injected.path.contains("a&file_type=xml"));
        assert!(injected.path.contains("a%26file_type%3Dxml"));
    }

    #[test]
    fn fred_release_and_regional_requests_build() {
        let release = fred_release_request(10, true)
            .unwrap_or_else(|error| panic!("release request should build: {error}"));
        assert_eq!(release.endpoint, "fred_release");
        assert!(release.path.contains("release_id=10"));
        assert!(fred_release_request(10, false).is_err());

        let regional = fred_regional_request("nypop", true)
            .unwrap_or_else(|error| panic!("regional request should build: {error}"));
        assert_eq!(regional.endpoint, "fred_regional");
        assert!(regional.path.contains("series_id=NYPOP"));
        assert!(fred_regional_request("NYPOP", false).is_err());
        assert!(fred_regional_request("bad&id", true).is_err());
    }
}
