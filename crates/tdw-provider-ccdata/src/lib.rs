#![forbid(unsafe_code)]

//! `CCData` (formerly `CryptoCompare`) market-data provider for the TDW platform.
//!
//! # Authentication
//!
//! Set `TDW_CCDATA_API_KEY` in the environment before making live calls.
//! The key is sent as the `authorization: Apikey {key}` request header on
//! every request.
//!
//! # Base URL
//!
//! `https://data-api.ccdata.io`

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::CCDataHttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider identifier used in registry entries and error messages.
pub const PROVIDER_ID: &str = "ccdata";

/// Base URL for the `CCData` REST API.
pub const BASE_URL: &str = "https://data-api.ccdata.io";

/// Environment variable that holds the `CCData` API key.
pub const API_KEY_ENV: &str = "TDW_CCDATA_API_KEY";

/// Convenience type alias for fallible `CCData` operations.
pub type Result<T> = std::result::Result<T, CCDataError>;

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Query parameters for the `CCData` daily OHLCV endpoint.
///
/// Maps to:
/// `GET /spot/v1/historical/days?market=ccix&instrument={instrument}&limit={limit}`
///
/// # Validation
///
/// `instrument` must match the pattern `ASSET-CURRENCY` (e.g. `BTC-USD`).
///
/// # Example
///
/// ```rust
/// use tdw_provider_ccdata::CCDataOhlcvQuery;
///
/// let q = CCDataOhlcvQuery::new("ccix", "BTC-USD", 30)
///     .expect("valid query");
/// assert_eq!(q.instrument, "BTC-USD");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CCDataOhlcvQuery {
    /// Exchange market identifier (e.g. `ccix`).
    pub market: String,
    /// Instrument pair in `ASSET-CURRENCY` format (e.g. `BTC-USD`).
    pub instrument: String,
    /// Number of daily bars to return (1–2 000).
    pub limit: u32,
}

impl CCDataOhlcvQuery {
    /// Construct and validate a new OHLCV query.
    ///
    /// # Errors
    ///
    /// - [`CCDataError::EmptyMarket`] when `market` is blank.
    /// - [`CCDataError::InvalidInstrument`] when `instrument` does not match
    ///   `ASSET-CURRENCY` (must contain exactly one `-`, non-empty parts,
    ///   ASCII-alphanumeric characters only in each part).
    pub fn new(market: &str, instrument: &str, limit: u32) -> Result<Self> {
        let market = normalize_nonempty(market, "market").map_err(|()| CCDataError::EmptyMarket)?;
        let instrument = validate_instrument(instrument)?;
        Ok(Self {
            market,
            instrument,
            limit,
        })
    }
}

/// Query parameters for the `CCData` asset metadata endpoint.
///
/// Maps to:
/// `GET /asset/v1/data/by/symbol?asset_symbol={symbol}`
///
/// # Example
///
/// ```rust
/// use tdw_provider_ccdata::CCDataAssetQuery;
///
/// let q = CCDataAssetQuery::new("BTC").expect("valid query");
/// assert_eq!(q.symbol, "BTC");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CCDataAssetQuery {
    /// Asset symbol (e.g. `BTC`).
    pub symbol: String,
}

impl CCDataAssetQuery {
    /// Construct and validate a new asset query.
    ///
    /// # Errors
    ///
    /// - [`CCDataError::EmptySymbol`] when `symbol` is blank.
    /// - [`CCDataError::InvalidSymbol`] when `symbol` contains non-ASCII-
    ///   alphanumeric characters.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the `CCData` provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CCDataError {
    /// The market string was empty or whitespace-only.
    #[error("ccdata market must not be empty")]
    EmptyMarket,

    /// The instrument does not follow the required `ASSET-CURRENCY` pattern.
    #[error("ccdata instrument must match ASSET-CURRENCY pattern (e.g. BTC-USD), got: {0}")]
    InvalidInstrument(String),

    /// The symbol string was empty or whitespace-only.
    #[error("ccdata asset symbol must not be empty")]
    EmptySymbol,

    /// The symbol contains characters not accepted by `CCData`.
    #[error("ccdata asset symbol contains unsupported characters")]
    InvalidSymbol,

    /// The `TDW_CCDATA_API_KEY` environment variable was not set or was empty.
    #[error("ccdata api key env TDW_CCDATA_API_KEY must be set")]
    MissingApiKey,

    /// A provider-level error (HTTP failure, unexpected JSON shape, etc.).
    #[error("ccdata provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Mock / offline data
// ---------------------------------------------------------------------------

/// Returns a minimal stub OHLCV JSON response matching the `CCData` shape for
/// offline testing.
#[must_use]
pub fn stub_ohlcv_response(instrument: &str) -> String {
    format!(
        r#"{{"Data":{{"TimeFrom":1704067200,"TimeTo":1706572800,"Entries":[{{"TIMESTAMP":1704067200,"OPEN":42000.0,"HIGH":45000.0,"LOW":41500.0,"CLOSE":44000.0,"VOLUME":850000.0,"QUOTE_VOLUME":37400000000.0}},{{"TIMESTAMP":1704153600,"OPEN":44000.0,"HIGH":46000.0,"LOW":43000.0,"CLOSE":45500.0,"VOLUME":920000.0,"QUOTE_VOLUME":41860000000.0}}],"Instrument":"{instrument}"}}}}"#
    )
}

/// Returns a minimal stub asset metadata JSON response matching the `CCData`
/// shape for offline testing.
#[must_use]
pub fn stub_asset_response(symbol: &str) -> String {
    format!(
        r#"{{"Data":{{"ID":1182,"SYMBOL":"{symbol}","NAME":"Bitcoin","ASSET_TYPE":"BLOCKCHAIN","LAUNCH_DATE":1230940800,"CIRCULATING_SUPPLY":19500000.0,"MARKET_CAP_USD":858000000000.0}}}}"#
    )
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn normalize_nonempty(value: &str, _field: &str) -> std::result::Result<String, ()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(())
    } else {
        Ok(trimmed.to_string())
    }
}

/// Validates that the instrument follows `ASSET-CURRENCY` (exactly one `-`,
/// both parts non-empty and ASCII-alphanumeric).
fn validate_instrument(instrument: &str) -> Result<String> {
    let instrument = instrument.trim();
    let parts: Vec<&str> = instrument.splitn(3, '-').collect();
    let valid = parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[0].chars().all(|c| c.is_ascii_alphanumeric())
        && parts[1].chars().all(|c| c.is_ascii_alphanumeric())
        && !instrument.is_empty();
    if valid {
        Ok(instrument.to_ascii_uppercase())
    } else {
        Err(CCDataError::InvalidInstrument(instrument.to_string()))
    }
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(CCDataError::EmptySymbol);
    }
    if !symbol.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(CCDataError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- CCDataOhlcvQuery ---

    #[test]
    fn ohlcv_query_normalizes_instrument_to_uppercase() {
        let q = CCDataOhlcvQuery::new("ccix", "btc-usd", 30)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.instrument, "BTC-USD");
        assert_eq!(q.market, "ccix");
        assert_eq!(q.limit, 30);
    }

    #[test]
    fn ohlcv_query_rejects_empty_market() {
        assert_eq!(
            CCDataOhlcvQuery::new("", "BTC-USD", 30),
            Err(CCDataError::EmptyMarket)
        );
        assert_eq!(
            CCDataOhlcvQuery::new("   ", "BTC-USD", 30),
            Err(CCDataError::EmptyMarket)
        );
    }

    #[test]
    fn ohlcv_query_rejects_instrument_without_dash() {
        let err =
            CCDataOhlcvQuery::new("ccix", "BTCUSD", 30).expect_err("missing dash must be rejected");
        assert!(
            matches!(err, CCDataError::InvalidInstrument(_)),
            "unexpected error variant: {err}"
        );
    }

    #[test]
    fn ohlcv_query_rejects_instrument_with_empty_parts() {
        let err = CCDataOhlcvQuery::new("ccix", "-USD", 30)
            .expect_err("empty asset part must be rejected");
        assert!(matches!(err, CCDataError::InvalidInstrument(_)));

        let err2 = CCDataOhlcvQuery::new("ccix", "BTC-", 30)
            .expect_err("empty currency part must be rejected");
        assert!(matches!(err2, CCDataError::InvalidInstrument(_)));
    }

    #[test]
    fn ohlcv_query_rejects_instrument_with_invalid_characters() {
        let err = CCDataOhlcvQuery::new("ccix", "BTC/USD", 30)
            .expect_err("slash in instrument must be rejected");
        assert!(matches!(err, CCDataError::InvalidInstrument(_)));

        let err2 = CCDataOhlcvQuery::new("ccix", "BTC-USD-EUR", 30)
            .expect_err("two dashes must be rejected");
        assert!(matches!(err2, CCDataError::InvalidInstrument(_)));
    }

    #[test]
    fn ohlcv_query_accepts_various_valid_pairs() {
        for pair in ["BTC-USD", "ETH-EUR", "SOL-USDT", "XRP-GBP"] {
            CCDataOhlcvQuery::new("ccix", pair, 7)
                .unwrap_or_else(|e| panic!("{pair} should be valid: {e}"));
        }
    }

    // --- CCDataAssetQuery ---

    #[test]
    fn asset_query_normalizes_symbol_to_uppercase() {
        let q = CCDataAssetQuery::new("btc").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "BTC");
    }

    #[test]
    fn asset_query_rejects_empty_symbol() {
        assert_eq!(CCDataAssetQuery::new(""), Err(CCDataError::EmptySymbol));
        assert_eq!(CCDataAssetQuery::new("  "), Err(CCDataError::EmptySymbol));
    }

    #[test]
    fn asset_query_rejects_symbols_with_invalid_characters() {
        assert_eq!(
            CCDataAssetQuery::new("BTC-USD"),
            Err(CCDataError::InvalidSymbol)
        );
        assert_eq!(
            CCDataAssetQuery::new("BTC/ETH"),
            Err(CCDataError::InvalidSymbol)
        );
        assert_eq!(
            CCDataAssetQuery::new("BTC?x=1"),
            Err(CCDataError::InvalidSymbol)
        );
    }

    // --- Stubs ---

    #[test]
    fn stub_ohlcv_response_contains_instrument_and_entries() {
        let json = stub_ohlcv_response("BTC-USD");
        assert!(json.contains("BTC-USD"));
        assert!(json.contains("TIMESTAMP"));
        assert!(json.contains("42000.0"));
    }

    #[test]
    fn stub_asset_response_contains_symbol_and_market_cap() {
        let json = stub_asset_response("BTC");
        assert!(json.contains("BTC"));
        assert!(json.contains("MARKET_CAP_USD"));
        assert!(json.contains("Bitcoin"));
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_human_readable() {
        assert_eq!(
            CCDataError::EmptyMarket.to_string(),
            "ccdata market must not be empty"
        );
        assert_eq!(
            CCDataError::EmptySymbol.to_string(),
            "ccdata asset symbol must not be empty"
        );
        assert_eq!(
            CCDataError::InvalidSymbol.to_string(),
            "ccdata asset symbol contains unsupported characters"
        );
        assert_eq!(
            CCDataError::MissingApiKey.to_string(),
            "ccdata api key env TDW_CCDATA_API_KEY must be set"
        );
        assert!(
            CCDataError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
        assert!(
            CCDataError::InvalidInstrument("BTCUSD".to_string())
                .to_string()
                .contains("BTCUSD")
        );
    }
}
