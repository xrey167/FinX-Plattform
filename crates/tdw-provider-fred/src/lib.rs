#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::FredHttpSeriesObservationsFetcher;

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
}

#[must_use]
pub const fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "series_observations",
        base_url: BASE_URL,
        credential_param: API_KEY_PARAM,
    }];
    ENDPOINTS
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
}
