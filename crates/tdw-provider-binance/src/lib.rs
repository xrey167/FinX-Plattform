#![forbid(unsafe_code)]

use thiserror::Error;

pub const PROVIDER_ID: &str = "binance";
pub const BASE_URL: &str = "https://api.binance.com";

pub type Result<T> = std::result::Result<T, BinanceProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub requires_credential: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
    pub requires_credential: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BinanceProviderError {
    #[error("binance symbol must not be empty")]
    EmptySymbol,
}

pub fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "ticker_price",
        base_url: BASE_URL,
        requires_credential: false,
    }];
    ENDPOINTS
}

pub fn ticker_price_request(symbol: &str) -> Result<ProviderRequest> {
    let symbol = normalize_symbol(symbol)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "ticker_price",
        path: format!("/api/v3/ticker/price?symbol={symbol}"),
        requires_credential: false,
    })
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(BinanceProviderError::EmptySymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_public_market_request_contract() {
        let request = ticker_price_request("btcusdt")
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(endpoints()[0].provider, "binance");
        assert_eq!(request.path, "/api/v3/ticker/price?symbol=BTCUSDT");
        assert!(!request.requires_credential);
        assert!(ticker_price_request("").is_err());
    }
}
