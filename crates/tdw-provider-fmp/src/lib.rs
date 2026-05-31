#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{FmpHttpHistoricalFetcher, FmpHttpIncomeFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "fmp";
pub const BASE_URL: &str = "https://financialmodelingprep.com/api/v3";
pub const API_KEY_ENV: &str = "TDW_FMP_API_KEY";

pub type Result<T> = std::result::Result<T, FmpError>;

/// Query for daily OHLCV bars from the FMP historical-price-full endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpHistoricalQuery {
    pub symbol: String,
}

impl FmpHistoricalQuery {
    /// Construct a validated query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] if `symbol` is blank, or
    /// [`FmpError::InvalidSymbol`] if it contains characters that are not
    /// ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Valid values for `statement` in [`FmpFundamentalsQuery`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FmpStatement {
    Income,
    Balance,
    Cashflow,
}

impl FmpStatement {
    /// Return the FMP API path segment for this statement type.
    #[must_use]
    pub const fn as_path_segment(self) -> &'static str {
        match self {
            Self::Income => "income-statement",
            Self::Balance => "balance-sheet-statement",
            Self::Cashflow => "cash-flow-statement",
        }
    }
}

/// Query for fundamental financial statements from FMP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpFundamentalsQuery {
    pub symbol: String,
    pub statement: FmpStatement,
    pub limit: u32,
}

impl FmpFundamentalsQuery {
    /// Construct a validated fundamentals query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] or [`FmpError::InvalidSymbol`] on bad
    /// input, and [`FmpError::InvalidLimit`] if `limit` is zero.
    pub fn new(symbol: &str, statement: FmpStatement, limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            statement,
            limit,
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FmpError {
    #[error("fmp symbol must not be empty")]
    EmptySymbol,
    #[error("fmp symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("fmp limit must be greater than zero")]
    InvalidLimit,
    #[error("fmp provider error: {0}")]
    Provider(String),
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(FmpError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(FmpError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

// ---------------------------------------------------------------------------
// Mock fetcher (offline, for unit tests and feature-flag-off builds)
// ---------------------------------------------------------------------------

/// Offline stub that returns a single hardcoded bar. Used in tests that do
/// not enable the `http` feature.
pub struct FmpMockHistoricalFetcher;

impl FmpMockHistoricalFetcher {
    /// Return one hardcoded OHLCV row for the queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FmpHistoricalQuery) -> Result<Vec<FmpOhlcvRow>> {
        Ok(vec![FmpOhlcvRow {
            date: "2024-01-02".to_string(),
            open: 185.6,
            high: 186.1,
            low: 184.4,
            close: 185.2,
            volume: 55_000_000,
            symbol: query.symbol.clone(),
        }])
    }
}

/// Offline stub for the income-statement endpoint.
pub struct FmpMockIncomeFetcher;

impl FmpMockIncomeFetcher {
    /// Return one hardcoded income-statement row.
    ///
    /// # Errors
    ///
    /// Never errors; signature mirrors the real fetcher.
    pub fn fetch_stub(query: &FmpFundamentalsQuery) -> Result<Vec<FmpIncomeRow>> {
        Ok(vec![FmpIncomeRow {
            date: "2024-09-28".to_string(),
            symbol: query.symbol.clone(),
            revenue: 391_035_000_000,
            gross_profit: 180_683_000_000,
            net_income: 93_736_000_000,
        }])
    }
}

/// A single OHLCV bar returned by the historical endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FmpOhlcvRow {
    pub symbol: String,
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
}

/// A single income-statement row returned by the fundamentals endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FmpIncomeRow {
    pub symbol: String,
    pub date: String,
    pub revenue: i64,
    pub gross_profit: i64,
    pub net_income: i64,
}

// ---------------------------------------------------------------------------
// Unit tests (no feature gate required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_query_normalises_symbol_to_uppercase() {
        let query = FmpHistoricalQuery::new("aapl")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.symbol, "AAPL");
    }

    #[test]
    fn historical_query_rejects_empty_symbol() {
        assert_eq!(FmpHistoricalQuery::new(""), Err(FmpError::EmptySymbol));
        assert_eq!(FmpHistoricalQuery::new("   "), Err(FmpError::EmptySymbol));
    }

    #[test]
    fn historical_query_rejects_invalid_characters() {
        assert_eq!(
            FmpHistoricalQuery::new("AAPL/../../secret"),
            Err(FmpError::InvalidSymbol)
        );
        assert_eq!(
            FmpHistoricalQuery::new("AAPL?adjusted=false"),
            Err(FmpError::InvalidSymbol)
        );
    }

    #[test]
    fn historical_query_accepts_dots_dashes_underscores() {
        assert!(FmpHistoricalQuery::new("BRK.B").is_ok());
        assert!(FmpHistoricalQuery::new("BRK-B").is_ok());
        assert!(FmpHistoricalQuery::new("SPY_ETF").is_ok());
    }

    #[test]
    fn fundamentals_query_rejects_zero_limit() {
        assert_eq!(
            FmpFundamentalsQuery::new("AAPL", FmpStatement::Income, 0),
            Err(FmpError::InvalidLimit)
        );
    }

    #[test]
    fn fundamentals_query_builds_valid_income_query() {
        let query = FmpFundamentalsQuery::new("aapl", FmpStatement::Income, 5)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.symbol, "AAPL");
        assert_eq!(query.statement, FmpStatement::Income);
        assert_eq!(query.limit, 5);
        assert_eq!(query.statement.as_path_segment(), "income-statement");
    }

    #[test]
    fn fundamentals_statement_path_segments_are_correct() {
        assert_eq!(FmpStatement::Balance.as_path_segment(), "balance-sheet-statement");
        assert_eq!(FmpStatement::Cashflow.as_path_segment(), "cash-flow-statement");
    }

    #[test]
    fn mock_historical_fetcher_returns_stub_row() {
        let query =
            FmpHistoricalQuery::new("MSFT").unwrap_or_else(|e| panic!("query: {e}"));
        let rows = FmpMockHistoricalFetcher::fetch_stub(&query)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "MSFT");
        assert_eq!(rows[0].date, "2024-01-02");
    }

    #[test]
    fn mock_income_fetcher_returns_stub_row() {
        let query = FmpFundamentalsQuery::new("AAPL", FmpStatement::Income, 1)
            .unwrap_or_else(|e| panic!("query: {e}"));
        let rows = FmpMockIncomeFetcher::fetch_stub(&query)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].revenue, 391_035_000_000);
    }

    #[test]
    fn fmp_error_messages_are_descriptive() {
        assert!(FmpError::EmptySymbol.to_string().contains("empty"));
        assert!(FmpError::InvalidSymbol.to_string().contains("unsupported"));
        assert!(FmpError::InvalidLimit.to_string().contains("greater than zero"));
        assert!(
            FmpError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
