#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::BinanceHttpTickerPriceFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BinanceTickerPriceQuery {
    pub symbol: String,
}

impl BinanceTickerPriceQuery {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BinanceTickerPrice {
    pub symbol: String,
    pub price: f64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BinanceProviderError {
    #[error("binance symbol must not be empty")]
    EmptySymbol,
    #[error("binance symbol contains unsupported characters")]
    InvalidSymbol,
}

#[must_use]
pub const fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "ticker_price",
        base_url: BASE_URL,
        requires_credential: false,
    }];
    ENDPOINTS
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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
    if !symbol
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(BinanceProviderError::InvalidSymbol);
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
        assert!(ticker_price_request("BTCUSDT&recvWindow=1").is_err());
    }

    #[test]
    fn builds_ticker_price_query_model() {
        let query = BinanceTickerPriceQuery::new("ethusdt")
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.symbol, "ETHUSDT");
        assert!(BinanceTickerPriceQuery::new("").is_err());
        assert!(BinanceTickerPriceQuery::new("ETHUSDT&recvWindow=1").is_err());
    }
}
