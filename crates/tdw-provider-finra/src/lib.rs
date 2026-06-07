#![forbid(unsafe_code)]
//! Offline types and validation for the FINRA Market Data provider.
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled
//! when the `http` feature is enabled. Everything in this module works
//! without any network access.
//!
//! FINRA public REST API requires no authentication.
//! Base URL: `https://api.finra.org/data/group`

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{FinraOtcSummaryHttpFetcher, FinraShortInterestHttpFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public base URL for the FINRA Market Data API.
pub const BASE_URL: &str = "https://api.finra.org/data/group";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "finra";
/// Maximum allowed limit for short-interest queries.
pub const SHORT_INTEREST_MAX_LIMIT: u32 = 1000;

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, FinraProviderError>;

// ── Query types ───────────────────────────────────────────────────────────────

/// Query for FINRA consolidated short-interest data.
///
/// Calls `GET /OTCMarket/block/CONSOLIDATEDSHORTINTERESTdata` with pipe-delimited
/// response. `limit` is clamped to [`SHORT_INTEREST_MAX_LIMIT`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinraShortInterestQuery {
    /// Maximum number of records to return (1–1000).
    pub limit: u32,
    /// Zero-based page offset.
    pub offset: u32,
}

impl FinraShortInterestQuery {
    /// Construct and validate a short-interest query.
    ///
    /// # Errors
    ///
    /// Returns [`FinraProviderError::InvalidLimit`] if `limit` is 0 or
    /// exceeds [`SHORT_INTEREST_MAX_LIMIT`].
    pub const fn new(limit: u32, offset: u32) -> Result<Self> {
        if limit == 0 || limit > SHORT_INTEREST_MAX_LIMIT {
            return Err(FinraProviderError::InvalidLimit {
                got: limit,
                max: SHORT_INTEREST_MAX_LIMIT,
            });
        }
        Ok(Self { limit, offset })
    }
}

/// Query for FINRA weekly OTC market summary.
///
/// Calls `GET /OTCMarket/block/WEEKLYSUMMARYdata` with pipe-delimited response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinraOtcSummaryQuery {
    /// Maximum number of records to return (1–1000).
    pub limit: u32,
}

impl FinraOtcSummaryQuery {
    /// Construct and validate an OTC summary query.
    ///
    /// # Errors
    ///
    /// Returns [`FinraProviderError::InvalidLimit`] if `limit` is 0 or
    /// exceeds [`SHORT_INTEREST_MAX_LIMIT`].
    pub const fn new(limit: u32) -> Result<Self> {
        if limit == 0 || limit > SHORT_INTEREST_MAX_LIMIT {
            return Err(FinraProviderError::InvalidLimit {
                got: limit,
                max: SHORT_INTEREST_MAX_LIMIT,
            });
        }
        Ok(Self { limit })
    }
}

// ── Output record types ───────────────────────────────────────────────────────

/// A single short-interest record parsed from the FINRA pipe-delimited response.
///
/// Sample wire row:
/// `APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinraShortInterestRecord {
    pub issuer_name: String,
    pub symbol: String,
    pub market_class_code: String,
    pub current_short_interest: i64,
    pub previous_short_interest: i64,
    pub percent_change: f64,
    pub avg_daily_share_volume: i64,
    pub days_to_cover: f64,
    pub settlement_date: String,
}

/// A single weekly OTC summary record parsed from the FINRA pipe-delimited response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinraOtcSummaryRecord {
    pub trade_report_date: String,
    pub market_participant_identifier: String,
    pub total_share_quantity: i64,
    pub total_trade_count: i64,
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq)]
pub enum FinraProviderError {
    #[error("finra limit must be between 1 and {max}, got {got}")]
    InvalidLimit { got: u32, max: u32 },
    #[error("finra response row has wrong field count: expected {expected}, got {got}")]
    MalformedRow { expected: usize, got: usize },
    #[error("finra response field '{field}' failed to parse: {reason}")]
    ParseField { field: &'static str, reason: String },
    #[error("finra provider error: {0}")]
    Provider(String),
}

// ── Pipe-delimited parsing helpers ────────────────────────────────────────────

/// Parse a single pipe-delimited short-interest row.
///
/// Expected field order (9 fields):
/// `issuerName|symbol|marketClassCode|currentShortInterest|previousShortInterest|
///  percentChangeinShortInterest|avgDailyShareVolume|daysToCoverShortInterest|settlementDate`
///
/// # Errors
///
/// Returns [`FinraProviderError::MalformedRow`] when the field count is wrong,
/// or [`FinraProviderError::ParseField`] when a numeric field fails to parse.
pub fn parse_short_interest_row(row: &str) -> Result<FinraShortInterestRecord> {
    const EXPECTED: usize = 9;
    let fields: Vec<&str> = row.split('|').collect();
    if fields.len() != EXPECTED {
        return Err(FinraProviderError::MalformedRow {
            expected: EXPECTED,
            got: fields.len(),
        });
    }

    let current_short_interest =
        fields[3]
            .parse::<i64>()
            .map_err(|e| FinraProviderError::ParseField {
                field: "currentShortInterest",
                reason: e.to_string(),
            })?;
    let previous_short_interest =
        fields[4]
            .parse::<i64>()
            .map_err(|e| FinraProviderError::ParseField {
                field: "previousShortInterest",
                reason: e.to_string(),
            })?;
    let percent_change = fields[5]
        .parse::<f64>()
        .map_err(|e| FinraProviderError::ParseField {
            field: "percentChangeinShortInterest",
            reason: e.to_string(),
        })?;
    let avg_daily_share_volume =
        fields[6]
            .parse::<i64>()
            .map_err(|e| FinraProviderError::ParseField {
                field: "avgDailyShareVolume",
                reason: e.to_string(),
            })?;
    let days_to_cover = fields[7]
        .parse::<f64>()
        .map_err(|e| FinraProviderError::ParseField {
            field: "daysToCoverShortInterest",
            reason: e.to_string(),
        })?;

    Ok(FinraShortInterestRecord {
        issuer_name: fields[0].to_string(),
        symbol: fields[1].to_string(),
        market_class_code: fields[2].to_string(),
        current_short_interest,
        previous_short_interest,
        percent_change,
        avg_daily_share_volume,
        days_to_cover,
        settlement_date: fields[8].to_string(),
    })
}

/// Parse a single pipe-delimited OTC summary row.
///
/// Expected field order (4 fields):
/// `tradeReportDate|marketParticipantIdentifier|totalShareQuantity|totalTradeCount`
///
/// # Errors
///
/// Returns [`FinraProviderError::MalformedRow`] or [`FinraProviderError::ParseField`].
pub fn parse_otc_summary_row(row: &str) -> Result<FinraOtcSummaryRecord> {
    const EXPECTED: usize = 4;
    let fields: Vec<&str> = row.split('|').collect();
    if fields.len() != EXPECTED {
        return Err(FinraProviderError::MalformedRow {
            expected: EXPECTED,
            got: fields.len(),
        });
    }

    let total_share_quantity =
        fields[2]
            .parse::<i64>()
            .map_err(|e| FinraProviderError::ParseField {
                field: "totalShareQuantity",
                reason: e.to_string(),
            })?;
    let total_trade_count =
        fields[3]
            .parse::<i64>()
            .map_err(|e| FinraProviderError::ParseField {
                field: "totalTradeCount",
                reason: e.to_string(),
            })?;

    Ok(FinraOtcSummaryRecord {
        trade_report_date: fields[0].to_string(),
        market_participant_identifier: fields[1].to_string(),
        total_share_quantity,
        total_trade_count,
    })
}

/// Parse a full pipe-delimited short-interest response body (multi-line).
///
/// Skips blank lines. Returns the first error encountered if any row fails.
///
/// # Errors
///
/// Propagates [`FinraProviderError`] from [`parse_short_interest_row`].
pub fn parse_short_interest_response(body: &str) -> Result<Vec<FinraShortInterestRecord>> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_short_interest_row)
        .collect()
}

/// Parse a full pipe-delimited OTC summary response body (multi-line).
///
/// Skips blank lines. Returns the first error encountered if any row fails.
///
/// # Errors
///
/// Propagates [`FinraProviderError`] from [`parse_otc_summary_row`].
pub fn parse_otc_summary_response(body: &str) -> Result<Vec<FinraOtcSummaryRecord>> {
    body.lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_otc_summary_row)
        .collect()
}

// ── Mock fetcher (no network, no feature gate) ────────────────────────────────

/// A mock fetcher that returns a fixed short-interest cassette string.
///
/// Intended for unit tests that exercise the full parse path without HTTP.
pub struct MockShortInterestFetcher {
    /// Pre-recorded pipe-delimited response body.
    pub body: String,
}

impl MockShortInterestFetcher {
    /// Construct a mock fetcher from a static cassette string.
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }

    /// Parse the stored body into records.
    ///
    /// # Errors
    ///
    /// Propagates parse errors from [`parse_short_interest_response`].
    pub fn fetch(&self) -> Result<Vec<FinraShortInterestRecord>> {
        parse_short_interest_response(&self.body)
    }
}

/// A mock fetcher that returns a fixed OTC summary cassette string.
pub struct MockOtcSummaryFetcher {
    /// Pre-recorded pipe-delimited response body.
    pub body: String,
}

impl MockOtcSummaryFetcher {
    /// Construct a mock fetcher from a static cassette string.
    pub fn new(body: impl Into<String>) -> Self {
        Self { body: body.into() }
    }

    /// Parse the stored body into records.
    ///
    /// # Errors
    ///
    /// Propagates parse errors from [`parse_otc_summary_response`].
    pub fn fetch(&self) -> Result<Vec<FinraOtcSummaryRecord>> {
        parse_otc_summary_response(&self.body)
    }
}

// ── Unit tests (no network, no feature gate) ──────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── FinraShortInterestQuery ────────────────────────────────────────────────

    #[test]
    fn short_interest_query_accepts_valid_params() {
        let q = FinraShortInterestQuery::new(25, 0).expect("should build");
        assert_eq!(q.limit, 25);
        assert_eq!(q.offset, 0);
    }

    #[test]
    fn short_interest_query_accepts_max_limit() {
        let q = FinraShortInterestQuery::new(1000, 50).expect("should build at max limit");
        assert_eq!(q.limit, 1000);
    }

    #[test]
    fn short_interest_query_rejects_zero_limit() {
        assert_eq!(
            FinraShortInterestQuery::new(0, 0),
            Err(FinraProviderError::InvalidLimit { got: 0, max: 1000 }),
        );
    }

    #[test]
    fn short_interest_query_rejects_over_max_limit() {
        assert_eq!(
            FinraShortInterestQuery::new(1001, 0),
            Err(FinraProviderError::InvalidLimit {
                got: 1001,
                max: 1000
            }),
        );
    }

    // ── FinraOtcSummaryQuery ───────────────────────────────────────────────────

    #[test]
    fn otc_summary_query_accepts_valid_limit() {
        let q = FinraOtcSummaryQuery::new(10).expect("should build");
        assert_eq!(q.limit, 10);
    }

    #[test]
    fn otc_summary_query_rejects_zero_limit() {
        assert_eq!(
            FinraOtcSummaryQuery::new(0),
            Err(FinraProviderError::InvalidLimit { got: 0, max: 1000 }),
        );
    }

    // ── parse_short_interest_row ───────────────────────────────────────────────

    #[test]
    fn parse_short_interest_row_parses_sample_record() {
        let row = "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15";
        let rec = parse_short_interest_row(row).expect("should parse sample row");
        assert_eq!(rec.issuer_name, "APPLE INC");
        assert_eq!(rec.symbol, "AAPL");
        assert_eq!(rec.market_class_code, "G");
        assert_eq!(rec.current_short_interest, 108_234_568);
        assert_eq!(rec.previous_short_interest, 107_982_345);
        assert!((rec.percent_change - 0.23).abs() < f64::EPSILON);
        assert_eq!(rec.avg_daily_share_volume, 56_789_012);
        assert!((rec.days_to_cover - 1.9).abs() < f64::EPSILON);
        assert_eq!(rec.settlement_date, "2024-01-15");
    }

    #[test]
    fn parse_short_interest_row_rejects_wrong_field_count() {
        let row = "APPLE INC|AAPL|G|108234568";
        assert_eq!(
            parse_short_interest_row(row),
            Err(FinraProviderError::MalformedRow {
                expected: 9,
                got: 4
            }),
        );
    }

    #[test]
    fn parse_short_interest_row_rejects_non_numeric_interest() {
        let row = "APPLE INC|AAPL|G|NOT_A_NUMBER|107982345|0.23|56789012|1.9|2024-01-15";
        let err = parse_short_interest_row(row).expect_err("must fail on bad numeric");
        assert!(
            matches!(
                err,
                FinraProviderError::ParseField {
                    field: "currentShortInterest",
                    ..
                }
            ),
            "unexpected error: {err}",
        );
    }

    // ── parse_otc_summary_row ──────────────────────────────────────────────────

    #[test]
    fn parse_otc_summary_row_parses_valid_row() {
        let row = "2024-01-15|MKTX|1234567|890";
        let rec = parse_otc_summary_row(row).expect("should parse");
        assert_eq!(rec.trade_report_date, "2024-01-15");
        assert_eq!(rec.market_participant_identifier, "MKTX");
        assert_eq!(rec.total_share_quantity, 1_234_567);
        assert_eq!(rec.total_trade_count, 890);
    }

    #[test]
    fn parse_otc_summary_row_rejects_wrong_field_count() {
        assert_eq!(
            parse_otc_summary_row("2024-01-15|MKTX|1234567"),
            Err(FinraProviderError::MalformedRow {
                expected: 4,
                got: 3
            }),
        );
    }

    // ── multi-line response parsing ────────────────────────────────────────────

    #[test]
    fn parse_short_interest_response_skips_blank_lines() {
        let body = concat!(
            "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15\n",
            "\n",
            "TESLA INC|TSLA|G|55000000|54000000|1.85|22000000|2.5|2024-01-15\n",
        );
        let recs = parse_short_interest_response(body).expect("should parse two rows");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].symbol, "AAPL");
        assert_eq!(recs[1].symbol, "TSLA");
    }

    #[test]
    fn parse_otc_summary_response_skips_blank_lines() {
        let body = "2024-01-15|MKTX|1234567|890\n\n2024-01-08|GSCO|9876543|1200\n";
        let recs = parse_otc_summary_response(body).expect("should parse two rows");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].market_participant_identifier, "MKTX");
        assert_eq!(recs[1].trade_report_date, "2024-01-08");
    }

    // ── MockShortInterestFetcher / MockOtcSummaryFetcher ─────────────────────

    #[test]
    fn mock_short_interest_fetcher_returns_parsed_records() {
        let body = "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15";
        let fetcher = MockShortInterestFetcher::new(body);
        let recs = fetcher.fetch().expect("mock fetch should succeed");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].symbol, "AAPL");
    }

    #[test]
    fn mock_otc_summary_fetcher_returns_parsed_records() {
        let fetcher = MockOtcSummaryFetcher::new("2024-01-15|MKTX|1234567|890");
        let recs = fetcher.fetch().expect("mock fetch should succeed");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].total_trade_count, 890);
    }

    // ── Error display ─────────────────────────────────────────────────────────

    #[test]
    fn error_display_is_human_readable() {
        let e = FinraProviderError::InvalidLimit { got: 0, max: 1000 };
        assert!(e.to_string().contains("1000"), "display: {e}");

        let e = FinraProviderError::MalformedRow {
            expected: 9,
            got: 4,
        };
        assert!(e.to_string().contains("9"), "display: {e}");

        let e = FinraProviderError::ParseField {
            field: "currentShortInterest",
            reason: "invalid digit".to_string(),
        };
        assert!(
            e.to_string().contains("currentShortInterest"),
            "display: {e}"
        );

        let e = FinraProviderError::Provider("timeout".to_string());
        assert!(e.to_string().contains("timeout"), "display: {e}");
    }

    // ── serde_json round-trip (hard dep — always available) ───────────────────

    #[test]
    fn short_interest_record_roundtrips_via_serde_json() {
        let rec = FinraShortInterestRecord {
            issuer_name: "APPLE INC".to_string(),
            symbol: "AAPL".to_string(),
            market_class_code: "G".to_string(),
            current_short_interest: 108_234_568,
            previous_short_interest: 107_982_345,
            percent_change: 0.23,
            avg_daily_share_volume: 56_789_012,
            days_to_cover: 1.9,
            settlement_date: "2024-01-15".to_string(),
        };
        let json = serde_json::to_string(&rec).expect("serialize must succeed");
        let back: FinraShortInterestRecord =
            serde_json::from_str(&json).expect("deserialize must succeed");
        assert_eq!(rec, back);
    }

    #[test]
    fn otc_summary_record_roundtrips_via_serde_json() {
        let rec = FinraOtcSummaryRecord {
            trade_report_date: "2024-01-15".to_string(),
            market_participant_identifier: "MKTX".to_string(),
            total_share_quantity: 1_234_567,
            total_trade_count: 890,
        };
        let json = serde_json::to_string(&rec).expect("serialize must succeed");
        let back: FinraOtcSummaryRecord =
            serde_json::from_str(&json).expect("deserialize must succeed");
        assert_eq!(rec, back);
    }
}
