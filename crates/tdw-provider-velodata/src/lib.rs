#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    VelodataHttpFundingFetcher, VelodataHttpLiquidationsFetcher, VelodataHttpOiFetcher,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const PROVIDER_ID: &str = "velodata";
pub const BASE_URL: &str = "https://api.velo.xyz/v1";
pub const API_KEY_HEADER: &str = "X-API-KEY";
pub const API_KEY_ENV: &str = "TDW_VELODATA_API_KEY";

pub type Result<T> = std::result::Result<T, VelodataProviderError>;

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query parameters for the Velodata funding-rates endpoint.
///
/// Fetches historical perpetual-futures funding rates for a given
/// exchange/symbol pair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VelodataFundingQuery {
    pub exchange: String,
    pub symbol: String,
    pub limit: u32,
}

/// Query parameters for the Velodata aggregated-liquidations endpoint.
///
/// Fetches aggregated long/short liquidation volumes for a given base
/// asset symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VelodataLiquidationsQuery {
    pub symbol: String,
    pub limit: u32,
}

/// Query parameters for the Velodata aggregated open-interest endpoint.
///
/// Fetches cross-exchange open-interest snapshots for a given base asset
/// symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VelodataOiQuery {
    pub symbol: String,
    pub limit: u32,
}

// ---------------------------------------------------------------------------
// Domain row types — defined here (not in tdw-domain) because they are
// Velodata-specific and serde_json is a hard dep.
// ---------------------------------------------------------------------------

/// One funding-rate sample from the Velodata `/funding/rates` endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FundingRate {
    pub timestamp: i64,
    pub exchange: String,
    pub symbol: String,
    #[serde(rename = "fundingRate")]
    pub funding_rate: f64,
    #[serde(rename = "fundingRateAnnualized")]
    pub funding_rate_annualized: f64,
}

/// One aggregated-liquidations sample from the Velodata
/// `/liquidations/aggregated` endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AggregatedLiquidation {
    pub timestamp: i64,
    pub symbol: String,
    #[serde(rename = "longLiquidations")]
    pub long_liquidations: f64,
    #[serde(rename = "shortLiquidations")]
    pub short_liquidations: f64,
    #[serde(rename = "totalLiquidations")]
    pub total_liquidations: f64,
}

/// One aggregated open-interest sample from the Velodata
/// `/oi/aggregated` endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AggregatedOi {
    pub timestamp: i64,
    pub symbol: String,
    #[serde(rename = "openInterest")]
    pub open_interest: f64,
    #[serde(rename = "openInterestChange")]
    pub open_interest_change: f64,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VelodataProviderError {
    #[error("velodata exchange must not be empty")]
    EmptyExchange,
    #[error("velodata exchange contains unsupported characters")]
    InvalidExchange,
    #[error("velodata symbol must not be empty")]
    EmptySymbol,
    #[error("velodata symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("velodata limit must be greater than zero")]
    InvalidLimit,
    #[error("velodata api key must be supplied via {API_KEY_ENV}")]
    MissingApiKey,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn normalize_exchange(exchange: &str) -> Result<String> {
    let exchange = exchange.trim();
    if exchange.is_empty() {
        return Err(VelodataProviderError::EmptyExchange);
    }
    if !exchange
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(VelodataProviderError::InvalidExchange);
    }
    Ok(exchange.to_ascii_lowercase())
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(VelodataProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(VelodataProviderError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

const fn validate_limit(limit: u32) -> Result<u32> {
    if limit == 0 {
        return Err(VelodataProviderError::InvalidLimit);
    }
    Ok(limit)
}

// ---------------------------------------------------------------------------
// Query constructors (offline, no feature gate)
// ---------------------------------------------------------------------------

impl VelodataFundingQuery {
    /// Build and validate a funding-rates query.
    ///
    /// # Errors
    ///
    /// Returns [`VelodataProviderError`] if any field fails validation.
    pub fn new(exchange: &str, symbol: &str, limit: u32) -> Result<Self> {
        Ok(Self {
            exchange: normalize_exchange(exchange)?,
            symbol: normalize_symbol(symbol)?,
            limit: validate_limit(limit)?,
        })
    }
}

impl VelodataLiquidationsQuery {
    /// Build and validate a liquidations query.
    ///
    /// # Errors
    ///
    /// Returns [`VelodataProviderError`] if any field fails validation.
    pub fn new(symbol: &str, limit: u32) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            limit: validate_limit(limit)?,
        })
    }
}

impl VelodataOiQuery {
    /// Build and validate an open-interest query.
    ///
    /// # Errors
    ///
    /// Returns [`VelodataProviderError`] if any field fails validation.
    pub fn new(symbol: &str, limit: u32) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            limit: validate_limit(limit)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Mock fetcher (offline, always available)
// ---------------------------------------------------------------------------

/// Returns a deterministic set of mock funding-rate rows for offline tests.
#[must_use]
pub fn mock_funding_rows(exchange: &str, symbol: &str) -> Vec<FundingRate> {
    vec![
        FundingRate {
            timestamp: 1_704_153_600_000,
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            funding_rate: 0.0001,
            funding_rate_annualized: 0.1095,
        },
        FundingRate {
            timestamp: 1_704_182_400_000,
            exchange: exchange.to_string(),
            symbol: symbol.to_string(),
            funding_rate: 0.00012,
            funding_rate_annualized: 0.1314,
        },
    ]
}

/// Returns a deterministic set of mock liquidation rows for offline tests.
#[must_use]
pub fn mock_liquidations_rows(symbol: &str) -> Vec<AggregatedLiquidation> {
    vec![
        AggregatedLiquidation {
            timestamp: 1_704_153_600_000,
            symbol: symbol.to_string(),
            long_liquidations: 1_250_000.0,
            short_liquidations: 850_000.0,
            total_liquidations: 2_100_000.0,
        },
        AggregatedLiquidation {
            timestamp: 1_704_157_200_000,
            symbol: symbol.to_string(),
            long_liquidations: 430_000.0,
            short_liquidations: 210_000.0,
            total_liquidations: 640_000.0,
        },
    ]
}

/// Returns a deterministic set of mock open-interest rows for offline tests.
#[must_use]
pub fn mock_oi_rows(symbol: &str) -> Vec<AggregatedOi> {
    vec![
        AggregatedOi {
            timestamp: 1_704_153_600_000,
            symbol: symbol.to_string(),
            open_interest: 15_200_000_000.0,
            open_interest_change: 250_000_000.0,
        },
        AggregatedOi {
            timestamp: 1_704_157_200_000,
            symbol: symbol.to_string(),
            open_interest: 15_450_000_000.0,
            open_interest_change: 250_000_000.0,
        },
    ]
}

// ---------------------------------------------------------------------------
// transform_query helpers (offline, used by both mock and http fetchers)
// ---------------------------------------------------------------------------

/// Parse a [`VelodataFundingQuery`] from a raw JSON params value.
///
/// # Errors
///
/// Returns an error if required fields are missing or fail validation.
pub fn funding_query_from_value(
    params: &Value,
) -> std::result::Result<VelodataFundingQuery, String> {
    let exchange = params
        .get("exchange")
        .and_then(Value::as_str)
        .ok_or_else(|| "velodata: missing exchange".to_string())?;
    let symbol = params
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| "velodata: missing symbol".to_string())?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(100);
    VelodataFundingQuery::new(exchange, symbol, limit).map_err(|e| e.to_string())
}

/// Parse a [`VelodataLiquidationsQuery`] from a raw JSON params value.
///
/// # Errors
///
/// Returns an error if required fields are missing or fail validation.
pub fn liquidations_query_from_value(
    params: &Value,
) -> std::result::Result<VelodataLiquidationsQuery, String> {
    let symbol = params
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| "velodata: missing symbol".to_string())?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(24);
    VelodataLiquidationsQuery::new(symbol, limit).map_err(|e| e.to_string())
}

/// Parse a [`VelodataOiQuery`] from a raw JSON params value.
///
/// # Errors
///
/// Returns an error if required fields are missing or fail validation.
pub fn oi_query_from_value(params: &Value) -> std::result::Result<VelodataOiQuery, String> {
    let symbol = params
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| "velodata: missing symbol".to_string())?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(24);
    VelodataOiQuery::new(symbol, limit).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- VelodataFundingQuery ---

    #[test]
    fn funding_query_normalizes_exchange_and_symbol() {
        let query = VelodataFundingQuery::new("Binance", "btcusdt", 100)
            .unwrap_or_else(|e| panic!("query should build: {e}"));

        assert_eq!(query.exchange, "binance");
        assert_eq!(query.symbol, "BTCUSDT");
        assert_eq!(query.limit, 100);
    }

    #[test]
    fn funding_query_rejects_empty_exchange() {
        assert_eq!(
            VelodataFundingQuery::new("", "BTCUSDT", 10),
            Err(VelodataProviderError::EmptyExchange),
        );
    }

    #[test]
    fn funding_query_rejects_invalid_exchange_characters() {
        assert_eq!(
            VelodataFundingQuery::new("bin/ance", "BTCUSDT", 10),
            Err(VelodataProviderError::InvalidExchange),
        );
        assert_eq!(
            VelodataFundingQuery::new("bin?ance", "BTCUSDT", 10),
            Err(VelodataProviderError::InvalidExchange),
        );
    }

    #[test]
    fn funding_query_rejects_empty_symbol() {
        assert_eq!(
            VelodataFundingQuery::new("binance", "", 10),
            Err(VelodataProviderError::EmptySymbol),
        );
    }

    #[test]
    fn funding_query_rejects_invalid_symbol_characters() {
        assert_eq!(
            VelodataFundingQuery::new("binance", "BTC/USDT", 10),
            Err(VelodataProviderError::InvalidSymbol),
        );
    }

    #[test]
    fn funding_query_rejects_zero_limit() {
        assert_eq!(
            VelodataFundingQuery::new("binance", "BTCUSDT", 0),
            Err(VelodataProviderError::InvalidLimit),
        );
    }

    // --- VelodataLiquidationsQuery ---

    #[test]
    fn liquidations_query_normalizes_symbol() {
        let query = VelodataLiquidationsQuery::new("btc", 24)
            .unwrap_or_else(|e| panic!("query should build: {e}"));

        assert_eq!(query.symbol, "BTC");
        assert_eq!(query.limit, 24);
    }

    #[test]
    fn liquidations_query_rejects_invalid_inputs() {
        assert_eq!(
            VelodataLiquidationsQuery::new("", 10),
            Err(VelodataProviderError::EmptySymbol),
        );
        assert_eq!(
            VelodataLiquidationsQuery::new("BTC/ETH", 10),
            Err(VelodataProviderError::InvalidSymbol),
        );
        assert_eq!(
            VelodataLiquidationsQuery::new("BTC", 0),
            Err(VelodataProviderError::InvalidLimit),
        );
    }

    // --- VelodataOiQuery ---

    #[test]
    fn oi_query_normalizes_symbol() {
        let query =
            VelodataOiQuery::new("eth", 48).unwrap_or_else(|e| panic!("query should build: {e}"));

        assert_eq!(query.symbol, "ETH");
        assert_eq!(query.limit, 48);
    }

    #[test]
    fn oi_query_rejects_invalid_inputs() {
        assert_eq!(
            VelodataOiQuery::new("", 10),
            Err(VelodataProviderError::EmptySymbol),
        );
        assert_eq!(
            VelodataOiQuery::new("BTC?", 10),
            Err(VelodataProviderError::InvalidSymbol),
        );
        assert_eq!(
            VelodataOiQuery::new("BTC", 0),
            Err(VelodataProviderError::InvalidLimit),
        );
    }

    // --- mock fetchers ---

    #[test]
    fn mock_funding_rows_returns_expected_shape() {
        let rows = mock_funding_rows("binance", "BTCUSDT");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].exchange, "binance");
        assert_eq!(rows[0].symbol, "BTCUSDT");
        assert_eq!(rows[0].funding_rate, 0.0001);
        assert!(rows[0].funding_rate_annualized > 0.0);
        assert!(rows[1].timestamp > rows[0].timestamp);
    }

    #[test]
    fn mock_liquidations_rows_returns_expected_shape() {
        let rows = mock_liquidations_rows("BTC");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "BTC");
        assert!(
            (rows[0].total_liquidations - rows[0].long_liquidations - rows[0].short_liquidations)
                .abs()
                < f64::EPSILON
        );
        assert!(rows[1].timestamp > rows[0].timestamp);
    }

    #[test]
    fn mock_oi_rows_returns_expected_shape() {
        let rows = mock_oi_rows("BTC");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "BTC");
        assert!(rows[0].open_interest > 0.0);
        assert!(rows[1].timestamp > rows[0].timestamp);
    }

    // --- funding_query_from_value ---

    #[test]
    fn funding_query_from_value_parses_valid_json() {
        let params = serde_json::json!({
            "exchange": "Binance",
            "symbol": "btcusdt",
            "limit": 50
        });
        let query =
            funding_query_from_value(&params).unwrap_or_else(|e| panic!("should parse: {e}"));

        assert_eq!(query.exchange, "binance");
        assert_eq!(query.symbol, "BTCUSDT");
        assert_eq!(query.limit, 50);
    }

    #[test]
    fn funding_query_from_value_uses_default_limit() {
        let params = serde_json::json!({ "exchange": "binance", "symbol": "ETHUSDT" });
        let query = funding_query_from_value(&params)
            .unwrap_or_else(|e| panic!("should parse with default limit: {e}"));

        assert_eq!(query.limit, 100);
    }

    #[test]
    fn funding_query_from_value_errors_on_missing_exchange() {
        let params = serde_json::json!({ "symbol": "BTCUSDT" });

        assert!(funding_query_from_value(&params).is_err());
    }

    // --- liquidations_query_from_value ---

    #[test]
    fn liquidations_query_from_value_parses_valid_json() {
        let params = serde_json::json!({ "symbol": "btc", "limit": 12 });
        let query =
            liquidations_query_from_value(&params).unwrap_or_else(|e| panic!("should parse: {e}"));

        assert_eq!(query.symbol, "BTC");
        assert_eq!(query.limit, 12);
    }

    #[test]
    fn liquidations_query_from_value_uses_default_limit() {
        let params = serde_json::json!({ "symbol": "ETH" });
        let query = liquidations_query_from_value(&params)
            .unwrap_or_else(|e| panic!("should parse with default limit: {e}"));

        assert_eq!(query.limit, 24);
    }

    // --- oi_query_from_value ---

    #[test]
    fn oi_query_from_value_parses_valid_json() {
        let params = serde_json::json!({ "symbol": "eth", "limit": 6 });
        let query = oi_query_from_value(&params).unwrap_or_else(|e| panic!("should parse: {e}"));

        assert_eq!(query.symbol, "ETH");
        assert_eq!(query.limit, 6);
    }

    #[test]
    fn oi_query_from_value_errors_on_missing_symbol() {
        let params = serde_json::json!({ "limit": 10 });

        assert!(oi_query_from_value(&params).is_err());
    }

    // --- serde round-trip for domain rows ---

    #[test]
    fn funding_rate_round_trips_json() {
        let row = FundingRate {
            timestamp: 1_704_153_600_000,
            exchange: "binance".to_string(),
            symbol: "BTCUSDT".to_string(),
            funding_rate: 0.0001,
            funding_rate_annualized: 0.1095,
        };
        let json = serde_json::to_string(&row).unwrap_or_else(|e| panic!("should serialize: {e}"));
        let back: FundingRate =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("should deserialize: {e}"));

        assert_eq!(back, row);
        assert!(json.contains("fundingRate"));
        assert!(json.contains("fundingRateAnnualized"));
    }

    #[test]
    fn aggregated_liquidation_round_trips_json() {
        let row = AggregatedLiquidation {
            timestamp: 1_704_153_600_000,
            symbol: "BTC".to_string(),
            long_liquidations: 1_250_000.0,
            short_liquidations: 850_000.0,
            total_liquidations: 2_100_000.0,
        };
        let json = serde_json::to_string(&row).unwrap_or_else(|e| panic!("should serialize: {e}"));
        let back: AggregatedLiquidation =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("should deserialize: {e}"));

        assert_eq!(back, row);
        assert!(json.contains("longLiquidations"));
    }

    #[test]
    fn aggregated_oi_round_trips_json() {
        let row = AggregatedOi {
            timestamp: 1_704_153_600_000,
            symbol: "BTC".to_string(),
            open_interest: 15_200_000_000.0,
            open_interest_change: 250_000_000.0,
        };
        let json = serde_json::to_string(&row).unwrap_or_else(|e| panic!("should serialize: {e}"));
        let back: AggregatedOi =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("should deserialize: {e}"));

        assert_eq!(back, row);
        assert!(json.contains("openInterest"));
        assert!(json.contains("openInterestChange"));
    }
}
