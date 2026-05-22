#![forbid(unsafe_code)]

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FredProviderError {
    #[error("fred series id must not be empty")]
    EmptySeriesId,
    #[error("fred api key must be supplied by the caller")]
    MissingApiKey,
}

pub fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "series_observations",
        base_url: BASE_URL,
        credential_param: API_KEY_PARAM,
    }];
    ENDPOINTS
}

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
    }
}
