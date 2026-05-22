#![forbid(unsafe_code)]

use thiserror::Error;

pub const PROVIDER_ID: &str = "polygon";
pub const BASE_URL: &str = "https://api.polygon.io";
pub const API_KEY_PARAM: &str = "apiKey";

pub type Result<T> = std::result::Result<T, PolygonProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_param: &'static str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolygonProviderError {
    #[error("polygon ticker must not be empty")]
    EmptyTicker,
    #[error("polygon api key must be supplied by the caller")]
    MissingApiKey,
}

pub fn aggregates_request(ticker: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(PolygonProviderError::MissingApiKey);
    }
    let ticker = normalize_ticker(ticker)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "aggregates",
        path: format!("/v2/aggs/ticker/{ticker}/range/1/day"),
        credential_param: API_KEY_PARAM,
    })
}

fn normalize_ticker(ticker: &str) -> Result<String> {
    let ticker = ticker.trim();
    if ticker.is_empty() {
        return Err(PolygonProviderError::EmptyTicker);
    }
    Ok(ticker.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_aggregates_request_contract() {
        let request = aggregates_request("msft", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(request.provider, "polygon");
        assert!(request.path.contains("/MSFT/range/1/day"));
        assert_eq!(request.credential_param, API_KEY_PARAM);
        assert!(aggregates_request("MSFT", false).is_err());
        assert!(aggregates_request("", true).is_err());
    }
}
