#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::AkShareHttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "akshare";
pub const BASE_URL: &str = "https://akshare.akfamily.xyz";

pub type Result<T> = std::result::Result<T, AkShareError>;

/// Which exchange market the symbol belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AkShareMarket {
    /// Mainland China A-share markets (6-digit symbol codes).
    AShares,
    /// Hong Kong equity market (5-digit symbol codes).
    HongKong,
}

impl AkShareMarket {
    /// Returns the AkShare endpoint path for this market.
    #[must_use]
    pub const fn endpoint_path(self) -> &'static str {
        match self {
            Self::AShares => "/api/public/stock_zh_a_hist",
            Self::HongKong => "/api/public/stock_hk_hist",
        }
    }

    /// Returns the canonical venue string for this market.
    #[must_use]
    pub const fn venue(self) -> &'static str {
        match self {
            Self::AShares => "akshare_a",
            Self::HongKong => "akshare_hk",
        }
    }

    /// Expected symbol length for this market.
    #[must_use]
    pub const fn expected_symbol_len(self) -> usize {
        match self {
            Self::AShares => 6,
            Self::HongKong => 5,
        }
    }
}

/// Query parameters for an AkShare historical OHLCV request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AkShareQuery {
    /// Exchange symbol (A-shares: 6 ASCII digits; HK: 5 ASCII digits).
    pub symbol: String,
    /// Target market.
    pub market: AkShareMarket,
    /// Start date in `YYYYMMDD` format.
    pub start_date: String,
    /// End date in `YYYYMMDD` format.
    pub end_date: String,
}

impl AkShareQuery {
    /// Construct and validate a new `AkShareQuery`.
    ///
    /// # Errors
    ///
    /// Returns an error if `symbol`, `start_date`, or `end_date` fail validation.
    pub fn new(
        symbol: &str,
        market: AkShareMarket,
        start_date: &str,
        end_date: &str,
    ) -> Result<Self> {
        Ok(Self {
            symbol: validate_symbol(symbol, market)?,
            market,
            start_date: validate_yyyymmdd(start_date)?,
            end_date: validate_yyyymmdd(end_date)?,
        })
    }
}

/// Errors produced by the AkShare provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AkShareError {
    #[error("akshare symbol must be ASCII digits only and {0} characters for the selected market")]
    InvalidSymbol(usize),
    #[error("akshare date must be in YYYYMMDD format (8 ASCII digits)")]
    InvalidDate,
    #[error("akshare provider error: {0}")]
    Provider(String),
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_symbol(symbol: &str, market: AkShareMarket) -> Result<String> {
    let symbol = symbol.trim();
    let expected = market.expected_symbol_len();
    let valid = (symbol.len() == expected
        || (market == AkShareMarket::AShares && symbol.len() == 5))
        && symbol.chars().all(|ch| ch.is_ascii_digit());
    if !valid {
        return Err(AkShareError::InvalidSymbol(expected));
    }
    Ok(symbol.to_string())
}

fn validate_yyyymmdd(date: &str) -> Result<String> {
    let date = date.trim();
    let valid = date.len() == 8 && date.chars().all(|ch| ch.is_ascii_digit());
    if !valid {
        return Err(AkShareError::InvalidDate);
    }
    Ok(date.to_string())
}

// ---------------------------------------------------------------------------
// Mock / offline fetcher (no HTTP dependency)
// ---------------------------------------------------------------------------

/// Returns a two-bar stub for offline/unit-test use.
///
/// # Errors
///
/// Returns `Err` if the query parameters fail validation.
pub fn fetch_stub(query: &AkShareQuery) -> Result<Vec<StubBar>> {
    Ok(vec![
        StubBar {
            date: format!("{}-{}-02", &query.start_date[..4], &query.start_date[4..6]),
            open: 10.5,
            close: 10.8,
            high: 10.9,
            low: 10.4,
            volume: 8_500_000.0,
        },
        StubBar {
            date: format!("{}-{}-03", &query.start_date[..4], &query.start_date[4..6]),
            open: 10.8,
            close: 11.0,
            high: 11.2,
            low: 10.7,
            volume: 9_200_000.0,
        },
    ])
}

/// A lightweight offline bar record returned by the mock fetcher.
#[derive(Clone, Debug, PartialEq)]
pub struct StubBar {
    pub date: String,
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- symbol validation ---------------------------------------------------

    #[test]
    fn accepts_valid_a_share_symbol() {
        let query = AkShareQuery::new("000001", AkShareMarket::AShares, "20240101", "20240131")
            .unwrap_or_else(|e| panic!("should accept valid A-share symbol: {e}"));
        assert_eq!(query.symbol, "000001");
    }

    #[test]
    fn accepts_valid_hk_symbol() {
        let query = AkShareQuery::new("00700", AkShareMarket::HongKong, "20240101", "20240131")
            .unwrap_or_else(|e| panic!("should accept valid HK symbol: {e}"));
        assert_eq!(query.symbol, "00700");
    }

    #[test]
    fn rejects_empty_symbol() {
        assert!(
            AkShareQuery::new("", AkShareMarket::AShares, "20240101", "20240131").is_err(),
            "empty symbol must be rejected"
        );
    }

    #[test]
    fn rejects_symbol_with_non_digit_chars() {
        assert!(
            AkShareQuery::new("00A001", AkShareMarket::AShares, "20240101", "20240131").is_err(),
            "non-digit chars must be rejected"
        );
    }

    #[test]
    fn rejects_hk_symbol_wrong_length() {
        // 6 digits is valid for A-shares but not for HK (expects 5)
        assert!(
            AkShareQuery::new("007000", AkShareMarket::HongKong, "20240101", "20240131").is_err(),
            "6-digit symbol must be rejected for HK market"
        );
    }

    // -- date validation -----------------------------------------------------

    #[test]
    fn accepts_valid_yyyymmdd_date() {
        let query = AkShareQuery::new("000001", AkShareMarket::AShares, "20240101", "20240131")
            .unwrap_or_else(|e| panic!("should accept valid dates: {e}"));
        assert_eq!(query.start_date, "20240101");
        assert_eq!(query.end_date, "20240131");
    }

    #[test]
    fn rejects_date_with_dashes() {
        assert!(
            AkShareQuery::new("000001", AkShareMarket::AShares, "2024-01-01", "2024-01-31")
                .is_err(),
            "YYYY-MM-DD format must be rejected (requires YYYYMMDD)"
        );
    }

    #[test]
    fn rejects_date_wrong_length() {
        assert!(
            AkShareQuery::new("000001", AkShareMarket::AShares, "240101", "20240131").is_err(),
            "6-char date must be rejected"
        );
    }

    // -- error messages ------------------------------------------------------

    #[test]
    fn invalid_symbol_error_message_includes_expected_length() {
        let err = AkShareQuery::new("007", AkShareMarket::HongKong, "20240101", "20240131")
            .expect_err("short symbol must error");
        assert!(
            err.to_string().contains('5'),
            "error should mention expected length 5, got: {err}"
        );
    }

    #[test]
    fn invalid_date_error_message_is_descriptive() {
        let err = AkShareQuery::new("000001", AkShareMarket::AShares, "jan2024", "20240131")
            .expect_err("non-numeric date must error");
        assert!(
            err.to_string().contains("YYYYMMDD"),
            "error should mention YYYYMMDD format, got: {err}"
        );
    }

    // -- stub fetcher --------------------------------------------------------

    #[test]
    fn stub_fetcher_returns_two_bars_for_valid_query() {
        let query = AkShareQuery::new("000001", AkShareMarket::AShares, "20240101", "20240131")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        let bars = fetch_stub(&query).unwrap_or_else(|e| panic!("stub must succeed: {e}"));
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].open, 10.5);
        assert_eq!(bars[0].close, 10.8);
        assert!(bars[0].volume > 0.0);
    }

    // -- market helpers -------------------------------------------------------

    #[test]
    fn market_enum_returns_correct_endpoint_and_venue() {
        assert_eq!(
            AkShareMarket::AShares.endpoint_path(),
            "/api/public/stock_zh_a_hist"
        );
        assert_eq!(
            AkShareMarket::HongKong.endpoint_path(),
            "/api/public/stock_hk_hist"
        );
        assert_eq!(AkShareMarket::AShares.venue(), "akshare_a");
        assert_eq!(AkShareMarket::HongKong.venue(), "akshare_hk");
    }
}
