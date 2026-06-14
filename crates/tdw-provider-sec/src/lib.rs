#![forbid(unsafe_code)]
//! Offline types and validation for the SEC EDGAR data provider.
//!
//! The real HTTP fetchers live in [`http_fetcher`] and are only compiled
//! when the `http` feature is enabled. Everything in this module works
//! without any network access.

pub mod catalog;
pub mod sic;

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::{
    SecCikMapHttpFetcher, SecCompanyFactsHttpFetcher, SecEtfHoldingsHttpFetcher,
    SecFailsToDeliverHttpFetcher, SecFilingHeadersHttpFetcher, SecFilingsHttpFetcher,
    SecForm13FHttpFetcher, SecHtmFileHttpFetcher, SecInstitutionsSearchHttpFetcher,
    SecLatestFinancialReportsHttpFetcher, SecManagementDiscussionAnalysisHttpFetcher,
    SecNportDisclosureHttpFetcher, SecRssLitigationHttpFetcher, SecSchemaFilesHttpFetcher,
    SecSicSearchHttpFetcher, SecSymbolMapHttpFetcher, SecXbrlHttpFetcher,
};

pub use catalog::{ENDPOINTS, SecEndpoint, SecModel};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public base URL for the SEC EDGAR data API.
pub const BASE_URL: &str = "https://data.sec.gov";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "sec";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, SecProviderError>;

// ── Query types ──────────────────────────────────────────────────────────────

/// Query for equity historical data keyed by ticker symbol.
///
/// The symbol is upper-cased and validated (ASCII alphanumeric plus `.`, `-`,
/// `_`) before use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecHistoricalQuery {
    pub symbol: String,
}

impl SecHistoricalQuery {
    /// Validate and normalise `symbol`.
    ///
    /// # Errors
    ///
    /// Returns [`SecProviderError::EmptySymbol`] for blank input or
    /// [`SecProviderError::InvalidSymbol`] for symbols containing unsupported
    /// characters.
    pub fn new(symbol: &str) -> Result<Self> {
        Ok(Self {
            symbol: normalize_symbol(symbol)?,
        })
    }
}

/// Query for SEC EDGAR filings (submissions) for a given CIK.
///
/// The CIK is validated to be a non-empty ASCII digit string and is padded to
/// 10 digits when building the request path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecFilingsQuery {
    /// Central Index Key (CIK) as reported by SEC. Leading zeros are optional.
    pub cik: String,
}

impl SecFilingsQuery {
    /// Validate and normalise `cik`.
    ///
    /// # Errors
    ///
    /// Returns [`SecProviderError::EmptyCik`] for blank input or
    /// [`SecProviderError::InvalidCik`] if the value contains non-digit
    /// characters.
    pub fn new(cik: &str) -> Result<Self> {
        Ok(Self {
            cik: normalize_cik(cik)?,
        })
    }

    /// Return the CIK zero-padded to 10 digits, as required by the EDGAR API.
    #[must_use]
    pub fn padded_cik(&self) -> String {
        format!("{:0>10}", self.cik)
    }
}

/// Parameter-free query for the SEC `company_tickers.json` map.
///
/// `regulators/sec/cik_map` fetches the full ticker↔CIK directory, so the query
/// carries no fields. It exists as a distinct type so the fetcher satisfies the
/// `Fetcher<Q, D>` contract with a `JsonSchema`-bearing query.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecCikMapQuery {}

impl SecCikMapQuery {
    /// Build the parameter-free query (always succeeds).
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }
}

/// A free-text search query used by the keyless regulator-utility routes
/// (`regulators/sec/institutions_search`, `regulators/sec/sic_search`).
///
/// The query is trimmed but otherwise preserved verbatim (case-insensitive
/// matching happens in the fetcher). An empty query is permitted and lists all
/// rows, mirroring the directory-listing convention of the other keyless routes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecSearchQuery {
    /// Free-text needle (matched case-insensitively against names/descriptions).
    pub query: String,
}

impl SecSearchQuery {
    /// Build a search query, trimming surrounding whitespace.
    #[must_use]
    pub fn new(query: &str) -> Self {
        Self {
            query: query.trim().to_string(),
        }
    }
}

/// A query identifying a single EDGAR filing by CIK and accession number, used
/// by `regulators/sec/filing_headers` and `regulators/sec/schema_files`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecAccessionQuery {
    /// Central Index Key (CIK), validated as digits and padded when needed.
    pub cik: String,
    /// Accession number, e.g. `"0000320193-24-000123"` (dashes optional).
    pub accession: String,
}

impl SecAccessionQuery {
    /// Validate and normalise the CIK and accession number.
    ///
    /// # Errors
    ///
    /// Returns [`SecProviderError::EmptyCik`] / [`SecProviderError::InvalidCik`]
    /// for a bad CIK, or [`SecProviderError::InvalidAccession`] when the
    /// accession is blank or contains characters outside `[0-9-]`.
    pub fn new(cik: &str, accession: &str) -> Result<Self> {
        let cik = normalize_cik(cik)?;
        let accession = normalize_accession(accession)?;
        Ok(Self { cik, accession })
    }

    /// Return the CIK zero-padded to 10 digits.
    #[must_use]
    pub fn padded_cik(&self) -> String {
        format!("{:0>10}", self.cik)
    }

    /// Return the accession number with dashes stripped (EDGAR archive form).
    #[must_use]
    pub fn accession_nodash(&self) -> String {
        self.accession.replace('-', "")
    }
}

/// A query identifying a single SEC filing document by its EDGAR archive URL,
/// used by `regulators/sec/htm_file`.
///
/// The URL is validated to be an `https://` URL on a `sec.gov` host so the
/// fetcher cannot be pointed at an arbitrary external host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecHtmlUrlQuery {
    /// Absolute `https://*.sec.gov/...` URL of the filing document to retrieve.
    pub url: String,
}

impl SecHtmlUrlQuery {
    /// Validate and normalise `url`.
    ///
    /// # Errors
    ///
    /// Returns [`SecProviderError::InvalidUrl`] when the URL is blank, is not an
    /// `https://` URL, or is not hosted on a `sec.gov` domain.
    pub fn new(url: &str) -> Result<Self> {
        let url = url.trim();
        if url.is_empty() {
            return Err(SecProviderError::InvalidUrl);
        }
        let Some(rest) = url.strip_prefix("https://") else {
            return Err(SecProviderError::InvalidUrl);
        };
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        if host != "sec.gov" && !host.ends_with(".sec.gov") {
            return Err(SecProviderError::InvalidUrl);
        }
        Ok(Self {
            url: url.to_string(),
        })
    }
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecProviderError {
    #[error("sec symbol must not be empty")]
    EmptySymbol,
    #[error("sec symbol contains unsupported characters")]
    InvalidSymbol,
    #[error("sec CIK must not be empty")]
    EmptyCik,
    #[error("sec CIK must contain only digits")]
    InvalidCik,
    #[error("sec accession number must contain only digits and dashes")]
    InvalidAccession,
    #[error("sec url must be an https sec.gov URL")]
    InvalidUrl,
    #[error("sec provider error: {0}")]
    Provider(String),
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn normalize_symbol(symbol: &str) -> Result<String> {
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return Err(SecProviderError::EmptySymbol);
    }
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(SecProviderError::InvalidSymbol);
    }
    Ok(symbol.to_ascii_uppercase())
}

fn normalize_cik(cik: &str) -> Result<String> {
    let cik = cik.trim();
    if cik.is_empty() {
        return Err(SecProviderError::EmptyCik);
    }
    if !cik.chars().all(|c| c.is_ascii_digit()) {
        return Err(SecProviderError::InvalidCik);
    }
    Ok(cik.to_string())
}

fn normalize_accession(accession: &str) -> Result<String> {
    let accession = accession.trim();
    if accession.is_empty() {
        return Err(SecProviderError::InvalidAccession);
    }
    if !accession.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(SecProviderError::InvalidAccession);
    }
    Ok(accession.to_string())
}

// ── Unit tests (no network, no feature gate needed) ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_query_normalises_symbol() {
        let q = SecHistoricalQuery::new("aapl").expect("should build");
        assert_eq!(q.symbol, "AAPL");
    }

    #[test]
    fn historical_query_rejects_empty_symbol() {
        assert_eq!(
            SecHistoricalQuery::new(""),
            Err(SecProviderError::EmptySymbol)
        );
        assert_eq!(
            SecHistoricalQuery::new("   "),
            Err(SecProviderError::EmptySymbol)
        );
    }

    #[test]
    fn historical_query_rejects_invalid_symbol() {
        assert_eq!(
            SecHistoricalQuery::new("AAPL/secret"),
            Err(SecProviderError::InvalidSymbol),
        );
        assert_eq!(
            SecHistoricalQuery::new("AAPL?foo=1"),
            Err(SecProviderError::InvalidSymbol),
        );
    }

    #[test]
    fn filings_query_accepts_valid_cik() {
        let q = SecFilingsQuery::new("320193").expect("should build");
        assert_eq!(q.cik, "320193");
        assert_eq!(q.padded_cik(), "0000320193");
    }

    #[test]
    fn filings_query_rejects_empty_cik() {
        assert_eq!(SecFilingsQuery::new(""), Err(SecProviderError::EmptyCik));
        assert_eq!(SecFilingsQuery::new("   "), Err(SecProviderError::EmptyCik));
    }

    #[test]
    fn filings_query_rejects_non_digit_cik() {
        assert_eq!(
            SecFilingsQuery::new("abc"),
            Err(SecProviderError::InvalidCik),
        );
        assert_eq!(
            SecFilingsQuery::new("320193abc"),
            Err(SecProviderError::InvalidCik),
        );
    }

    #[test]
    fn padded_cik_always_ten_digits() {
        let q = SecFilingsQuery::new("1").expect("should build");
        assert_eq!(q.padded_cik(), "0000000001");

        let q = SecFilingsQuery::new("0000320193").expect("should build");
        assert_eq!(q.padded_cik(), "0000320193");
    }

    #[test]
    fn html_url_query_accepts_sec_gov_https_only() {
        let q = SecHtmlUrlQuery::new(
            "https://www.sec.gov/Archives/edgar/data/320193/000032019324000123/aapl.htm",
        )
        .expect("sec.gov https url is valid");
        assert!(q.url.contains("aapl.htm"));
        assert!(SecHtmlUrlQuery::new("https://sec.gov/files/x.htm").is_ok());
        // Rejected: non-https, non-sec.gov host, lookalike host, blank.
        assert_eq!(
            SecHtmlUrlQuery::new("http://www.sec.gov/x.htm"),
            Err(SecProviderError::InvalidUrl)
        );
        assert_eq!(
            SecHtmlUrlQuery::new("https://evil.com/x.htm"),
            Err(SecProviderError::InvalidUrl)
        );
        assert_eq!(
            SecHtmlUrlQuery::new("https://sec.gov.evil.com/x.htm"),
            Err(SecProviderError::InvalidUrl)
        );
        assert_eq!(
            SecHtmlUrlQuery::new("  "),
            Err(SecProviderError::InvalidUrl)
        );
    }

    #[test]
    fn error_display_is_human_readable() {
        assert!(SecProviderError::EmptySymbol.to_string().contains("symbol"));
        assert!(SecProviderError::InvalidCik.to_string().contains("CIK"));
        assert!(
            SecProviderError::Provider("network failure".to_string())
                .to_string()
                .contains("network failure")
        );
    }
}
