#![forbid(unsafe_code)]

use thiserror::Error;

pub const PROVIDER_ID: &str = "alpaca";
pub const BASE_URL: &str = "https://data.alpaca.markets";
pub const API_KEY_HEADER: &str = "APCA-API-KEY-ID";

pub type Result<T> = std::result::Result<T, AlpacaProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub credential_header: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub credential_header: &'static str,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AlpacaProviderError {
    #[error("alpaca symbol must not be empty")]
    EmptySymbol,
    #[error("alpaca api key must be supplied by the caller")]
    MissingApiKey,
}

pub fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "stock_bars",
        base_url: BASE_URL,
        credential_header: API_KEY_HEADER,
    }];
    ENDPOINTS
}

pub fn stock_bars_request(symbol: &str, api_key_present: bool) -> Result<ProviderRequest> {
    if !api_key_present {
        return Err(AlpacaProviderError::MissingApiKey);
    }
    let symbol = normalize_symbol(symbol)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "stock_bars",
        path: format!("/v2/stocks/{symbol}/bars"),
        credential_header: API_KEY_HEADER,
    })
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(AlpacaProviderError::EmptySymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_offline_request_contract_without_secret_material() {
        let request = stock_bars_request("aapl", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(endpoints()[0].provider, "alpaca");
        assert_eq!(request.path, "/v2/stocks/AAPL/bars");
        assert_eq!(request.credential_header, API_KEY_HEADER);
        assert!(stock_bars_request("AAPL", false).is_err());
        assert!(stock_bars_request("", true).is_err());
    }
}
