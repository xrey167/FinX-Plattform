#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{DatabentoHttpTimeseriesFetcher, DatabentoMetadataFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "databento";
pub const BASE_URL: &str = "https://hist.databento.com/v0";
pub const API_KEY_ENV: &str = "TDW_DATABENTO_API_KEY";

pub type Result<T> = std::result::Result<T, DatabentoError>;

/// Query for Databento historical equity bars (single-symbol convenience wrapper).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabentoHistoricalQuery {
    pub symbol: String,
}

impl DatabentoHistoricalQuery {
    /// Construct a validated historical query.
    ///
    /// # Errors
    ///
    /// Returns [`DatabentoError::EmptySymbol`] when `symbol` is blank.
    pub fn new(symbol: &str) -> Result<Self> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(DatabentoError::EmptySymbol);
        }
        Ok(Self {
            symbol: symbol.to_ascii_uppercase(),
        })
    }
}

/// Query for the Databento timeseries endpoint (`/timeseries.get_range`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabentoTimeseriesQuery {
    /// Databento dataset identifier, e.g. `"GLBX.MDP3"`.
    pub dataset: String,
    /// List of instrument symbols, e.g. `["ESH5"]`.
    pub symbols: Vec<String>,
    /// ISO-8601 date string (`YYYY-MM-DD`) for the range start.
    pub start: String,
    /// ISO-8601 date string (`YYYY-MM-DD`) for the range end.
    pub end: String,
}

impl DatabentoTimeseriesQuery {
    /// Construct a validated timeseries query.
    ///
    /// # Errors
    ///
    /// Returns an error when `dataset` is blank, `symbols` is empty, or
    /// either date is not in `YYYY-MM-DD` format.
    pub fn new(dataset: &str, symbols: Vec<String>, start: &str, end: &str) -> Result<Self> {
        let dataset = dataset.trim();
        if dataset.is_empty() {
            return Err(DatabentoError::Provider(
                "dataset must not be empty".to_string(),
            ));
        }
        if symbols.is_empty() {
            return Err(DatabentoError::Provider(
                "symbols must not be empty".to_string(),
            ));
        }
        for sym in &symbols {
            if sym.trim().is_empty() {
                return Err(DatabentoError::EmptySymbol);
            }
        }
        let start = validate_date(start)?;
        let end = validate_date(end)?;
        Ok(Self {
            dataset: dataset.to_string(),
            symbols,
            start,
            end,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DatabentoError {
    #[error("databento symbol must not be empty")]
    EmptySymbol,
    #[error("databento date must be YYYY-MM-DD")]
    InvalidDate,
    #[error("databento error: {0}")]
    Provider(String),
}

fn validate_date(date: &str) -> Result<String> {
    let date = date.trim();
    let bytes = date.as_bytes();
    let valid = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| idx == 4 || idx == 7 || byte.is_ascii_digit());
    if !valid {
        return Err(DatabentoError::InvalidDate);
    }
    Ok(date.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_query_normalises_symbol() {
        let query =
            DatabentoHistoricalQuery::new("esh5").unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(query.symbol, "ESH5");
    }

    #[test]
    fn historical_query_rejects_empty_symbol() {
        assert_eq!(
            DatabentoHistoricalQuery::new(""),
            Err(DatabentoError::EmptySymbol)
        );
        assert_eq!(
            DatabentoHistoricalQuery::new("   "),
            Err(DatabentoError::EmptySymbol)
        );
    }

    #[test]
    fn timeseries_query_validates_inputs() {
        let q = DatabentoTimeseriesQuery::new(
            "GLBX.MDP3",
            vec!["ESH5".to_string()],
            "2024-01-01",
            "2024-01-31",
        )
        .unwrap_or_else(|e| panic!("should build: {e}"));
        assert_eq!(q.dataset, "GLBX.MDP3");
        assert_eq!(q.symbols, vec!["ESH5"]);
        assert_eq!(q.start, "2024-01-01");
        assert_eq!(q.end, "2024-01-31");
    }

    #[test]
    fn timeseries_query_rejects_empty_dataset() {
        let result =
            DatabentoTimeseriesQuery::new("", vec!["ESH5".to_string()], "2024-01-01", "2024-01-31");
        assert!(result.is_err());
    }

    #[test]
    fn timeseries_query_rejects_empty_symbols() {
        let result = DatabentoTimeseriesQuery::new("GLBX.MDP3", vec![], "2024-01-01", "2024-01-31");
        assert!(result.is_err());
    }

    #[test]
    fn timeseries_query_rejects_invalid_date() {
        let result = DatabentoTimeseriesQuery::new(
            "GLBX.MDP3",
            vec!["ESH5".to_string()],
            "20240101",
            "2024-01-31",
        );
        assert_eq!(result, Err(DatabentoError::InvalidDate));
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert_eq!(
            DatabentoError::EmptySymbol.to_string(),
            "databento symbol must not be empty"
        );
        assert_eq!(
            DatabentoError::InvalidDate.to_string(),
            "databento date must be YYYY-MM-DD"
        );
        assert_eq!(
            DatabentoError::Provider("oops".to_string()).to_string(),
            "databento error: oops"
        );
    }
}
