#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::CoinGeckoHttpOhlcFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "coingecko";
pub const BASE_URL: &str = "https://api.coingecko.com/api/v3";
pub const API_KEY_HEADER: &str = "x-cg-demo-api-key";

pub type Result<T> = std::result::Result<T, CoinGeckoProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRequest {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub path: String,
}

/// Query parameters for CoinGecko OHLC endpoint.
///
/// `coin_id` is the CoinGecko coin identifier (e.g. `"bitcoin"`, `"ethereum"`).
/// `vs_currency` is the target currency (e.g. `"usd"`, `"eur"`).
/// `days` is the number of days of data to fetch (1, 7, 14, 30, 90, 180, 365, or `"max"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoinGeckoOhlcQuery {
    pub coin_id: String,
    pub vs_currency: String,
    pub days: u32,
}

impl CoinGeckoOhlcQuery {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(coin_id: &str, vs_currency: &str, days: u32) -> Result<Self> {
        Ok(Self {
            coin_id: normalize_coin_id(coin_id)?,
            vs_currency: normalize_currency(vs_currency)?,
            days: validate_days(days)?,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CoinGeckoProviderError {
    #[error("coingecko coin id must not be empty")]
    EmptyCoinId,
    #[error("coingecko coin id contains unsupported characters")]
    InvalidCoinId,
    #[error("coingecko vs_currency must not be empty")]
    EmptyCurrency,
    #[error("coingecko vs_currency contains unsupported characters")]
    InvalidCurrency,
    #[error("coingecko days must be one of: 1, 7, 14, 30, 90, 180, 365")]
    InvalidDays,
}

/// Build an OHLC request for the given coin.
///
/// # Errors
///
/// Returns an error variant if any parameter is invalid.
pub fn ohlc_request(coin_id: &str, vs_currency: &str, days: u32) -> Result<ProviderRequest> {
    let coin_id = normalize_coin_id(coin_id)?;
    let vs_currency = normalize_currency(vs_currency)?;
    let days = validate_days(days)?;
    Ok(ProviderRequest {
        provider: PROVIDER_ID,
        endpoint: "ohlc",
        path: format!("/coins/{coin_id}/ohlc?vs_currency={vs_currency}&days={days}"),
    })
}

fn normalize_coin_id(coin_id: &str) -> Result<String> {
    let coin_id = coin_id.trim();
    if coin_id.is_empty() {
        return Err(CoinGeckoProviderError::EmptyCoinId);
    }
    if !coin_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(CoinGeckoProviderError::InvalidCoinId);
    }
    Ok(coin_id.to_ascii_lowercase())
}

fn normalize_currency(currency: &str) -> Result<String> {
    let currency = currency.trim();
    if currency.is_empty() {
        return Err(CoinGeckoProviderError::EmptyCurrency);
    }
    if !currency.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CoinGeckoProviderError::InvalidCurrency);
    }
    Ok(currency.to_ascii_lowercase())
}

fn validate_days(days: u32) -> Result<u32> {
    match days {
        1 | 7 | 14 | 30 | 90 | 180 | 365 => Ok(days),
        _ => Err(CoinGeckoProviderError::InvalidDays),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_ohlc_request_contract() {
        let request = ohlc_request("bitcoin", "usd", 30)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(request.provider, "coingecko");
        assert_eq!(request.endpoint, "ohlc");
        assert!(request.path.contains("/coins/bitcoin/ohlc"));
        assert!(request.path.contains("vs_currency=usd"));
        assert!(request.path.contains("days=30"));
    }

    #[test]
    fn normalises_coin_id_to_lowercase() {
        let request = ohlc_request("Bitcoin", "USD", 7)
            .unwrap_or_else(|error| panic!("request should build: {error}"));
        assert!(request.path.contains("/coins/bitcoin/ohlc"));
        assert!(request.path.contains("vs_currency=usd"));
    }

    #[test]
    fn rejects_empty_coin_id() {
        assert_eq!(
            ohlc_request("", "usd", 30),
            Err(CoinGeckoProviderError::EmptyCoinId)
        );
    }

    #[test]
    fn rejects_invalid_coin_id_characters() {
        assert_eq!(
            ohlc_request("bit coin!", "usd", 30),
            Err(CoinGeckoProviderError::InvalidCoinId)
        );
    }

    #[test]
    fn rejects_empty_currency() {
        assert_eq!(
            ohlc_request("bitcoin", "", 30),
            Err(CoinGeckoProviderError::EmptyCurrency)
        );
    }

    #[test]
    fn rejects_invalid_currency_characters() {
        assert_eq!(
            ohlc_request("bitcoin", "us$d", 30),
            Err(CoinGeckoProviderError::InvalidCurrency)
        );
    }

    #[test]
    fn rejects_invalid_days() {
        assert_eq!(
            ohlc_request("bitcoin", "usd", 5),
            Err(CoinGeckoProviderError::InvalidDays)
        );
        assert_eq!(
            ohlc_request("bitcoin", "usd", 0),
            Err(CoinGeckoProviderError::InvalidDays)
        );
    }

    #[test]
    fn accepts_all_valid_days_values() {
        for &days in &[1u32, 7, 14, 30, 90, 180, 365] {
            assert!(
                ohlc_request("bitcoin", "usd", days).is_ok(),
                "days={days} should be valid"
            );
        }
    }

    #[test]
    fn builds_ohlc_query_model() {
        let query = CoinGeckoOhlcQuery::new("ethereum", "eur", 14)
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.coin_id, "ethereum");
        assert_eq!(query.vs_currency, "eur");
        assert_eq!(query.days, 14);
    }
}
