#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    FmpHttpDividendsFetcher, FmpHttpEarningsFetcher, FmpHttpHistoricalFetcher,
    FmpHttpIncomeFetcher, FmpHttpKeyMetricsFetcher, FmpHttpPeersFetcher, FmpHttpProfileFetcher,
    FmpHttpQuoteSnapshotFetcher, FmpHttpRatiosFetcher, FmpHttpSplitsFetcher,
    FmpHttpStatementFetcher,
};

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

    /// Return the FMP API path segment for this statement's growth-rate sibling.
    #[must_use]
    pub const fn as_growth_path_segment(self) -> &'static str {
        match self {
            Self::Income => "income-statement-growth",
            Self::Balance => "balance-sheet-statement-growth",
            Self::Cashflow => "cash-flow-statement-growth",
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

/// Reporting period for the fundamentals cluster endpoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FmpPeriod {
    Annual,
    Quarter,
}

impl FmpPeriod {
    /// Return the FMP `period` query-parameter value for this variant.
    #[must_use]
    pub const fn as_param(self) -> &'static str {
        match self {
            Self::Annual => "annual",
            Self::Quarter => "quarter",
        }
    }

    /// Parse an FMP `period` value, defaulting unknown/missing to annual.
    #[must_use]
    pub fn from_param(value: Option<&str>) -> Self {
        match value {
            Some("quarter") => Self::Quarter,
            _ => Self::Annual,
        }
    }
}

/// Query for a financial statement (balance / income / cash) and its optional
/// growth-rate sibling from FMP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpStatementQuery {
    pub symbol: String,
    pub statement: FmpStatement,
    /// When `true`, hit the `*-statement-growth` endpoint instead of the
    /// statement itself.
    pub growth: bool,
    pub period: FmpPeriod,
    pub limit: u32,
}

impl FmpStatementQuery {
    /// Construct a validated statement query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] / [`FmpError::InvalidSymbol`] on a bad
    /// symbol, or [`FmpError::InvalidLimit`] if `limit` is zero.
    pub fn new(
        symbol: &str,
        statement: FmpStatement,
        growth: bool,
        period: FmpPeriod,
        limit: u32,
    ) -> Result<Self> {
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            statement,
            growth,
            period,
            limit,
        })
    }
}

/// Query for the per-period fundamentals endpoints that take a `period` and
/// `limit` (key-metrics, ratios).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpFundamentalQuery {
    pub symbol: String,
    pub period: FmpPeriod,
    pub limit: u32,
}

impl FmpFundamentalQuery {
    /// Construct a validated per-period fundamentals query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] / [`FmpError::InvalidSymbol`] on a bad
    /// symbol, or [`FmpError::InvalidLimit`] if `limit` is zero.
    pub fn new(symbol: &str, period: FmpPeriod, limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            period,
            limit,
        })
    }
}

/// Query for the symbol-only fundamentals endpoints (peers, profile,
/// dividends, splits, historical earnings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpSymbolQuery {
    pub symbol: String,
}

impl FmpSymbolQuery {
    /// Construct a validated symbol-only query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] if `symbol` is blank, or
    /// [`FmpError::InvalidSymbol`] if it contains unsupported characters.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
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

/// Query for a last-price quote snapshot from the FMP `/quote/{symbol}` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpQuoteQuery {
    pub symbol: String,
}

impl FmpQuoteQuery {
    /// Construct a validated quote query.
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

/// Offline stub for the quote-snapshot endpoint.
pub struct FmpMockQuoteSnapshotFetcher;

impl FmpMockQuoteSnapshotFetcher {
    /// Return one hardcoded [`QuoteSnapshotRow`] for the queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FmpQuoteQuery) -> Result<Vec<QuoteSnapshotRow>> {
        Ok(vec![QuoteSnapshotRow {
            symbol: query.symbol.clone(),
            price: 189.30,
            change: 1.20,
            changes_percentage: 0.638,
            previous_close: 188.10,
            timestamp: 1_717_200_000,
        }])
    }
}

/// A single row returned by the FMP `/quote/{symbol}` endpoint, used to build
/// a [`tdw_domain::QuoteSnapshot`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QuoteSnapshotRow {
    pub symbol: String,
    pub price: f64,
    pub change: f64,
    #[serde(rename = "changesPercentage")]
    pub changes_percentage: f64,
    #[serde(rename = "previousClose")]
    pub previous_close: f64,
    /// Unix epoch in seconds (FMP returns seconds; multiplied by 1000 for
    /// `ts_ms` in [`tdw_domain::QuoteSnapshot`]).
    pub timestamp: i64,
}

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[allow(clippy::float_cmp)] // exact-value parse assertions; deterministic parser, exact comparison intended
mod tests {
    use super::*;

    #[test]
    fn historical_query_normalises_symbol_to_uppercase() {
        let query =
            FmpHistoricalQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
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
        assert_eq!(
            FmpStatement::Balance.as_path_segment(),
            "balance-sheet-statement"
        );
        assert_eq!(
            FmpStatement::Cashflow.as_path_segment(),
            "cash-flow-statement"
        );
    }

    #[test]
    fn quote_query_normalises_symbol() {
        let q = FmpQuoteQuery::new("aapl").unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn quote_query_rejects_empty_symbol() {
        assert_eq!(FmpQuoteQuery::new(""), Err(FmpError::EmptySymbol));
    }

    #[test]
    fn mock_quote_snapshot_fetcher_returns_stub_row() {
        let q = FmpQuoteQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
        let rows =
            FmpMockQuoteSnapshotFetcher::fetch_stub(&q).unwrap_or_else(|e| panic!("stub: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].price, 189.30);
        assert_eq!(rows[0].timestamp, 1_717_200_000);
    }

    #[test]
    fn mock_historical_fetcher_returns_stub_row() {
        let query = FmpHistoricalQuery::new("MSFT").unwrap_or_else(|e| panic!("query: {e}"));
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
    fn statement_query_validates_and_selects_growth_segment() {
        let q = FmpStatementQuery::new("aapl", FmpStatement::Balance, true, FmpPeriod::Quarter, 3)
            .unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(q.symbol, "AAPL");
        assert!(q.growth);
        assert_eq!(q.period, FmpPeriod::Quarter);
        assert_eq!(q.period.as_param(), "quarter");
        assert_eq!(
            q.statement.as_growth_path_segment(),
            "balance-sheet-statement-growth"
        );
        assert_eq!(
            FmpStatementQuery::new("AAPL", FmpStatement::Income, false, FmpPeriod::Annual, 0),
            Err(FmpError::InvalidLimit)
        );
    }

    #[test]
    fn fundamental_and_symbol_queries_validate() {
        let f = FmpFundamentalQuery::new("aapl", FmpPeriod::Annual, 5)
            .unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(f.symbol, "AAPL");
        assert_eq!(f.period, FmpPeriod::Annual);
        assert_eq!(
            FmpFundamentalQuery::new("AAPL", FmpPeriod::Annual, 0),
            Err(FmpError::InvalidLimit)
        );

        let s = FmpSymbolQuery::new("brk.b").unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(s.symbol, "BRK.B");
        assert_eq!(FmpSymbolQuery::new(""), Err(FmpError::EmptySymbol));
    }

    #[test]
    fn period_parses_param_with_annual_default() {
        assert_eq!(FmpPeriod::from_param(Some("quarter")), FmpPeriod::Quarter);
        assert_eq!(FmpPeriod::from_param(Some("annual")), FmpPeriod::Annual);
        assert_eq!(FmpPeriod::from_param(None), FmpPeriod::Annual);
        assert_eq!(FmpPeriod::from_param(Some("bogus")), FmpPeriod::Annual);
    }

    #[test]
    fn fmp_error_messages_are_descriptive() {
        assert!(FmpError::EmptySymbol.to_string().contains("empty"));
        assert!(FmpError::InvalidSymbol.to_string().contains("unsupported"));
        assert!(
            FmpError::InvalidLimit
                .to_string()
                .contains("greater than zero")
        );
        assert!(
            FmpError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }
}
