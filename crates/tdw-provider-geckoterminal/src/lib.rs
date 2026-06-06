#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::GeckoTerminalHttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "geckoterminal";
pub const BASE_URL: &str = "https://api.geckoterminal.com/api/v2";
pub const ACCEPT_HEADER: &str = "application/json;version=20230302";

pub type Result<T> = std::result::Result<T, GeckoTerminalError>;

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query for a single DEX pool by network and pool address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GeckoTerminalPoolQuery {
    /// Network slug, e.g. `"eth"`, `"solana"`, `"bsc"`.
    pub network: String,
    /// Pool contract address (EVM `0x…` or Solana base58, 32-44 chars).
    pub pool_address: String,
}

impl GeckoTerminalPoolQuery {
    /// Construct and validate a pool query.
    ///
    /// # Errors
    ///
    /// Returns [`GeckoTerminalError`] when `network` or `pool_address` fail
    /// validation.
    pub fn new(network: &str, pool_address: &str) -> Result<Self> {
        Ok(Self {
            network: validate_network(network)?,
            pool_address: validate_pool_address(pool_address)?,
        })
    }
}

/// Query for trending pools on a network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GeckoTerminalTrendingQuery {
    /// Network slug, e.g. `"eth"`, `"solana"`, `"bsc"`.
    pub network: String,
}

impl GeckoTerminalTrendingQuery {
    /// Construct and validate a trending-pools query.
    ///
    /// # Errors
    ///
    /// Returns [`GeckoTerminalError`] when `network` fails validation.
    pub fn new(network: &str) -> Result<Self> {
        Ok(Self {
            network: validate_network(network)?,
        })
    }
}

/// Query for pools associated with a specific token on a network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GeckoTerminalTokenPoolsQuery {
    /// Network slug, e.g. `"eth"`, `"solana"`, `"bsc"`.
    pub network: String,
    /// Token contract address.
    pub token_address: String,
}

impl GeckoTerminalTokenPoolsQuery {
    /// Construct and validate a token-pools query.
    ///
    /// # Errors
    ///
    /// Returns [`GeckoTerminalError`] when `network` or `token_address` fail
    /// validation.
    pub fn new(network: &str, token_address: &str) -> Result<Self> {
        Ok(Self {
            network: validate_network(network)?,
            token_address: validate_pool_address(token_address)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single DEX pool record returned by `GeckoTerminal`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DexPool {
    /// `GeckoTerminal` composite id, e.g. `"eth_0x88e6a0…"`.
    pub id: String,
    /// Human-readable name, e.g. `"USDC/WETH 0.05%"`.
    pub name: String,
    /// Network slug.
    pub network: String,
    /// Pool contract address.
    pub pool_address: String,
    /// Base-token price in USD, if available.
    pub base_token_price_usd: Option<f64>,
    /// Quote-token price in USD, if available.
    pub quote_token_price_usd: Option<f64>,
    /// Total reserve in USD.
    pub reserve_in_usd: Option<f64>,
    /// 24-hour trading volume in USD.
    pub volume_usd_h24: Option<f64>,
    /// 6-hour trading volume in USD.
    pub volume_usd_h6: Option<f64>,
    /// 1-hour trading volume in USD.
    pub volume_usd_h1: Option<f64>,
    /// 24-hour price change percentage.
    pub price_change_h24: Option<f64>,
    /// 6-hour price change percentage.
    pub price_change_h6: Option<f64>,
    /// 1-hour price change percentage.
    pub price_change_h1: Option<f64>,
    /// Pool creation timestamp (ISO-8601), if available.
    pub pool_created_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GeckoTerminalError {
    #[error("network must be non-empty lowercase alphanumeric/hyphen, got: {0:?}")]
    InvalidNetwork(String),
    #[error("pool address must start with 0x (EVM) or be 32-44 chars (Solana), got: {0:?}")]
    InvalidPoolAddress(String),
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_network(network: &str) -> Result<String> {
    let network = network.trim().to_ascii_lowercase();
    if network.is_empty()
        || !network
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(GeckoTerminalError::InvalidNetwork(network));
    }
    Ok(network)
}

fn validate_pool_address(addr: &str) -> Result<String> {
    let addr = addr.trim().to_ascii_lowercase();
    // Accept EVM (0x-prefixed, min 42 chars) or Solana (32-44 base58 chars).
    let is_evm = addr.starts_with("0x") && addr.len() >= 42;
    let is_solana = !addr.starts_with("0x") && addr.len() >= 32 && addr.len() <= 44;
    if !is_evm && !is_solana {
        return Err(GeckoTerminalError::InvalidPoolAddress(addr));
    }
    Ok(addr)
}

// ---------------------------------------------------------------------------
// Mock fetcher (no network I/O — for unit tests and offline use)
// ---------------------------------------------------------------------------

/// Returns a hard-coded [`DexPool`] for offline testing.
///
/// # Errors
///
/// Propagates query-validation errors.
pub fn mock_pool(network: &str, pool_address: &str) -> Result<DexPool> {
    let query = GeckoTerminalPoolQuery::new(network, pool_address)?;
    Ok(DexPool {
        id: format!("{}_{}", query.network, query.pool_address),
        name: "USDC/WETH 0.05%".to_string(),
        network: query.network,
        pool_address: query.pool_address,
        base_token_price_usd: Some(1.0001),
        quote_token_price_usd: Some(3125.50),
        reserve_in_usd: Some(125_000_000.5),
        volume_usd_h24: Some(85_000_000.0),
        volume_usd_h6: Some(22_000_000.0),
        volume_usd_h1: Some(4_500_000.0),
        price_change_h24: Some(-0.25),
        price_change_h6: Some(0.10),
        price_change_h1: Some(0.05),
        pool_created_at: Some("2021-05-04T00:00:00Z".to_string()),
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- GeckoTerminalPoolQuery ---

    #[test]
    fn pool_query_accepts_evm_address() {
        let q = GeckoTerminalPoolQuery::new("eth", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .unwrap_or_else(|e| panic!("expected Ok: {e}"));

        assert_eq!(q.network, "eth");
        assert_eq!(q.pool_address, "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640");
    }

    #[test]
    fn pool_query_accepts_solana_address() {
        // 44-char base58 Solana address
        let addr = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        let q = GeckoTerminalPoolQuery::new("solana", addr)
            .unwrap_or_else(|e| panic!("expected Ok: {e}"));

        assert_eq!(q.network, "solana");
        assert_eq!(q.pool_address, addr.to_ascii_lowercase());
    }

    #[test]
    fn pool_query_normalises_network_to_lowercase() {
        let q = GeckoTerminalPoolQuery::new("ETH", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .unwrap_or_else(|e| panic!("expected Ok: {e}"));

        assert_eq!(q.network, "eth");
    }

    #[test]
    fn pool_query_rejects_empty_network() {
        let err = GeckoTerminalPoolQuery::new("", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .expect_err("empty network must be rejected");

        assert!(matches!(err, GeckoTerminalError::InvalidNetwork(_)));
    }

    #[test]
    fn pool_query_rejects_network_with_slash() {
        let err = GeckoTerminalPoolQuery::new(
            "eth/../../secret",
            "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640",
        )
        .expect_err("network with slash must be rejected");

        assert!(matches!(err, GeckoTerminalError::InvalidNetwork(_)));
    }

    #[test]
    fn pool_query_rejects_short_evm_address() {
        // 0x-prefixed but only 10 chars — too short for EVM
        let err = GeckoTerminalPoolQuery::new("eth", "0x1234567890")
            .expect_err("short EVM address must be rejected");

        assert!(matches!(err, GeckoTerminalError::InvalidPoolAddress(_)));
    }

    #[test]
    fn pool_query_rejects_short_solana_address() {
        let err = GeckoTerminalPoolQuery::new("solana", "short")
            .expect_err("short Solana address must be rejected");

        assert!(matches!(err, GeckoTerminalError::InvalidPoolAddress(_)));
    }

    // --- GeckoTerminalTrendingQuery ---

    #[test]
    fn trending_query_accepts_valid_network() {
        let q =
            GeckoTerminalTrendingQuery::new("bsc").unwrap_or_else(|e| panic!("expected Ok: {e}"));

        assert_eq!(q.network, "bsc");
    }

    #[test]
    fn trending_query_rejects_invalid_network() {
        let err = GeckoTerminalTrendingQuery::new("ETH network!")
            .expect_err("network with spaces/bang must be rejected");

        assert!(matches!(err, GeckoTerminalError::InvalidNetwork(_)));
    }

    // --- GeckoTerminalTokenPoolsQuery ---

    #[test]
    fn token_pools_query_accepts_evm_token() {
        let token = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
        let q = GeckoTerminalTokenPoolsQuery::new("eth", token)
            .unwrap_or_else(|e| panic!("expected Ok: {e}"));

        assert_eq!(q.network, "eth");
        assert_eq!(q.token_address, token);
    }

    // --- mock_pool ---

    #[test]
    fn mock_pool_returns_expected_shape() {
        let pool = mock_pool("eth", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .unwrap_or_else(|e| panic!("mock_pool should succeed: {e}"));

        assert_eq!(pool.network, "eth");
        assert!(pool.id.starts_with("eth_0x"));
        assert_eq!(pool.name, "USDC/WETH 0.05%");
        assert!(pool.volume_usd_h24.is_some());
        assert_eq!(pool.price_change_h24, Some(-0.25));
    }

    #[test]
    fn mock_pool_propagates_validation_error() {
        let err = mock_pool("", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .expect_err("empty network must propagate");

        assert!(matches!(err, GeckoTerminalError::InvalidNetwork(_)));
    }

    // --- serde round-trip ---

    #[test]
    fn dex_pool_round_trips_through_json() {
        let pool = mock_pool("eth", "0x88e6a0c2ddd26feeb64f039a2c41296fcb3f5640")
            .unwrap_or_else(|e| panic!("mock_pool should succeed: {e}"));

        let json =
            serde_json::to_string(&pool).unwrap_or_else(|e| panic!("serialize must succeed: {e}"));
        let decoded: DexPool =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize must succeed: {e}"));

        assert_eq!(pool, decoded);
    }
}
