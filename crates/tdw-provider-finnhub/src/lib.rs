#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    FinnhubHttpCompanyNewsFetcher, FinnhubHttpProfileFetcher, FinnhubHttpQuoteSnapshotFetcher,
    FinnhubHttpSymbolSearchFetcher,
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "finnhub";
pub const BASE_URL: &str = "https://finnhub.io/api/v1";
pub const API_KEY_ENV: &str = "TDW_FINNHUB_API_KEY";

pub type Result<T> = std::result::Result<T, FinnhubError>;

/// Query for company profile data from the Finnhub `/stock/profile2` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubProfileQuery {
    pub symbol: String,
}

impl FinnhubProfileQuery {
    /// Construct a validated profile query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptySymbol`] if `symbol` is blank, or
    /// [`FinnhubError::InvalidSymbol`] if it contains characters that are not
    /// ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query for a last-price quote snapshot from the Finnhub `/quote` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubQuoteQuery {
    pub symbol: String,
}

impl FinnhubQuoteQuery {
    /// Construct a validated quote query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptySymbol`] if `symbol` is blank, or
    /// [`FinnhubError::InvalidSymbol`] if it contains characters that are not
    /// ASCII alphanumeric, `.`, `-`, or `_`.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query for a symbol search from the Finnhub `/search` endpoint.
///
/// Unlike the symbol-keyed queries, the search term is free text (it may
/// contain spaces) so it is validated as non-blank only, not via
/// [`normalize_symbol`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubSearchQuery {
    pub query: String,
}

impl FinnhubSearchQuery {
    /// Construct a validated symbol-search query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptyQuery`] if `query` is blank after trimming.
    pub fn new(query: &str) -> Result<Self> {
        let query = query.trim();
        if query.is_empty() {
            return Err(FinnhubError::EmptyQuery);
        }
        Ok(Self {
            query: query.to_string(),
        })
    }
}

/// Query for company news from the Finnhub `/company-news` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubCompanyNewsQuery {
    pub symbol: String,
    pub from: String,
    pub to: String,
}

impl FinnhubCompanyNewsQuery {
    /// Construct a validated company-news query.
    ///
    /// # Errors
    ///
    /// Returns [`FinnhubError::EmptySymbol`] or [`FinnhubError::InvalidSymbol`]
    /// if `symbol` is invalid (see [`FinnhubProfileQuery::new`]), or
    /// [`FinnhubError::InvalidDate`] if `from`/`to` are not `YYYY-MM-DD` dates or
    /// `from` is later than `to`.
    pub fn new(symbol: &str, from: &str, to: &str) -> Result<Self> {
        let symbol = normalize_symbol(symbol)?;
        let from = validate_iso_date(from)?;
        let to = validate_iso_date(to)?;
        if from > to {
            return Err(FinnhubError::InvalidDate);
        }
        Ok(Self { symbol, from, to })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FinnhubError {
    #[error("finnhub symbol must not be empty")]
    EmptySymbol,
    #[error("finnhub symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("finnhub search query must not be empty")]
    EmptyQuery,
    #[error("finnhub date must be a YYYY-MM-DD value")]
    InvalidDate,
    #[error("finnhub provider error: {0}")]
    Provider(String),
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(FinnhubError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(FinnhubError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

/// Validate that `date` is a `YYYY-MM-DD` calendar date string.
///
/// This performs a shape check only (length 10, digits in the right places,
/// dashes at positions 4 and 7); it does not verify the date is real. ISO dates
/// in this form compare lexically, which the company-news query relies on.
fn validate_iso_date(date: &str) -> Result<String> {
    let date = date.trim();
    if date.len() != 10 {
        return Err(FinnhubError::InvalidDate);
    }
    let ok = date.as_bytes().iter().enumerate().all(|(i, &b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    });
    if !ok {
        return Err(FinnhubError::InvalidDate);
    }
    Ok(date.to_string())
}

// ---------------------------------------------------------------------------
// Mock fetchers (offline, for unit tests and feature-flag-off builds)
// ---------------------------------------------------------------------------

/// Offline stub for the company-profile endpoint.
pub struct FinnhubMockProfileFetcher;

impl FinnhubMockProfileFetcher {
    /// Return one hardcoded [`tdw_domain::CompanyProfile`]-compatible row for the
    /// queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FinnhubProfileQuery) -> Result<Vec<FinnhubProfileRow>> {
        Ok(vec![FinnhubProfileRow {
            ticker: query.symbol.clone(),
            name: "Apple Inc".to_string(),
            currency: "USD".to_string(),
            exchange: "NASDAQ NMS - GLOBAL MARKET".to_string(),
            logo: "https://static2.finnhub.io/file/publicdatany/finnhubimage/stock_logo/AAPL.png"
                .to_string(),
            market_capitalization: 3_000_000.0,
        }])
    }
}

/// Offline stub for the quote-snapshot endpoint.
pub struct FinnhubMockQuoteSnapshotFetcher;

impl FinnhubMockQuoteSnapshotFetcher {
    /// Return one hardcoded quote row for the queried symbol.
    ///
    /// # Errors
    ///
    /// Never errors in the current implementation; the signature mirrors the
    /// real fetcher so callers can swap implementations without changes.
    pub fn fetch_stub(query: &FinnhubQuoteQuery) -> Result<Vec<FinnhubQuoteRow>> {
        Ok(vec![FinnhubQuoteRow {
            symbol: query.symbol.clone(),
            c: 189.30,
            d: 1.20,
            dp: 0.638,
            pc: 188.10,
            t: 1_717_200_000,
        }])
    }
}

/// Intermediate row shape for a company-profile response, used to build
/// [`tdw_domain::CompanyProfile`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubProfileRow {
    /// Exchange ticker symbol.
    pub ticker: String,
    /// Company name.
    pub name: String,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Exchange name.
    pub exchange: String,
    /// Logo URL.
    pub logo: String,
    /// Market capitalisation in millions.
    pub market_capitalization: f64,
}

/// Intermediate row shape for a quote response, used to build
/// [`tdw_domain::QuoteSnapshot`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FinnhubQuoteRow {
    /// Ticker symbol (supplied by the query, not the response).
    pub symbol: String,
    /// Current price.
    pub c: f64,
    /// Change (absolute).
    pub d: f64,
    /// Change percent.
    pub dp: f64,
    /// Previous close.
    pub pc: f64,
    /// Unix epoch timestamp in seconds.
    pub t: i64,
}

/// A single symbol-search match from the Finnhub `/search` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SymbolMatch {
    /// Exchange ticker symbol (e.g. `AAPL`).
    pub symbol: String,
    /// Display symbol shown to end users.
    pub display_symbol: String,
    /// Human-readable description (e.g. `APPLE INC`).
    pub description: String,
    /// Instrument type (e.g. `Common Stock`); the wire `type` field.
    pub kind: String,
}

/// A single company-news article from the Finnhub `/company-news` endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CompanyNewsItem {
    /// Finnhub article id.
    pub id: i64,
    /// Publication time in Unix milliseconds (wire `datetime` seconds * 1000).
    pub datetime_ms: i64,
    /// Article headline.
    pub headline: String,
    /// Article summary.
    pub summary: String,
    /// News source name.
    pub source: String,
    /// Article URL.
    pub url: String,
    /// Article category.
    pub category: String,
    /// Related symbols (comma-separated as supplied by Finnhub).
    pub related: String,
}

// ---------------------------------------------------------------------------
// Unit tests (no feature gate required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_query_normalises_symbol_to_uppercase() {
        let q =
            FinnhubProfileQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn profile_query_rejects_empty_symbol() {
        assert_eq!(FinnhubProfileQuery::new(""), Err(FinnhubError::EmptySymbol));
        assert_eq!(
            FinnhubProfileQuery::new("   "),
            Err(FinnhubError::EmptySymbol)
        );
    }

    #[test]
    fn profile_query_rejects_invalid_characters() {
        assert_eq!(
            FinnhubProfileQuery::new("AAPL/../../secret"),
            Err(FinnhubError::InvalidSymbol)
        );
        assert_eq!(
            FinnhubProfileQuery::new("AAPL?token=x"),
            Err(FinnhubError::InvalidSymbol)
        );
    }

    #[test]
    fn profile_query_accepts_dots_dashes_underscores() {
        assert!(FinnhubProfileQuery::new("BRK.B").is_ok());
        assert!(FinnhubProfileQuery::new("BRK-B").is_ok());
        assert!(FinnhubProfileQuery::new("SPY_ETF").is_ok());
    }

    #[test]
    fn quote_query_normalises_symbol() {
        let q =
            FinnhubQuoteQuery::new("aapl").unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn quote_query_rejects_empty_symbol() {
        assert_eq!(FinnhubQuoteQuery::new(""), Err(FinnhubError::EmptySymbol));
    }

    #[test]
    fn mock_profile_fetcher_returns_stub_row() {
        let q = FinnhubProfileQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
        let rows =
            FinnhubMockProfileFetcher::fetch_stub(&q).unwrap_or_else(|e| panic!("stub: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ticker, "AAPL");
        assert_eq!(rows[0].currency, "USD");
        assert_eq!(rows[0].market_capitalization, 3_000_000.0);
    }

    #[test]
    fn mock_quote_snapshot_fetcher_returns_stub_row() {
        let q = FinnhubQuoteQuery::new("AAPL").unwrap_or_else(|e| panic!("query: {e}"));
        let rows =
            FinnhubMockQuoteSnapshotFetcher::fetch_stub(&q).unwrap_or_else(|e| panic!("stub: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "AAPL");
        assert_eq!(rows[0].c, 189.30);
        assert_eq!(rows[0].t, 1_717_200_000);
    }

    #[test]
    fn finnhub_error_messages_are_descriptive() {
        assert!(FinnhubError::EmptySymbol.to_string().contains("empty"));
        assert!(
            FinnhubError::InvalidSymbol
                .to_string()
                .contains("unsupported")
        );
        assert!(FinnhubError::EmptyQuery.to_string().contains("empty"));
        assert!(FinnhubError::InvalidDate.to_string().contains("YYYY-MM-DD"));
        assert!(
            FinnhubError::Provider("timeout".to_string())
                .to_string()
                .contains("timeout")
        );
    }

    #[test]
    fn search_query_trims_but_preserves_free_text() {
        let q = FinnhubSearchQuery::new("  apple inc  ")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.query, "apple inc");
    }

    #[test]
    fn search_query_rejects_blank() {
        assert_eq!(FinnhubSearchQuery::new(""), Err(FinnhubError::EmptyQuery));
        assert_eq!(
            FinnhubSearchQuery::new("   "),
            Err(FinnhubError::EmptyQuery)
        );
    }

    #[test]
    fn company_news_query_normalises_symbol_and_keeps_dates() {
        let q = FinnhubCompanyNewsQuery::new("aapl", "2024-01-01", "2024-01-31")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        assert_eq!(q.symbol, "AAPL");
        assert_eq!(q.from, "2024-01-01");
        assert_eq!(q.to, "2024-01-31");
    }

    #[test]
    fn company_news_query_rejects_invalid_symbol() {
        assert_eq!(
            FinnhubCompanyNewsQuery::new("", "2024-01-01", "2024-01-31"),
            Err(FinnhubError::EmptySymbol)
        );
        assert_eq!(
            FinnhubCompanyNewsQuery::new("AAPL/../x", "2024-01-01", "2024-01-31"),
            Err(FinnhubError::InvalidSymbol)
        );
    }

    #[test]
    fn company_news_query_rejects_bad_dates() {
        assert_eq!(
            FinnhubCompanyNewsQuery::new("AAPL", "2024/01/01", "2024-01-31"),
            Err(FinnhubError::InvalidDate)
        );
        assert_eq!(
            FinnhubCompanyNewsQuery::new("AAPL", "2024-1-1", "2024-01-31"),
            Err(FinnhubError::InvalidDate)
        );
        // from later than to is rejected.
        assert_eq!(
            FinnhubCompanyNewsQuery::new("AAPL", "2024-02-01", "2024-01-31"),
            Err(FinnhubError::InvalidDate)
        );
    }
}
