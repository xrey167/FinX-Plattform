#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{TradingEconomicsHttpCalendarFetcher, TradingEconomicsHttpIndicatorFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "trading_economics";
pub const BASE_URL: &str = "https://api.tradingeconomics.com";
pub const API_KEY_ENV: &str = "TDW_TRADING_ECONOMICS_API_KEY";

pub type Result<T> = std::result::Result<T, TradingEconomicsError>;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TradingEconomicsError {
    #[error("trading economics: importance_min must be between 1 and 3, got {0}")]
    InvalidImportance(u8),
    #[error("trading economics: country must not be empty")]
    EmptyCountry,
    #[error("trading economics: indicator must not be empty")]
    EmptyIndicator,
    #[error("trading economics: country contains unsupported characters")]
    InvalidCountry,
    #[error("trading economics: indicator contains unsupported characters")]
    InvalidIndicator,
    #[error("trading economics provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Query structs
// ---------------------------------------------------------------------------

/// Query for the economic calendar endpoint.
///
/// `importance_min` filters events to those with importance >= the given
/// value. Importance runs from 1 (low) to 3 (high).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradingEconomicsCalendarQuery {
    /// Minimum importance level to include (1–3).
    pub importance_min: u8,
}

impl TradingEconomicsCalendarQuery {
    /// Construct a validated calendar query.
    ///
    /// # Errors
    ///
    /// Returns [`TradingEconomicsError::InvalidImportance`] if
    /// `importance_min` is 0 or greater than 3.
    pub const fn new(importance_min: u8) -> Result<Self> {
        if importance_min == 0 || importance_min > 3 {
            return Err(TradingEconomicsError::InvalidImportance(importance_min));
        }
        Ok(Self { importance_min })
    }
}

/// Query for the country indicator endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradingEconomicsIndicatorQuery {
    /// Country name in URL-safe form (e.g. `"united+states"`).
    pub country: String,
    /// Indicator slug (e.g. `"gdp-growth-rate"`).
    pub indicator: String,
}

impl TradingEconomicsIndicatorQuery {
    /// Construct a validated indicator query.
    ///
    /// # Errors
    ///
    /// Returns [`TradingEconomicsError::EmptyCountry`],
    /// [`TradingEconomicsError::InvalidCountry`],
    /// [`TradingEconomicsError::EmptyIndicator`], or
    /// [`TradingEconomicsError::InvalidIndicator`] on bad input.
    pub fn new(country: &str, indicator: &str) -> Result<Self> {
        let country = validate_country(country)?;
        let indicator = validate_indicator(indicator)?;
        Ok(Self { country, indicator })
    }
}

// ---------------------------------------------------------------------------
// Response row types
// ---------------------------------------------------------------------------

/// A single economic calendar event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradingEconomicsCalendarEvent {
    pub date: String,
    pub country: String,
    pub category: String,
    pub event: String,
    pub actual: String,
    pub previous: String,
    pub forecast: String,
    pub te_forecast: String,
    pub importance: u8,
}

/// A single country indicator observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TradingEconomicsIndicatorRow {
    pub country: String,
    pub category: String,
    pub date_time: String,
    pub value: String,
    pub frequency: String,
    pub historical_data_symbol: String,
}

// ---------------------------------------------------------------------------
// Mock fetcher (offline, for unit tests and feature-flag-off builds)
// ---------------------------------------------------------------------------

/// Offline stub returning a single hardcoded calendar event.
pub struct TradingEconomicsMockCalendarFetcher;

impl TradingEconomicsMockCalendarFetcher {
    /// Return one hardcoded high-importance event.
    ///
    /// # Errors
    ///
    /// Never errors; the signature mirrors the real fetcher.
    pub fn fetch_stub(
        _query: &TradingEconomicsCalendarQuery,
    ) -> Result<Vec<TradingEconomicsCalendarEvent>> {
        Ok(vec![TradingEconomicsCalendarEvent {
            date: "2024-01-05T13:30:00".to_string(),
            country: "United States".to_string(),
            category: "Non Farm Payrolls".to_string(),
            event: "Non Farm Payrolls".to_string(),
            actual: String::new(),
            previous: "199000".to_string(),
            forecast: "170000".to_string(),
            te_forecast: "175000".to_string(),
            importance: 3,
        }])
    }
}

/// Offline stub returning a single hardcoded indicator row.
pub struct TradingEconomicsMockIndicatorFetcher;

impl TradingEconomicsMockIndicatorFetcher {
    /// Return one hardcoded GDP growth rate row.
    ///
    /// # Errors
    ///
    /// Never errors; the signature mirrors the real fetcher.
    pub fn fetch_stub(
        _query: &TradingEconomicsIndicatorQuery,
    ) -> Result<Vec<TradingEconomicsIndicatorRow>> {
        Ok(vec![TradingEconomicsIndicatorRow {
            country: "United States".to_string(),
            category: "GDP Growth Rate".to_string(),
            date_time: "2023-10-01".to_string(),
            value: "1.2".to_string(),
            frequency: "Quarter".to_string(),
            historical_data_symbol: "UNITEDSTAGSQDP".to_string(),
        }])
    }
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_country(country: &str) -> Result<String> {
    let country = country.trim();
    if country.is_empty() {
        return Err(TradingEconomicsError::EmptyCountry);
    }
    // Allow ASCII alphanumeric, spaces, hyphens, plus signs (URL form), dots
    if !country
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '+' | '-' | '.'))
    {
        return Err(TradingEconomicsError::InvalidCountry);
    }
    Ok(country.to_string())
}

fn validate_indicator(indicator: &str) -> Result<String> {
    let indicator = indicator.trim();
    if indicator.is_empty() {
        return Err(TradingEconomicsError::EmptyIndicator);
    }
    // Allow ASCII alphanumeric, hyphens, underscores (URL slug form)
    if !indicator
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(TradingEconomicsError::InvalidIndicator);
    }
    Ok(indicator.to_string())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_query_accepts_valid_importance_levels() {
        assert!(TradingEconomicsCalendarQuery::new(1).is_ok());
        assert!(TradingEconomicsCalendarQuery::new(2).is_ok());
        assert!(TradingEconomicsCalendarQuery::new(3).is_ok());
    }

    #[test]
    fn calendar_query_rejects_zero_and_out_of_range() {
        assert_eq!(
            TradingEconomicsCalendarQuery::new(0),
            Err(TradingEconomicsError::InvalidImportance(0))
        );
        assert_eq!(
            TradingEconomicsCalendarQuery::new(4),
            Err(TradingEconomicsError::InvalidImportance(4))
        );
    }

    #[test]
    fn indicator_query_accepts_valid_country_and_indicator() {
        let query = TradingEconomicsIndicatorQuery::new("united+states", "gdp-growth-rate")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.country, "united+states");
        assert_eq!(query.indicator, "gdp-growth-rate");
    }

    #[test]
    fn indicator_query_rejects_empty_country() {
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("", "gdp"),
            Err(TradingEconomicsError::EmptyCountry)
        );
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("   ", "gdp"),
            Err(TradingEconomicsError::EmptyCountry)
        );
    }

    #[test]
    fn indicator_query_rejects_empty_indicator() {
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("united+states", ""),
            Err(TradingEconomicsError::EmptyIndicator)
        );
    }

    #[test]
    fn indicator_query_rejects_invalid_characters_in_country() {
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("united?states", "gdp"),
            Err(TradingEconomicsError::InvalidCountry)
        );
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("united&states=x", "gdp"),
            Err(TradingEconomicsError::InvalidCountry)
        );
    }

    #[test]
    fn indicator_query_rejects_invalid_characters_in_indicator() {
        assert_eq!(
            TradingEconomicsIndicatorQuery::new("united+states", "gdp?hack=1"),
            Err(TradingEconomicsError::InvalidIndicator)
        );
    }

    #[test]
    fn mock_calendar_fetcher_returns_stub_event() {
        let query = TradingEconomicsCalendarQuery::new(3).unwrap_or_else(|e| panic!("query: {e}"));
        let events = TradingEconomicsMockCalendarFetcher::fetch_stub(&query)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].importance, 3);
        assert_eq!(events[0].country, "United States");
    }

    #[test]
    fn mock_indicator_fetcher_returns_stub_row() {
        let query = TradingEconomicsIndicatorQuery::new("united+states", "gdp-growth-rate")
            .unwrap_or_else(|e| panic!("query: {e}"));
        let rows = TradingEconomicsMockIndicatorFetcher::fetch_stub(&query)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "1.2");
        assert_eq!(rows[0].frequency, "Quarter");
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            TradingEconomicsError::InvalidImportance(5)
                .to_string()
                .contains("importance_min")
        );
        assert!(
            TradingEconomicsError::EmptyCountry
                .to_string()
                .contains("country")
        );
        assert!(
            TradingEconomicsError::EmptyIndicator
                .to_string()
                .contains("indicator")
        );
        assert!(
            TradingEconomicsError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
