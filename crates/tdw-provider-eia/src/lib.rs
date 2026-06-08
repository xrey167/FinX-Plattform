#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{EiaHttpNaturalGasFetcher, EiaHttpSpotPriceFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "eia";
pub const BASE_URL: &str = "https://api.eia.gov/v2";
pub const API_KEY_PARAM: &str = "api_key";
pub const API_KEY_ENV: &str = "TDW_EIA_API_KEY";

pub type Result<T> = std::result::Result<T, EiaProviderError>;

/// Which energy commodity to query from the EIA spot-price endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EiaCommodity {
    CrudeOilWti,
    CrudeOilBrent,
    NaturalGas,
    Gasoline,
}

/// Query parameters for the EIA petroleum spot-price endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EiaSpotPriceQuery {
    pub commodity: EiaCommodity,
    pub length: u32,
}

impl EiaSpotPriceQuery {
    /// Construct a new spot-price query, validating that `length` is in
    /// the range `1..=10_000`.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError::InvalidLength`] when `length` is zero or
    /// exceeds 10 000.
    pub fn new(commodity: EiaCommodity, length: u32) -> Result<Self> {
        validate_length(length)?;
        Ok(Self { commodity, length })
    }
}

/// Query parameters for the EIA natural-gas price endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EiaNaturalGasQuery {
    pub length: u32,
}

impl EiaNaturalGasQuery {
    /// Construct a new natural-gas query, validating `length`.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError::InvalidLength`] when `length` is zero or
    /// exceeds 10 000.
    pub fn new(length: u32) -> Result<Self> {
        validate_length(length)?;
        Ok(Self { length })
    }
}

/// A single row returned by the EIA petroleum spot-price endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EiaSpotPriceRecord {
    /// ISO-8601 date (daily frequency: `YYYY-MM-DD`).
    pub period: String,
    /// Human-readable product name, e.g. `"Crude Oil WTI"`.
    pub product_name: String,
    /// Numeric value parsed from the API string field.
    pub value: f64,
    /// Unit description, e.g. `"Dollars per Barrel"`.
    pub units: String,
}

/// A single row returned by the EIA natural-gas price endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EiaNaturalGasRecord {
    /// ISO-8601 period (monthly frequency: `YYYY-MM`).
    pub period: String,
    /// Human-readable series description.
    pub series_description: String,
    /// Numeric value parsed from the API string field.
    pub value: f64,
    /// Unit description, e.g. `"Dollars per Million Btu"`.
    pub units: String,
}

/// All errors that can be produced by the EIA provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EiaProviderError {
    #[error("eia query length must be between 1 and 10000, got {0}")]
    InvalidLength(u32),
    #[error("eia api key env {API_KEY_ENV} must be set")]
    MissingApiKey,
    #[error("eia provider error: {0}")]
    Provider(String),
}

const fn validate_length(length: u32) -> Result<()> {
    if length == 0 || length > 10_000 {
        return Err(EiaProviderError::InvalidLength(length));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mock fetcher returning stub data (no HTTP; always available).
// ---------------------------------------------------------------------------

/// Returns a single hard-coded [`EiaSpotPriceRecord`] for offline / test use.
///
/// # Errors
///
/// Propagates any validation error from [`EiaSpotPriceQuery`].
pub fn mock_spot_price(query: &EiaSpotPriceQuery) -> Result<Vec<EiaSpotPriceRecord>> {
    let _ = query; // query is validated at construction time
    Ok(vec![EiaSpotPriceRecord {
        period: "2024-01-02".to_string(),
        product_name: "Crude Oil WTI".to_string(),
        value: 72.36,
        units: "Dollars per Barrel".to_string(),
    }])
}

/// Returns a single hard-coded [`EiaNaturalGasRecord`] for offline / test use.
///
/// # Errors
///
/// Propagates any validation error from [`EiaNaturalGasQuery`].
pub fn mock_natural_gas(query: &EiaNaturalGasQuery) -> Result<Vec<EiaNaturalGasRecord>> {
    let _ = query;
    Ok(vec![EiaNaturalGasRecord {
        period: "2024-01".to_string(),
        series_description: "Henry Hub Natural Gas Spot Price".to_string(),
        value: 2.53,
        units: "Dollars per Million Btu".to_string(),
    }])
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact-value parse assertions; deterministic parser, exact comparison intended
mod tests {
    use super::*;

    // -- EiaSpotPriceQuery validation ----------------------------------------

    #[test]
    fn spot_price_query_accepts_valid_length() {
        let query = EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 10)
            .unwrap_or_else(|e| panic!("valid query must construct: {e}"));
        assert_eq!(query.length, 10);
    }

    #[test]
    fn spot_price_query_rejects_zero_length() {
        let err = EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 0)
            .expect_err("length=0 must be rejected");
        assert_eq!(err, EiaProviderError::InvalidLength(0));
    }

    #[test]
    fn spot_price_query_rejects_length_exceeding_max() {
        let err = EiaSpotPriceQuery::new(EiaCommodity::NaturalGas, 10_001)
            .expect_err("length>10000 must be rejected");
        assert_eq!(err, EiaProviderError::InvalidLength(10_001));
    }

    #[test]
    fn spot_price_query_accepts_boundary_length() {
        EiaSpotPriceQuery::new(EiaCommodity::Gasoline, 10_000)
            .unwrap_or_else(|e| panic!("boundary length 10000 must be accepted: {e}"));
        EiaSpotPriceQuery::new(EiaCommodity::CrudeOilBrent, 1)
            .unwrap_or_else(|e| panic!("boundary length 1 must be accepted: {e}"));
    }

    // -- EiaNaturalGasQuery validation ---------------------------------------

    #[test]
    fn natural_gas_query_accepts_valid_length() {
        let query = EiaNaturalGasQuery::new(12)
            .unwrap_or_else(|e| panic!("valid query must construct: {e}"));
        assert_eq!(query.length, 12);
    }

    #[test]
    fn natural_gas_query_rejects_zero() {
        EiaNaturalGasQuery::new(0).expect_err("length=0 must be rejected");
    }

    // -- Mock fetchers -------------------------------------------------------

    #[test]
    fn mock_spot_price_returns_stub_record() {
        let query = EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 10)
            .unwrap_or_else(|e| panic!("query: {e}"));
        let records = mock_spot_price(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].period, "2024-01-02");
        assert_eq!(records[0].value, 72.36);
    }

    #[test]
    fn mock_natural_gas_returns_stub_record() {
        let query = EiaNaturalGasQuery::new(12).unwrap_or_else(|e| panic!("query: {e}"));
        let records = mock_natural_gas(&query).unwrap_or_else(|e| panic!("mock must succeed: {e}"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].period, "2024-01");
        assert_eq!(records[0].value, 2.53);
    }

    // -- Error display -------------------------------------------------------

    #[test]
    fn error_display_includes_context() {
        assert!(EiaProviderError::InvalidLength(0).to_string().contains('0'));
        assert!(
            EiaProviderError::MissingApiKey
                .to_string()
                .contains(API_KEY_ENV)
        );
        assert!(
            EiaProviderError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
