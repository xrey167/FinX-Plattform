#![forbid(unsafe_code)]
//! TMX Money provider for tdw-core.
//!
//! Exposes two query types for the TMX Money public REST API (no auth
//! required):
//!
//! * [`TmxQuoteQuery`] — single-symbol equity quote (endpoint
//!   `equity_quote`).
//! * [`TmxBatchQuoteQuery`] — up to 10 symbols in one request
//!   (endpoint `equity_batch_quote`).
//!
//! The default (no-feature) build includes query structs, validation,
//! the error enum, a mock fetcher suitable for unit tests, and unit
//! tests. Enable the `http` feature for the real HTTP backend.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{TmxHttpBatchQuoteFetcher, TmxHttpIndexSectorsFetcher, TmxHttpQuoteFetcher};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

// ── constants ──────────────────────────────────────────────────────────────

/// Maximum number of symbols accepted by the batch endpoint.
pub const BATCH_MAX_SYMBOLS: usize = 10;

/// Regex-style description of a valid TMX ticker character set.
/// Used only for documentation; validation is done inline.
const SYMBOL_ALLOWED_CHARS: &str = "A-Z 0-9 .";

// ── error ──────────────────────────────────────────────────────────────────

/// Errors produced by query validation or response parsing.
#[derive(Debug, Error)]
pub enum TmxError {
    /// The ticker symbol does not meet TMX naming rules.
    #[error("invalid symbol {symbol:?}: {reason} (allowed: {SYMBOL_ALLOWED_CHARS})")]
    InvalidSymbol {
        symbol: String,
        reason: &'static str,
    },

    /// The batch request exceeds the allowed symbol cap.
    #[error("batch query contains {count} symbols, max is {BATCH_MAX_SYMBOLS}")]
    TooManySymbols { count: usize },

    /// A required field was absent from the caller-supplied JSON.
    #[error("missing required field: {field}")]
    MissingField { field: &'static str },

    /// The JSON could not be deserialised into the expected shape.
    #[error("json parse error: {0}")]
    ParseJson(String),

    /// A lower-level provider error forwarded from reqwest / tdw-core.
    #[error("provider error: {0}")]
    Provider(String),
}

// ── symbol validation ──────────────────────────────────────────────────────

/// Validate and normalise a single TMX ticker symbol.
///
/// Rules:
/// * 1–10 characters (exclusive).
/// * Only ASCII uppercase letters (`A-Z`), decimal digits (`0-9`),
///   and dots (`.`).
///
/// # Errors
///
/// Returns [`TmxError::InvalidSymbol`] when any rule is violated.
pub fn validate_symbol(raw: &str) -> Result<String, TmxError> {
    let upper = raw.to_uppercase();
    if upper.is_empty() {
        return Err(TmxError::InvalidSymbol {
            symbol: raw.to_string(),
            reason: "symbol must not be empty",
        });
    }
    if upper.len() > 10 {
        return Err(TmxError::InvalidSymbol {
            symbol: raw.to_string(),
            reason: "symbol must be at most 10 characters",
        });
    }
    for ch in upper.chars() {
        if !ch.is_ascii_uppercase() && !ch.is_ascii_digit() && ch != '.' {
            return Err(TmxError::InvalidSymbol {
                symbol: raw.to_string(),
                reason: "symbol contains an invalid character",
            });
        }
    }
    Ok(upper)
}

// ── query types ────────────────────────────────────────────────────────────

/// Query parameters for a single TMX equity quote.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TmxQuoteQuery {
    /// TSX/TSX-V ticker symbol (e.g. `"TD"`, `"RY"`).
    pub symbol: String,
}

impl TmxQuoteQuery {
    /// Validate raw caller params and construct a [`TmxQuoteQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`TmxError`] when the `symbol` field is absent or
    /// fails validation.
    pub fn from_params(params: &Value) -> Result<Self, TmxError> {
        let raw = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or(TmxError::MissingField { field: "symbol" })?;
        let symbol = validate_symbol(raw)?;
        Ok(Self { symbol })
    }
}

/// Validate and normalise a TMX index symbol (e.g. `"^TSX"`, `"^TXCX"`).
///
/// Index symbols allow a leading `^` in addition to the equity character set.
/// A missing leading `^` is added so the query always carries the TMX index
/// form.
///
/// # Errors
///
/// Returns [`TmxError::InvalidSymbol`] when the symbol is empty, too long, or
/// contains a character outside `^ A-Z 0-9 .`.
pub fn validate_index_symbol(raw: &str) -> Result<String, TmxError> {
    let upper = raw.trim().to_uppercase();
    if upper.is_empty() {
        return Err(TmxError::InvalidSymbol {
            symbol: raw.to_string(),
            reason: "index symbol must not be empty",
        });
    }
    if upper.len() > 12 {
        return Err(TmxError::InvalidSymbol {
            symbol: raw.to_string(),
            reason: "index symbol must be at most 12 characters",
        });
    }
    for ch in upper.chars() {
        if !ch.is_ascii_uppercase() && !ch.is_ascii_digit() && ch != '.' && ch != '^' {
            return Err(TmxError::InvalidSymbol {
                symbol: raw.to_string(),
                reason: "index symbol contains an invalid character",
            });
        }
    }
    Ok(if upper.starts_with('^') {
        upper
    } else {
        format!("^{upper}")
    })
}

/// Query parameters for a TMX index sector breakdown (`index/sectors`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TmxIndexSectorsQuery {
    /// TMX index symbol (e.g. `"^TSX"`), normalised to carry a leading `^`.
    pub symbol: String,
}

impl TmxIndexSectorsQuery {
    /// Validate raw caller params and construct a [`TmxIndexSectorsQuery`].
    ///
    /// Accepts either `symbol` or `index`; defaults to the S&P/TSX Composite
    /// (`^TSX`) when neither is supplied.
    ///
    /// # Errors
    ///
    /// Returns [`TmxError`] when the supplied symbol fails index-symbol
    /// validation.
    pub fn from_params(params: &Value) -> Result<Self, TmxError> {
        let raw = params
            .get("symbol")
            .or_else(|| params.get("index"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("^TSX");
        let symbol = validate_index_symbol(raw)?;
        Ok(Self { symbol })
    }
}

/// Query parameters for a batch TMX equity quote (up to 10 symbols).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TmxBatchQuoteQuery {
    /// List of TSX/TSX-V ticker symbols.
    ///
    /// At most [`BATCH_MAX_SYMBOLS`] entries are allowed.
    pub symbols: Vec<String>,
}

impl TmxBatchQuoteQuery {
    /// Validate raw caller params and construct a [`TmxBatchQuoteQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`TmxError`] when the `symbols` field is absent, any
    /// symbol fails validation, or more than [`BATCH_MAX_SYMBOLS`] are
    /// supplied.
    pub fn from_params(params: &Value) -> Result<Self, TmxError> {
        let arr = params
            .get("symbols")
            .and_then(Value::as_array)
            .ok_or(TmxError::MissingField { field: "symbols" })?;

        if arr.len() > BATCH_MAX_SYMBOLS {
            return Err(TmxError::TooManySymbols { count: arr.len() });
        }

        let symbols = arr
            .iter()
            .map(|v| {
                let raw = v
                    .as_str()
                    .ok_or(TmxError::MissingField { field: "symbols[]" })?;
                validate_symbol(raw)
            })
            .collect::<Result<Vec<_>, TmxError>>()?;

        Ok(Self { symbols })
    }
}

// ── data model ─────────────────────────────────────────────────────────────

/// A single equity quote returned by the TMX Money API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TmxQuote {
    /// Ticker symbol as returned by TMX.
    pub symbol: String,
    /// Exchange identifier (e.g. `"TSX"`).
    pub exchange: String,
    /// Last traded price.
    pub last_price: f64,
    /// Absolute change vs. previous close.
    pub change: f64,
    /// Percentage change vs. previous close.
    pub change_percent: f64,
    /// Day trading volume.
    pub volume: u64,
    /// Market capitalisation in CAD.
    pub market_cap: Option<f64>,
    /// Opening price for the current session.
    pub open: f64,
    /// Intraday high.
    pub high: f64,
    /// Intraday low.
    pub low: f64,
    /// Previous (or current) close.
    pub close: f64,
}

// ── mock fetcher (no `http` feature required) ──────────────────────────────

/// A deterministic in-process mock of the TMX single-quote fetcher.
///
/// Always returns a single hardcoded [`TmxQuote`] for the requested
/// symbol. Useful for unit tests that must not touch the network.
#[cfg_attr(not(feature = "http"), derive(Clone, Debug, Default))]
#[cfg_attr(feature = "http", derive(Clone, Debug, Default))]
pub struct TmxMockQuoteFetcher;

impl TmxMockQuoteFetcher {
    /// Perform a synchronous (no-async) round-trip through the mock
    /// fetcher: validate the params, produce mock bytes, decode them.
    ///
    /// # Errors
    ///
    /// Returns [`TmxError`] if validation or JSON encoding fails.
    pub fn fetch_mock(params: &Value) -> Result<Vec<TmxQuote>, TmxError> {
        let query = TmxQuoteQuery::from_params(params)?;
        let quote = TmxQuote {
            symbol: query.symbol,
            exchange: "TSX".to_string(),
            last_price: 79.50,
            change: 0.25,
            change_percent: 0.32,
            volume: 3_500_000,
            market_cap: Some(145_200_000_000.0),
            open: 79.25,
            high: 79.85,
            low: 79.10,
            close: 79.50,
        };
        Ok(vec![quote])
    }
}

// ── JSON helpers ───────────────────────────────────────────────────────────

/// Deserialise a TMX `getquote` JSON response body into a list of
/// [`TmxQuote`]s.
///
/// The envelope shape is:
/// ```json
/// { "results": [ { "symbol": "TD", ... } ] }
/// ```
///
/// # Errors
///
/// Returns [`TmxError::ParseJson`] when the payload cannot be parsed.
pub fn parse_quote_response(raw: &[u8]) -> Result<Vec<TmxQuote>, TmxError> {
    #[derive(Deserialize)]
    struct Envelope {
        results: Vec<RawQuote>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawQuote {
        symbol: String,
        exchange: String,
        last_price: f64,
        change: f64,
        change_percent: f64,
        volume: u64,
        market_cap: Option<f64>,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
    }

    let envelope: Envelope =
        serde_json::from_slice(raw).map_err(|error| TmxError::ParseJson(error.to_string()))?;

    Ok(envelope
        .results
        .into_iter()
        .map(|r| TmxQuote {
            symbol: r.symbol,
            exchange: r.exchange,
            last_price: r.last_price,
            change: r.change,
            change_percent: r.change_percent,
            volume: r.volume,
            market_cap: r.market_cap,
            open: r.open,
            high: r.high,
            low: r.low,
            close: r.close,
        })
        .collect())
}

// ── unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── symbol validation ─────────────────────────────────────────────────

    #[test]
    fn index_sectors_query_normalises_and_defaults() {
        // Caret added when missing, default applied when absent.
        let q = TmxIndexSectorsQuery::from_params(&serde_json::json!({ "symbol": "tsx" }))
            .expect("valid");
        assert_eq!(q.symbol, "^TSX");
        let defaulted =
            TmxIndexSectorsQuery::from_params(&serde_json::json!({})).expect("default applies");
        assert_eq!(defaulted.symbol, "^TSX");
        let kept = TmxIndexSectorsQuery::from_params(&serde_json::json!({ "index": "^TXCX" }))
            .expect("valid");
        assert_eq!(kept.symbol, "^TXCX");
        assert!(
            TmxIndexSectorsQuery::from_params(&serde_json::json!({ "symbol": "TSX?x=1" })).is_err()
        );
    }

    #[test]
    fn validate_symbol_accepts_clean_tickers() {
        assert_eq!(validate_symbol("TD").expect("TD is valid"), "TD");
        assert_eq!(validate_symbol("ry").expect("lowercase normalised"), "RY");
        assert_eq!(
            validate_symbol("BNS.PR.A").expect("dot allowed"),
            "BNS.PR.A"
        );
        assert_eq!(validate_symbol("ABC123").expect("digits allowed"), "ABC123");
    }

    #[test]
    fn validate_symbol_rejects_empty() {
        let err = validate_symbol("").expect_err("empty must fail");
        assert!(matches!(err, TmxError::InvalidSymbol { .. }));
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_symbol_rejects_too_long() {
        let err = validate_symbol("ABCDEFGHIJK").expect_err("11 chars must fail");
        assert!(matches!(err, TmxError::InvalidSymbol { .. }));
        assert!(err.to_string().contains("at most 10"));
    }

    #[test]
    fn validate_symbol_rejects_invalid_chars() {
        let err = validate_symbol("TD?").expect_err("? must fail");
        assert!(matches!(err, TmxError::InvalidSymbol { .. }));
        assert!(err.to_string().contains("invalid character"));

        let err2 = validate_symbol("TD BANK").expect_err("space must fail");
        assert!(matches!(err2, TmxError::InvalidSymbol { .. }));
    }

    #[test]
    fn validate_symbol_accepts_boundary_length() {
        // Exactly 10 chars — must succeed.
        assert!(validate_symbol("ABCDEFGHIJ").is_ok());
    }

    // ── TmxQuoteQuery ─────────────────────────────────────────────────────

    #[test]
    fn tmx_quote_query_from_params_happy_path() {
        let q = TmxQuoteQuery::from_params(&serde_json::json!({ "symbol": "td" }))
            .expect("query must parse");
        assert_eq!(q.symbol, "TD");
    }

    #[test]
    fn tmx_quote_query_from_params_missing_symbol() {
        let err = TmxQuoteQuery::from_params(&serde_json::json!({}))
            .expect_err("missing symbol must fail");
        assert!(matches!(err, TmxError::MissingField { field: "symbol" }));
    }

    #[test]
    fn tmx_quote_query_from_params_invalid_symbol() {
        let err = TmxQuoteQuery::from_params(&serde_json::json!({ "symbol": "TD?" }))
            .expect_err("invalid symbol must fail");
        assert!(matches!(err, TmxError::InvalidSymbol { .. }));
    }

    // ── TmxBatchQuoteQuery ────────────────────────────────────────────────

    #[test]
    fn tmx_batch_query_from_params_happy_path() {
        let q =
            TmxBatchQuoteQuery::from_params(&serde_json::json!({ "symbols": ["TD", "ry", "BNS"] }))
                .expect("batch query must parse");
        assert_eq!(q.symbols, vec!["TD", "RY", "BNS"]);
    }

    #[test]
    fn tmx_batch_query_rejects_too_many_symbols() {
        let symbols: Vec<&str> = vec!["A"; 11];
        let err = TmxBatchQuoteQuery::from_params(&serde_json::json!({ "symbols": symbols }))
            .expect_err("11 symbols must fail");
        assert!(matches!(err, TmxError::TooManySymbols { count: 11 }));
        assert!(err.to_string().contains("11"));
    }

    #[test]
    fn tmx_batch_query_rejects_invalid_symbol_in_list() {
        let err =
            TmxBatchQuoteQuery::from_params(&serde_json::json!({ "symbols": ["TD", "BAD!"] }))
                .expect_err("invalid symbol in batch must fail");
        assert!(matches!(err, TmxError::InvalidSymbol { .. }));
    }

    #[test]
    fn tmx_batch_query_missing_symbols_field() {
        let err = TmxBatchQuoteQuery::from_params(&serde_json::json!({}))
            .expect_err("missing symbols must fail");
        assert!(matches!(err, TmxError::MissingField { field: "symbols" }));
    }

    // ── mock fetcher ──────────────────────────────────────────────────────

    #[test]
    #[allow(clippy::float_cmp)] // exact representable constant, not a computed result
    fn mock_fetcher_returns_deterministic_quote() {
        let rows = TmxMockQuoteFetcher::fetch_mock(&serde_json::json!({ "symbol": "TD" }))
            .expect("mock fetch must succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "TD");
        assert_eq!(rows[0].exchange, "TSX");
        assert_eq!(rows[0].last_price, 79.50);
        assert_eq!(rows[0].volume, 3_500_000);
        assert_eq!(rows[0].market_cap, Some(145_200_000_000.0));
    }

    #[test]
    fn mock_fetcher_normalises_symbol_case() {
        let rows = TmxMockQuoteFetcher::fetch_mock(&serde_json::json!({ "symbol": "ry" }))
            .expect("mock fetch must succeed");
        assert_eq!(rows[0].symbol, "RY");
    }

    // ── parse_quote_response ──────────────────────────────────────────────

    #[test]
    #[allow(clippy::float_cmp)] // exact representable constants from literal JSON
    fn parse_quote_response_decodes_envelope() {
        let raw = serde_json::json!({
            "results": [{
                "symbol": "TD",
                "exchange": "TSX",
                "lastPrice": 79.50,
                "change": 0.25,
                "changePercent": 0.32,
                "volume": 3_500_000u64,
                "marketCap": 145_200_000_000.0f64,
                "open": 79.25,
                "high": 79.85,
                "low": 79.10,
                "close": 79.50
            }]
        })
        .to_string();

        let quotes = parse_quote_response(raw.as_bytes()).expect("parse must succeed");

        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].symbol, "TD");
        assert_eq!(quotes[0].last_price, 79.50);
        assert_eq!(quotes[0].volume, 3_500_000);
        assert_eq!(quotes[0].market_cap, Some(145_200_000_000.0));
    }

    #[test]
    fn parse_quote_response_handles_null_market_cap() {
        let raw = serde_json::json!({
            "results": [{
                "symbol": "XIT",
                "exchange": "TSX",
                "lastPrice": 40.0,
                "change": 0.0,
                "changePercent": 0.0,
                "volume": 100_000u64,
                "marketCap": null,
                "open": 40.0,
                "high": 40.5,
                "low": 39.5,
                "close": 40.0
            }]
        })
        .to_string();

        let quotes = parse_quote_response(raw.as_bytes()).expect("parse must succeed");
        assert_eq!(quotes[0].market_cap, None);
    }

    #[test]
    fn parse_quote_response_rejects_malformed_json() {
        let err = parse_quote_response(b"not json").expect_err("bad json must fail");
        assert!(matches!(err, TmxError::ParseJson(_)));
    }

    #[test]
    fn parse_quote_response_decodes_multiple_results() {
        let raw = serde_json::json!({
            "results": [
                {
                    "symbol": "TD", "exchange": "TSX",
                    "lastPrice": 79.50, "change": 0.25, "changePercent": 0.32,
                    "volume": 3_500_000u64, "marketCap": null,
                    "open": 79.25, "high": 79.85, "low": 79.10, "close": 79.50
                },
                {
                    "symbol": "RY", "exchange": "TSX",
                    "lastPrice": 130.0, "change": -0.50, "changePercent": -0.38,
                    "volume": 2_100_000u64, "marketCap": null,
                    "open": 130.5, "high": 131.0, "low": 129.5, "close": 130.0
                }
            ]
        })
        .to_string();

        let quotes = parse_quote_response(raw.as_bytes()).expect("parse must succeed");
        assert_eq!(quotes.len(), 2);
        assert_eq!(quotes[1].symbol, "RY");
    }
}
