#![forbid(unsafe_code)]
//! Deribit crypto derivatives REST API provider.
//!
//! No authentication is required for public endpoints.
//! Base URL: `https://www.deribit.com/api/v2`

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    DeribitHttpFundingFetcher, DeribitHttpInstrumentsFetcher, DeribitHttpOrderBookFetcher,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "deribit";
pub const BASE_URL: &str = "https://www.deribit.com/api/v2";

pub type Result<T> = std::result::Result<T, DeribitProviderError>;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The instrument kind filter for `/public/get_instruments`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeribitKind {
    Option,
    Future,
    Perpetual,
    FutureCombo,
    OptionCombo,
}

impl DeribitKind {
    /// Return the lowercase string value used as the `kind` query parameter.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Option => "option",
            Self::Future => "future",
            Self::Perpetual => "perpetual",
            Self::FutureCombo => "future_combo",
            Self::OptionCombo => "option_combo",
        }
    }
}

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Query parameters for `/public/get_instruments`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitInstrumentsQuery {
    /// Currency (e.g. `"BTC"`, `"ETH"`). Normalised to uppercase ASCII.
    pub currency: String,
    /// Instrument kind filter.
    pub kind: DeribitKind,
}

impl DeribitInstrumentsQuery {
    /// Construct and validate a new instruments query.
    ///
    /// # Errors
    ///
    /// Returns [`DeribitProviderError::EmptyCurrency`] when `currency` is blank,
    /// or [`DeribitProviderError::InvalidCurrency`] when it contains non-ASCII
    /// alphabetic characters.
    pub fn new(currency: &str, kind: DeribitKind) -> Result<Self> {
        Ok(Self {
            currency: normalize_currency(currency)?,
            kind,
        })
    }
}

/// Query parameters for `/public/get_order_book`.
///
/// `depth` must be one of 1, 5, 10, or 20.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitOrderBookQuery {
    /// Instrument name (e.g. `"BTC-19JAN24-40000-C"`).
    pub instrument_name: String,
    /// Order book depth — must be 1, 5, 10, or 20.
    pub depth: u32,
}

impl DeribitOrderBookQuery {
    /// Construct and validate a new order-book query.
    ///
    /// # Errors
    ///
    /// Returns [`DeribitProviderError::EmptyInstrumentName`] when
    /// `instrument_name` is blank, [`DeribitProviderError::InvalidInstrumentName`]
    /// when it contains characters outside `[A-Za-z0-9._-]`, or
    /// [`DeribitProviderError::InvalidDepth`] when `depth` is not 1, 5, 10 or 20.
    pub fn new(instrument_name: &str, depth: u32) -> Result<Self> {
        Ok(Self {
            instrument_name: normalize_instrument_name(instrument_name)?,
            depth: validate_depth(depth)?,
        })
    }
}

/// Query parameters for `/public/get_funding_rate_history`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitFundingQuery {
    /// Instrument name (e.g. `"BTC-PERPETUAL"`).
    pub instrument_name: String,
    /// Start of the time range as a Unix timestamp in milliseconds.
    pub start_ms: u64,
    /// End of the time range as a Unix timestamp in milliseconds.
    pub end_ms: u64,
    /// Maximum number of records to return (1–1000).
    pub count: u32,
}

impl DeribitFundingQuery {
    /// Construct and validate a new funding-rate-history query.
    ///
    /// # Errors
    ///
    /// Returns validation errors for instrument name, time range, or count.
    pub fn new(instrument_name: &str, start_ms: u64, end_ms: u64, count: u32) -> Result<Self> {
        let instrument_name = normalize_instrument_name(instrument_name)?;
        if end_ms < start_ms {
            return Err(DeribitProviderError::InvalidTimeRange);
        }
        if count == 0 || count > 1000 {
            return Err(DeribitProviderError::InvalidCount);
        }
        Ok(Self {
            instrument_name,
            start_ms,
            end_ms,
            count,
        })
    }
}

// ---------------------------------------------------------------------------
// Response / domain types
// ---------------------------------------------------------------------------

/// A single active instrument returned by `/public/get_instruments`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitInstrument {
    pub instrument_name: String,
    pub kind: String,
    pub strike: Option<f64>,
    pub expiration_timestamp: Option<u64>,
    pub option_type: Option<String>,
    pub is_active: bool,
}

/// Greeks for an option instrument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitGreeks {
    pub delta: f64,
    pub gamma: f64,
    pub vega: f64,
    pub theta: f64,
}

/// Order book / market data row returned by `/public/get_order_book`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitOrderBook {
    pub instrument_name: String,
    pub bid_iv: Option<f64>,
    pub ask_iv: Option<f64>,
    pub mark_iv: Option<f64>,
    pub underlying_price: Option<f64>,
    /// Best bid levels as `[price, amount]` pairs.
    pub bids: Vec<[f64; 2]>,
    /// Best ask levels as `[price, amount]` pairs.
    pub asks: Vec<[f64; 2]>,
    pub greeks: Option<DeribitGreeks>,
}

/// A single funding-rate record returned by `/public/get_funding_rate_history`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeribitFundingRecord {
    pub timestamp: u64,
    pub interest: f64,
    pub index_price: f64,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DeribitProviderError {
    #[error("deribit currency must not be empty")]
    EmptyCurrency,
    #[error("deribit currency must be ASCII alphabetic characters only")]
    InvalidCurrency,
    #[error("deribit instrument name must not be empty")]
    EmptyInstrumentName,
    #[error("deribit instrument name contains unsupported characters")]
    InvalidInstrumentName,
    #[error("deribit order book depth must be 1, 5, 10, or 20")]
    InvalidDepth,
    #[error("deribit funding query end_ms must be >= start_ms")]
    InvalidTimeRange,
    #[error("deribit funding query count must be between 1 and 1000")]
    InvalidCount,
    #[error("deribit provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Request-path helpers (no I/O — usable offline)
// ---------------------------------------------------------------------------

/// Build the instruments URL path without performing I/O.
///
/// # Errors
///
/// Propagates currency validation errors.
pub fn instruments_request_path(currency: &str, kind: &DeribitKind) -> Result<String> {
    let currency = normalize_currency(currency)?;
    Ok(format!(
        "/public/get_instruments?currency={currency}&kind={}&expired=false",
        kind.as_str()
    ))
}

/// Build the order-book URL path without performing I/O.
///
/// # Errors
///
/// Propagates instrument name and depth validation errors.
pub fn order_book_request_path(instrument_name: &str, depth: u32) -> Result<String> {
    let instrument_name = normalize_instrument_name(instrument_name)?;
    let depth = validate_depth(depth)?;
    Ok(format!(
        "/public/get_order_book?instrument_name={instrument_name}&depth={depth}"
    ))
}

/// Build the funding-rate-history URL path without performing I/O.
///
/// # Errors
///
/// Propagates instrument name and time-range validation errors.
pub fn funding_request_path(
    instrument_name: &str,
    start_ms: u64,
    end_ms: u64,
    count: u32,
) -> Result<String> {
    let instrument_name = normalize_instrument_name(instrument_name)?;
    if end_ms < start_ms {
        return Err(DeribitProviderError::InvalidTimeRange);
    }
    if count == 0 || count > 1000 {
        return Err(DeribitProviderError::InvalidCount);
    }
    Ok(format!(
        "/public/get_funding_rate_history\
         ?instrument_name={instrument_name}\
         &start_timestamp={start_ms}\
         &end_timestamp={end_ms}\
         &count={count}"
    ))
}

// ---------------------------------------------------------------------------
// Mock / offline stub fetchers
// ---------------------------------------------------------------------------

/// Returns a hard-coded stub instruments list for any valid currency + kind.
///
/// # Errors
///
/// Propagates currency validation errors.
pub fn stub_instruments(currency: &str, kind: DeribitKind) -> Result<Vec<DeribitInstrument>> {
    let currency = normalize_currency(currency)?;
    Ok(vec![DeribitInstrument {
        instrument_name: format!("{currency}-19JAN24-40000-C"),
        kind: kind.as_str().to_string(),
        strike: Some(40_000.0),
        expiration_timestamp: Some(1_705_651_200_000),
        option_type: Some("call".to_string()),
        is_active: true,
    }])
}

/// Returns a hard-coded stub order book for any valid instrument name + depth.
///
/// # Errors
///
/// Propagates instrument name or depth validation errors.
pub fn stub_order_book(instrument_name: &str, depth: u32) -> Result<DeribitOrderBook> {
    let instrument_name = normalize_instrument_name(instrument_name)?;
    let _depth = validate_depth(depth)?;
    Ok(DeribitOrderBook {
        instrument_name,
        bid_iv: Some(45.2),
        ask_iv: Some(46.8),
        mark_iv: Some(46.0),
        underlying_price: Some(42_500.0),
        bids: vec![[5.1, 10.0]],
        asks: vec![[5.3, 8.0]],
        greeks: Some(DeribitGreeks {
            delta: 0.35,
            gamma: 0.000_02,
            vega: 85.0,
            theta: -120.0,
        }),
    })
}

/// Returns a hard-coded stub funding-rate record for any valid query.
///
/// # Errors
///
/// Propagates instrument name, time-range, or count validation errors.
pub fn stub_funding_records(
    instrument_name: &str,
    start_ms: u64,
    end_ms: u64,
    count: u32,
) -> Result<Vec<DeribitFundingRecord>> {
    let instrument_name = normalize_instrument_name(instrument_name)?;
    let _query = DeribitFundingQuery::new(&instrument_name, start_ms, end_ms, count)?;
    Ok(vec![DeribitFundingRecord {
        timestamp: start_ms,
        interest: 0.0001,
        index_price: 42_000.0,
    }])
}

// ---------------------------------------------------------------------------
// Validation helpers (private)
// ---------------------------------------------------------------------------

fn normalize_currency(currency: &str) -> Result<String> {
    let currency = currency.trim();
    if currency.is_empty() {
        return Err(DeribitProviderError::EmptyCurrency);
    }
    if !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(DeribitProviderError::InvalidCurrency);
    }
    Ok(currency.to_ascii_uppercase())
}

fn normalize_instrument_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DeribitProviderError::EmptyInstrumentName);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(DeribitProviderError::InvalidInstrumentName);
    }
    Ok(name.to_ascii_uppercase())
}

const fn validate_depth(depth: u32) -> Result<u32> {
    match depth {
        1 | 5 | 10 | 20 => Ok(depth),
        _ => Err(DeribitProviderError::InvalidDepth),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- DeribitKind ---

    #[test]
    fn kind_as_str_returns_correct_values() {
        assert_eq!(DeribitKind::Option.as_str(), "option");
        assert_eq!(DeribitKind::Future.as_str(), "future");
        assert_eq!(DeribitKind::Perpetual.as_str(), "perpetual");
        assert_eq!(DeribitKind::FutureCombo.as_str(), "future_combo");
        assert_eq!(DeribitKind::OptionCombo.as_str(), "option_combo");
    }

    // --- DeribitInstrumentsQuery ---

    #[test]
    fn instruments_query_normalises_currency_to_uppercase() {
        let query = DeribitInstrumentsQuery::new("btc", DeribitKind::Option)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.currency, "BTC");
        assert_eq!(query.kind, DeribitKind::Option);
    }

    #[test]
    fn instruments_query_rejects_empty_currency() {
        assert_eq!(
            DeribitInstrumentsQuery::new("", DeribitKind::Option),
            Err(DeribitProviderError::EmptyCurrency)
        );
        assert_eq!(
            DeribitInstrumentsQuery::new("   ", DeribitKind::Option),
            Err(DeribitProviderError::EmptyCurrency)
        );
    }

    #[test]
    fn instruments_query_rejects_non_alpha_currency() {
        assert_eq!(
            DeribitInstrumentsQuery::new("BTC1", DeribitKind::Option),
            Err(DeribitProviderError::InvalidCurrency)
        );
        assert_eq!(
            DeribitInstrumentsQuery::new("BTC?", DeribitKind::Option),
            Err(DeribitProviderError::InvalidCurrency)
        );
    }

    // --- DeribitOrderBookQuery ---

    #[test]
    fn order_book_query_normalises_instrument_name() {
        let query = DeribitOrderBookQuery::new("btc-19jan24-40000-c", 5)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.instrument_name, "BTC-19JAN24-40000-C");
        assert_eq!(query.depth, 5);
    }

    #[test]
    fn order_book_query_rejects_empty_instrument_name() {
        assert_eq!(
            DeribitOrderBookQuery::new("", 5),
            Err(DeribitProviderError::EmptyInstrumentName)
        );
    }

    #[test]
    fn order_book_query_rejects_injection_characters() {
        assert_eq!(
            DeribitOrderBookQuery::new("BTC-PERP&depth=99", 5),
            Err(DeribitProviderError::InvalidInstrumentName)
        );
    }

    #[test]
    fn order_book_query_accepts_valid_depths() {
        for depth in [1u32, 5, 10, 20] {
            DeribitOrderBookQuery::new("BTC-PERPETUAL", depth)
                .unwrap_or_else(|e| panic!("depth {depth} should be valid: {e}"));
        }
    }

    #[test]
    fn order_book_query_rejects_invalid_depth() {
        assert_eq!(
            DeribitOrderBookQuery::new("BTC-PERPETUAL", 3),
            Err(DeribitProviderError::InvalidDepth)
        );
        assert_eq!(
            DeribitOrderBookQuery::new("BTC-PERPETUAL", 0),
            Err(DeribitProviderError::InvalidDepth)
        );
    }

    // --- DeribitFundingQuery ---

    #[test]
    fn funding_query_builds_with_valid_params() {
        let query = DeribitFundingQuery::new("BTC-PERPETUAL", 1_000_000, 2_000_000, 100)
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(query.instrument_name, "BTC-PERPETUAL");
        assert_eq!(query.start_ms, 1_000_000);
        assert_eq!(query.end_ms, 2_000_000);
        assert_eq!(query.count, 100);
    }

    #[test]
    fn funding_query_rejects_inverted_time_range() {
        assert_eq!(
            DeribitFundingQuery::new("BTC-PERPETUAL", 2_000_000, 1_000_000, 100),
            Err(DeribitProviderError::InvalidTimeRange)
        );
    }

    #[test]
    fn funding_query_rejects_zero_count() {
        assert_eq!(
            DeribitFundingQuery::new("BTC-PERPETUAL", 1_000_000, 2_000_000, 0),
            Err(DeribitProviderError::InvalidCount)
        );
    }

    #[test]
    fn funding_query_rejects_count_exceeding_max() {
        assert_eq!(
            DeribitFundingQuery::new("BTC-PERPETUAL", 1_000_000, 2_000_000, 1001),
            Err(DeribitProviderError::InvalidCount)
        );
    }

    // --- Request path helpers ---

    #[test]
    fn instruments_request_path_contains_currency_and_kind() {
        let path = instruments_request_path("btc", &DeribitKind::Option)
            .unwrap_or_else(|e| panic!("path should build: {e}"));
        assert!(path.contains("currency=BTC"), "path={path}");
        assert!(path.contains("kind=option"), "path={path}");
        assert!(path.contains("expired=false"), "path={path}");
    }

    #[test]
    fn instruments_request_path_rejects_injection() {
        assert!(instruments_request_path("BTC&expired=true", &DeribitKind::Option).is_err());
    }

    #[test]
    fn order_book_request_path_contains_instrument_and_depth() {
        let path = order_book_request_path("BTC-19JAN24-40000-C", 5)
            .unwrap_or_else(|e| panic!("path should build: {e}"));
        assert!(
            path.contains("instrument_name=BTC-19JAN24-40000-C"),
            "path={path}"
        );
        assert!(path.contains("depth=5"), "path={path}");
    }

    #[test]
    fn funding_request_path_contains_all_params() {
        let path = funding_request_path("BTC-PERPETUAL", 1_704_153_600_000, 1_704_240_000_000, 100)
            .unwrap_or_else(|e| panic!("path should build: {e}"));
        assert!(
            path.contains("instrument_name=BTC-PERPETUAL"),
            "path={path}"
        );
        assert!(
            path.contains("start_timestamp=1704153600000"),
            "path={path}"
        );
        assert!(path.contains("end_timestamp=1704240000000"), "path={path}");
        assert!(path.contains("count=100"), "path={path}");
    }

    // --- Stub fetchers ---

    #[test]
    fn stub_instruments_returns_active_contract() {
        let instruments = stub_instruments("BTC", DeribitKind::Option)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert!(!instruments.is_empty());
        assert!(instruments[0].instrument_name.starts_with("BTC"));
        assert!(instruments[0].is_active);
    }

    #[test]
    fn stub_instruments_propagates_currency_error() {
        assert!(stub_instruments("", DeribitKind::Option).is_err());
        assert!(stub_instruments("BTC1", DeribitKind::Option).is_err());
    }

    #[test]
    fn stub_order_book_returns_bids_and_asks() {
        let book = stub_order_book("BTC-19JAN24-40000-C", 5)
            .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert_eq!(book.instrument_name, "BTC-19JAN24-40000-C");
        assert!(!book.bids.is_empty());
        assert!(!book.asks.is_empty());
        assert!(book.greeks.is_some());
    }

    #[test]
    fn stub_order_book_propagates_depth_error() {
        assert!(stub_order_book("BTC-PERPETUAL", 3).is_err());
    }

    #[test]
    fn stub_funding_records_returns_entry() {
        let records =
            stub_funding_records("BTC-PERPETUAL", 1_704_153_600_000, 1_704_240_000_000, 100)
                .unwrap_or_else(|e| panic!("stub should succeed: {e}"));
        assert!(!records.is_empty());
        assert!(records[0].interest > 0.0);
    }

    #[test]
    fn stub_funding_records_propagates_time_range_error() {
        assert!(stub_funding_records("BTC-PERPETUAL", 2_000_000, 1_000_000, 100).is_err());
    }

    // --- Error messages ---

    #[test]
    fn error_messages_are_descriptive() {
        assert_eq!(
            DeribitProviderError::EmptyCurrency.to_string(),
            "deribit currency must not be empty"
        );
        assert_eq!(
            DeribitProviderError::InvalidDepth.to_string(),
            "deribit order book depth must be 1, 5, 10, or 20"
        );
        assert_eq!(
            DeribitProviderError::InvalidTimeRange.to_string(),
            "deribit funding query end_ms must be >= start_ms"
        );
    }
}
