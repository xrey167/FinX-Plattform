#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    FmpHttpAnalystEstimatesFetcher, FmpHttpCurrencySnapshotsFetcher, FmpHttpDiscoveryFetcher,
    FmpHttpDividendsFetcher, FmpHttpEarningsFetcher, FmpHttpEmployeeCountFetcher,
    FmpHttpEsgScoreFetcher, FmpHttpEtfCountriesFetcher, FmpHttpEtfEquityExposureFetcher,
    FmpHttpEtfInfoFetcher, FmpHttpEtfPricePerformanceFetcher, FmpHttpEtfSearchFetcher,
    FmpHttpEtfSectorsFetcher, FmpHttpExecutiveCompensationFetcher, FmpHttpFilingsFetcher,
    FmpHttpGovernmentTradesFetcher, FmpHttpHistoricalFetcher, FmpHttpHistoricalMarketCapFetcher,
    FmpHttpIncomeFetcher, FmpHttpIndexConstituentsFetcher, FmpHttpInsiderTradingFetcher,
    FmpHttpInstitutionalOwnershipFetcher, FmpHttpKeyExecutivesFetcher, FmpHttpKeyMetricsFetcher,
    FmpHttpLatestFilingsFetcher, FmpHttpPeersFetcher, FmpHttpPriceTargetFetcher,
    FmpHttpProfileFetcher, FmpHttpQuoteSnapshotFetcher, FmpHttpRatiosFetcher,
    FmpHttpRevenueSegmentFetcher, FmpHttpScreenerFetcher, FmpHttpSearchFetcher,
    FmpHttpSplitCalendarFetcher, FmpHttpSplitsFetcher, FmpHttpStatementFetcher,
    FmpHttpTranscriptFetcher,
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

/// Query for the FMP index-constituents endpoint (`/{index}_constituent`).
///
/// FMP publishes a constituent list per major index under a fixed path segment:
/// `sp500_constituent`, `nasdaq_constituent`, `dowjones_constituent`. The query
/// carries the validated index handle (the leading `{index}` path segment); an
/// unknown / missing handle defaults to `sp500`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpIndexConstituentQuery {
    /// Index handle / path segment, e.g. `"sp500"`, `"nasdaq"`, `"dowjones"`.
    pub index: String,
}

impl FmpIndexConstituentQuery {
    /// Construct a query from an optional index handle, defaulting to `sp500`
    /// and normalizing the three supported aliases.
    #[must_use]
    pub fn from_param(value: Option<&str>) -> Self {
        let index = match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("nasdaq" | "nasdaq100" | "ndx") => "nasdaq",
            Some("dowjones" | "dow" | "djia" | "dji") => "dowjones",
            _ => "sp500",
        };
        Self {
            index: index.to_string(),
        }
    }
}

/// Query for the FMP forex snapshot endpoint (`/fx`).
///
/// The `/fx` endpoint returns the full set of FX-pair quotes and takes no path
/// or filter parameter; an optional `symbol` keeps only matching pairs
/// client-side so callers can narrow to one pair without a second request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpFxSnapshotQuery {
    /// Optional `BASE/QUOTE` (or `BASEQUOTE`) filter; empty = every pair.
    #[serde(default)]
    pub symbol: String,
}

impl FmpFxSnapshotQuery {
    /// Construct a snapshot query from an optional pair filter.
    #[must_use]
    pub fn from_param(value: Option<&str>) -> Self {
        Self {
            symbol: value.unwrap_or_default().trim().to_ascii_uppercase(),
        }
    }
}

/// Direction of the FMP stock-market discovery endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FmpDiscoveryDirection {
    /// Largest gainers (`/stock_market/gainers`).
    Gainers,
    /// Largest losers (`/stock_market/losers`).
    Losers,
    /// Most active by volume (`/stock_market/actives`).
    Actives,
}

impl FmpDiscoveryDirection {
    /// Return the FMP `/stock_market/{segment}` path segment for this direction.
    #[must_use]
    pub const fn as_path_segment(self) -> &'static str {
        match self {
            Self::Gainers => "gainers",
            Self::Losers => "losers",
            Self::Actives => "actives",
        }
    }

    /// Parse a discovery direction, defaulting unknown/missing to gainers.
    #[must_use]
    pub fn from_param(value: Option<&str>) -> Self {
        match value {
            Some("losers") => Self::Losers,
            Some("actives" | "active") => Self::Actives,
            _ => Self::Gainers,
        }
    }
}

/// Query for an FMP stock-market discovery list (gainers / losers / actives).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpDiscoveryQuery {
    pub direction: FmpDiscoveryDirection,
}

impl FmpDiscoveryQuery {
    /// Construct a discovery query for `direction`.
    #[must_use]
    pub const fn new(direction: FmpDiscoveryDirection) -> Self {
        Self { direction }
    }
}

/// Query for the FMP stock screener (`/stock-screener`).
///
/// Every filter is optional; an empty query returns the screener's unfiltered
/// default page. `limit` caps the row count when set.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FmpScreenerQuery {
    pub market_cap_more_than: Option<f64>,
    pub market_cap_lower_than: Option<f64>,
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub exchange: Option<String>,
    pub limit: Option<u32>,
}

/// Query for the per-symbol fundamentals endpoints that also take a `limit`
/// (employee-count, SEC filings).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpSymbolLimitQuery {
    pub symbol: String,
    pub limit: u32,
}

impl FmpSymbolLimitQuery {
    /// Construct a validated symbol+limit query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] / [`FmpError::InvalidSymbol`] on a bad
    /// symbol, or [`FmpError::InvalidLimit`] if `limit` is zero.
    pub fn new(symbol: &str, limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            limit,
        })
    }
}

/// Which revenue-segmentation breakdown a [`FmpRevenueSegmentQuery`] requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FmpSegmentKind {
    /// Revenue broken down by business product line.
    Product,
    /// Revenue broken down by geographic region.
    Geography,
}

impl FmpSegmentKind {
    /// Return the FMP `/v4` path segment for this breakdown.
    #[must_use]
    pub const fn as_path_segment(self) -> &'static str {
        match self {
            Self::Product => "revenue-product-segmentation",
            Self::Geography => "revenue-geographic-segmentation",
        }
    }

    /// Return the [`tdw_domain::RevenueSegment`] `kind` discriminator.
    #[must_use]
    pub const fn as_kind(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Geography => "geography",
        }
    }

    /// Parse a segment kind, defaulting unknown/missing to product.
    #[must_use]
    pub fn from_param(value: Option<&str>) -> Self {
        match value {
            Some("geography" | "geographic") => Self::Geography,
            _ => Self::Product,
        }
    }
}

/// Query for the revenue-segmentation endpoints (product / geography).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpRevenueSegmentQuery {
    pub symbol: String,
    pub kind: FmpSegmentKind,
    pub period: FmpPeriod,
}

impl FmpRevenueSegmentQuery {
    /// Construct a validated revenue-segmentation query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] / [`FmpError::InvalidSymbol`] on a bad
    /// symbol.
    pub fn new(symbol: &str, kind: FmpSegmentKind, period: FmpPeriod) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            kind,
            period,
        })
    }
}

/// Query for the earnings-call-transcript endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpTranscriptQuery {
    pub symbol: String,
    pub year: u32,
    pub quarter: u32,
}

impl FmpTranscriptQuery {
    /// Construct a validated transcript query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptySymbol`] / [`FmpError::InvalidSymbol`] on a bad
    /// symbol, or [`FmpError::InvalidTranscriptPeriod`] if `year` or `quarter` is
    /// zero or `quarter` exceeds 4.
    pub fn new(symbol: &str, year: u32, quarter: u32) -> Result<Self> {
        if year == 0 || quarter == 0 || quarter > 4 {
            return Err(FmpError::InvalidTranscriptPeriod);
        }
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
            year,
            quarter,
        })
    }
}

/// Query for the FMP ticker-search endpoint (`/search`).
///
/// `query` is a free-text name/symbol fragment; `limit` caps the result count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpSearchQuery {
    pub query: String,
    pub limit: u32,
}

impl FmpSearchQuery {
    /// Construct a validated search query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::EmptyQuery`] if `query` is blank, or
    /// [`FmpError::InvalidLimit`] if `limit` is zero.
    pub fn new(query: &str, limit: u32) -> Result<Self> {
        let query = query.trim();
        if query.is_empty() {
            return Err(FmpError::EmptyQuery);
        }
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self {
            query: query.to_string(),
            limit,
        })
    }
}

/// Query for the FMP date-range calendar endpoints (e.g. split calendar).
///
/// Both bounds are optional; an empty query returns the calendar's default
/// window. Dates are validated to `YYYY-MM-DD` so they cannot inject extra query
/// parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpCalendarRangeQuery {
    pub from: Option<String>,
    pub to: Option<String>,
}

impl FmpCalendarRangeQuery {
    /// Construct a validated date-range calendar query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::InvalidDate`] if either bound is present and not in
    /// `YYYY-MM-DD` form.
    pub fn new(from: Option<&str>, to: Option<&str>) -> Result<Self> {
        Ok(Self {
            from: from.map(normalize_date).transpose()?,
            to: to.map(normalize_date).transpose()?,
        })
    }
}

/// Query for the symbol-free, limit-only FMP endpoints (e.g. the latest-filings
/// RSS feed).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FmpLimitQuery {
    pub limit: u32,
}

impl FmpLimitQuery {
    /// Construct a validated limit-only query.
    ///
    /// # Errors
    ///
    /// Returns [`FmpError::InvalidLimit`] if `limit` is zero.
    pub const fn new(limit: u32) -> Result<Self> {
        if limit == 0 {
            return Err(FmpError::InvalidLimit);
        }
        Ok(Self { limit })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FmpError {
    #[error("fmp symbol must not be empty")]
    EmptySymbol,
    #[error("fmp search query must not be empty")]
    EmptyQuery,
    #[error("fmp date must be YYYY-MM-DD")]
    InvalidDate,
    #[error("fmp symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("fmp limit must be greater than zero")]
    InvalidLimit,
    #[error("fmp transcript period must have a non-zero year and a quarter in 1..=4")]
    InvalidTranscriptPeriod,
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

/// Validate a `YYYY-MM-DD` date string, returning it unchanged on success.
///
/// Used by the date-range calendar queries so a caller-supplied bound cannot
/// inject extra query parameters into the FMP request URL.
fn normalize_date(date: &str) -> Result<String> {
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
        return Err(FmpError::InvalidDate);
    }
    Ok(date.to_string())
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
    fn index_constituent_query_parses_aliases_with_sp500_default() {
        assert_eq!(
            FmpIndexConstituentQuery::from_param(Some("sp500")).index,
            "sp500"
        );
        assert_eq!(
            FmpIndexConstituentQuery::from_param(Some("NASDAQ")).index,
            "nasdaq"
        );
        assert_eq!(
            FmpIndexConstituentQuery::from_param(Some("dow")).index,
            "dowjones"
        );
        assert_eq!(FmpIndexConstituentQuery::from_param(None).index, "sp500");
        assert_eq!(
            FmpIndexConstituentQuery::from_param(Some("bogus")).index,
            "sp500"
        );
    }

    #[test]
    fn fx_snapshot_query_normalizes_optional_filter() {
        assert_eq!(FmpFxSnapshotQuery::from_param(None).symbol, "");
        assert_eq!(
            FmpFxSnapshotQuery::from_param(Some(" eur/usd ")).symbol,
            "EUR/USD"
        );
        assert_eq!(FmpFxSnapshotQuery::default().symbol, "");
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
            FmpError::InvalidTranscriptPeriod
                .to_string()
                .contains("transcript period")
        );
        assert!(
            FmpError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }

    #[test]
    fn search_query_validates_query_and_limit() {
        let q = FmpSearchQuery::new("  apple ", 10).unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(q.query, "apple");
        assert_eq!(q.limit, 10);
        assert_eq!(FmpSearchQuery::new("", 10), Err(FmpError::EmptyQuery));
        assert_eq!(FmpSearchQuery::new("   ", 10), Err(FmpError::EmptyQuery));
        assert_eq!(FmpSearchQuery::new("apple", 0), Err(FmpError::InvalidLimit));
    }

    #[test]
    fn calendar_range_query_validates_dates() {
        let q = FmpCalendarRangeQuery::new(Some("2024-01-01"), Some("2024-12-31"))
            .unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(q.from.as_deref(), Some("2024-01-01"));
        assert_eq!(q.to.as_deref(), Some("2024-12-31"));
        assert!(FmpCalendarRangeQuery::new(None, None).is_ok());
        assert_eq!(
            FmpCalendarRangeQuery::new(Some("2024/01/01"), None),
            Err(FmpError::InvalidDate)
        );
        assert_eq!(
            FmpCalendarRangeQuery::new(None, Some("20241231")),
            Err(FmpError::InvalidDate)
        );
    }

    #[test]
    fn limit_query_rejects_zero() {
        let q = FmpLimitQuery::new(50).unwrap_or_else(|e| panic!("query: {e}"));
        assert_eq!(q.limit, 50);
        assert_eq!(FmpLimitQuery::new(0), Err(FmpError::InvalidLimit));
    }

    #[test]
    fn transcript_query_validates_year_and_quarter() {
        let q = FmpTranscriptQuery::new("aapl", 2024, 4)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
        assert_eq!(q.year, 2024);
        assert_eq!(q.quarter, 4);
        // Invalid year/quarter yields the dedicated period variant, not the
        // misleading row-limit error.
        assert_eq!(
            FmpTranscriptQuery::new("AAPL", 0, 1),
            Err(FmpError::InvalidTranscriptPeriod)
        );
        assert_eq!(
            FmpTranscriptQuery::new("AAPL", 2024, 0),
            Err(FmpError::InvalidTranscriptPeriod)
        );
        assert_eq!(
            FmpTranscriptQuery::new("AAPL", 2024, 5),
            Err(FmpError::InvalidTranscriptPeriod)
        );
    }
}
