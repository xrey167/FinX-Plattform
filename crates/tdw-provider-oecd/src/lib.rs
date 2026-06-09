#![forbid(unsafe_code)]
//! Offline types and validation for the OECD Statistics data provider.
//!
//! The real HTTP fetcher lives in [`http_fetcher`] and is only compiled
//! when the `http` feature is enabled. Everything in this module works
//! without any network access.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::OecdHttpDataFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public base URL for the OECD SDMX-JSON Statistics API.
pub const BASE_URL: &str = "https://stats.oecd.org/SDMX-JSON/data";
/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "oecd";

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, OecdProviderError>;

// ── Query types ──────────────────────────────────────────────────────────────

/// Query parameters for a single OECD dataset slice.
///
/// Maps to:
/// `GET /SDMX-JSON/data/{dataset}/{filter}/OECD?startTime={start_time}&endTime={end_time}`
///
/// `dataset` must be non-empty ASCII alphanumeric or underscore, max 50 chars.
/// `filter`, `start_time`, and `end_time` are passed verbatim (OECD uses
/// dimension-key notation such as `AUS+AUT.B1_GE.CUR+VOBARSA.Q`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OecdQuery {
    /// OECD dataset code, e.g. `"QNA"` or `"MEI"`.
    pub dataset: String,
    /// Dimension filter in SDMX key notation.
    pub filter: String,
    /// Start period / year, e.g. `"2020"` or `"2020-Q1"`.
    pub start_time: String,
    /// End period / year, e.g. `"2023"` or `"2023-Q4"`.
    pub end_time: String,
}

impl OecdQuery {
    /// Validate and construct an [`OecdQuery`].
    ///
    /// # Errors
    ///
    /// Returns [`OecdProviderError::EmptyDataset`] if `dataset` is blank,
    /// [`OecdProviderError::InvalidDataset`] if it contains unsupported
    /// characters or exceeds 50 characters,
    /// [`OecdProviderError::EmptyFilter`] if `filter` is blank, or
    /// [`OecdProviderError::InvalidFilter`] if `filter` contains characters
    /// outside the SDMX key grammar.
    pub fn new(dataset: &str, filter: &str, start_time: &str, end_time: &str) -> Result<Self> {
        let dataset = validate_dataset(dataset)?;
        let filter = validate_filter(filter)?;
        Ok(Self {
            dataset,
            filter,
            start_time: start_time.trim().to_string(),
            end_time: end_time.trim().to_string(),
        })
    }
}

// ── Observation output row ────────────────────────────────────────────────────

/// A single decoded observation returned by the OECD SDMX-JSON API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OecdObservation {
    /// The OECD dataset code this observation belongs to.
    pub dataset: String,
    /// The SDMX observation key (e.g. `"0:0:0:0"`).
    pub key: String,
    /// Time-period label as reported by OECD (e.g. `"2023-Q1"`).
    pub period: String,
    /// Numeric value of the observation.
    pub value: f64,
}

// ── Error enum ───────────────────────────────────────────────────────────────

/// Errors produced by `tdw-provider-oecd`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OecdProviderError {
    #[error("oecd dataset code must not be empty")]
    EmptyDataset,
    #[error("oecd dataset code contains unsupported characters or exceeds 50 chars")]
    InvalidDataset,
    #[error("oecd filter must not be empty")]
    EmptyFilter,
    #[error("oecd filter contains characters outside the SDMX key grammar")]
    InvalidFilter,
}

// ── Provider metadata ─────────────────────────────────────────────────────────

/// Registry-style endpoint descriptor (no external dep required).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderEndpoint {
    pub provider: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
}

/// Returns the static list of endpoints advertised by this provider.
#[must_use]
pub const fn endpoints() -> &'static [ProviderEndpoint] {
    const ENDPOINTS: &[ProviderEndpoint] = &[ProviderEndpoint {
        provider: PROVIDER_ID,
        name: "sdmx_data",
        base_url: BASE_URL,
    }];
    ENDPOINTS
}

/// Build the URL path for an OECD SDMX-JSON data request.
///
/// # Errors
///
/// Returns an error if `dataset` or `filter` fail validation.
pub fn sdmx_data_url(query: &OecdQuery) -> Result<String> {
    // Re-validate so callers cannot smuggle an already-built query with a
    // hand-crafted invalid dataset or filter (both land on the URL path).
    validate_dataset(&query.dataset)?;
    validate_filter(&query.filter)?;
    Ok(format!(
        "{}/{}/{}/OECD?startTime={}&endTime={}&contentType=application/json",
        BASE_URL, query.dataset, query.filter, query.start_time, query.end_time,
    ))
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn validate_dataset(dataset: &str) -> Result<String> {
    let dataset = dataset.trim();
    if dataset.is_empty() {
        return Err(OecdProviderError::EmptyDataset);
    }
    if dataset.len() > 50 {
        return Err(OecdProviderError::InvalidDataset);
    }
    if !dataset
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(OecdProviderError::InvalidDataset);
    }
    Ok(dataset.to_string())
}

/// Validate an SDMX dimension filter such as `AUS+AUT.B1_GE.CUR+VOBARSA.Q`.
///
/// The filter sits on the request path between `{dataset}` and `/OECD`, so it
/// is constrained to the SDMX key grammar — ASCII alphanumerics plus the
/// structural characters OECD uses: `.` (dimension separator), `+` (OR within
/// a dimension), `_`, and `-`. This excludes `/`, `?`, `&` and `#`, closing the
/// same path/query-injection gap `dataset` was already protected against.
fn validate_filter(filter: &str) -> Result<String> {
    let filter = filter.trim();
    if filter.is_empty() {
        return Err(OecdProviderError::EmptyFilter);
    }
    if !filter
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '_' | '-'))
    {
        return Err(OecdProviderError::InvalidFilter);
    }
    Ok(filter.to_string())
}

// ── Mock fetcher (no-http, always-offline) ────────────────────────────────────

/// A deterministic in-memory fetcher for use in tests that do not enable the
/// `http` feature. Returns a hard-coded single-row response for any valid query.
pub struct MockOecdFetcher;

impl MockOecdFetcher {
    /// Simulate fetching: validates `query` then returns a synthetic row.
    ///
    /// # Errors
    ///
    /// Propagates any [`OecdProviderError`] produced by query validation.
    pub fn fetch(&self, query: &OecdQuery) -> Result<Vec<OecdObservation>> {
        validate_dataset(&query.dataset)?;
        Ok(vec![OecdObservation {
            dataset: query.dataset.clone(),
            key: "0:0:0:0".to_string(),
            period: query.start_time.clone(),
            value: 42.0,
        }])
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact-value parse assertions; deterministic parser, exact comparison intended
mod tests {
    use super::*;

    // ── OecdQuery construction ────────────────────────────────────────────

    #[test]
    fn query_accepts_valid_input() {
        let q = OecdQuery::new("QNA", "AUS.B1_GE.Q", "2020", "2023")
            .unwrap_or_else(|e| panic!("valid query must build: {e}"));
        assert_eq!(q.dataset, "QNA");
        assert_eq!(q.filter, "AUS.B1_GE.Q");
        assert_eq!(q.start_time, "2020");
        assert_eq!(q.end_time, "2023");
    }

    #[test]
    fn query_trims_whitespace_from_all_fields() {
        let q = OecdQuery::new("  QNA  ", "  AUS  ", " 2020 ", " 2023 ")
            .unwrap_or_else(|e| panic!("trim should succeed: {e}"));
        assert_eq!(q.dataset, "QNA");
        assert_eq!(q.filter, "AUS");
        assert_eq!(q.start_time, "2020");
        assert_eq!(q.end_time, "2023");
    }

    #[test]
    fn query_rejects_empty_dataset() {
        assert_eq!(
            OecdQuery::new("", "AUS", "2020", "2023").expect_err("empty dataset must fail"),
            OecdProviderError::EmptyDataset,
        );
    }

    #[test]
    fn query_rejects_whitespace_only_dataset() {
        assert_eq!(
            OecdQuery::new("   ", "AUS", "2020", "2023")
                .expect_err("whitespace-only dataset must fail"),
            OecdProviderError::EmptyDataset,
        );
    }

    #[test]
    fn query_rejects_dataset_with_invalid_chars() {
        assert_eq!(
            OecdQuery::new("QNA&drop=1", "AUS", "2020", "2023")
                .expect_err("invalid chars must fail"),
            OecdProviderError::InvalidDataset,
        );
        assert_eq!(
            OecdQuery::new("QNA-MEI", "AUS", "2020", "2023").expect_err("hyphen must fail"),
            OecdProviderError::InvalidDataset,
        );
    }

    #[test]
    fn query_rejects_dataset_exceeding_50_chars() {
        let long = "A".repeat(51);
        assert_eq!(
            OecdQuery::new(&long, "AUS", "2020", "2023").expect_err("51-char dataset must fail"),
            OecdProviderError::InvalidDataset,
        );
    }

    #[test]
    fn query_accepts_dataset_exactly_50_chars() {
        let exactly_50 = "A".repeat(50);
        let q = OecdQuery::new(&exactly_50, "AUS", "2020", "2023")
            .unwrap_or_else(|e| panic!("50-char dataset must be accepted: {e}"));
        assert_eq!(q.dataset.len(), 50);
    }

    #[test]
    fn query_rejects_empty_filter() {
        assert_eq!(
            OecdQuery::new("QNA", "", "2020", "2023").expect_err("empty filter must fail"),
            OecdProviderError::EmptyFilter,
        );
    }

    #[test]
    fn query_rejects_filter_with_injection_characters() {
        // The filter is interpolated into the path between `{dataset}` and
        // `/OECD`; `/`, `?` and `&` would inject path segments / query params.
        for bad in ["AUS/../x", "AUS?evil=1", "AUS&drop=1", "AUS OECD"] {
            assert_eq!(
                OecdQuery::new("QNA", bad, "2020", "2023")
                    .expect_err("filter with injection chars must fail"),
                OecdProviderError::InvalidFilter,
                "filter {bad:?} should be rejected",
            );
        }
        // Legitimate SDMX dimension-key notation still parses.
        assert!(OecdQuery::new("QNA", "AUS+AUT.B1_GE.CUR+VOBARSA.Q", "2020", "2023").is_ok());
    }

    // ── sdmx_data_url ─────────────────────────────────────────────────────

    #[test]
    fn sdmx_data_url_contains_all_query_parts() {
        let q = OecdQuery::new("QNA", "AUS.B1GQ.Q", "2020", "2023")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        let url = sdmx_data_url(&q).unwrap_or_else(|e| panic!("url must build: {e}"));

        assert!(url.contains("stats.oecd.org"), "url={url}");
        assert!(url.contains("/QNA/"), "url={url}");
        assert!(url.contains("AUS.B1GQ.Q"), "url={url}");
        assert!(url.contains("/OECD"), "url={url}");
        assert!(url.contains("startTime=2020"), "url={url}");
        assert!(url.contains("endTime=2023"), "url={url}");
    }

    // ── endpoints() ───────────────────────────────────────────────────────

    #[test]
    fn endpoints_exposes_oecd_provider() {
        let eps = endpoints();
        assert!(!eps.is_empty());
        assert_eq!(eps[0].provider, "oecd");
        assert_eq!(eps[0].name, "sdmx_data");
        assert!(eps[0].base_url.contains("stats.oecd.org"));
    }

    // ── MockOecdFetcher ───────────────────────────────────────────────────

    #[test]
    fn mock_fetcher_returns_synthetic_row() {
        let fetcher = MockOecdFetcher;
        let q = OecdQuery::new("MEI", "AUS.IRLT.A", "2022", "2023")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .fetch(&q)
            .unwrap_or_else(|e| panic!("mock fetch must succeed: {e}"));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].dataset, "MEI");
        assert_eq!(rows[0].period, "2022");
        assert_eq!(rows[0].value, 42.0);
    }

    #[test]
    fn mock_fetcher_propagates_invalid_dataset_error() {
        let fetcher = MockOecdFetcher;
        // Bypass OecdQuery::new by constructing the struct directly so we can
        // test that MockOecdFetcher itself validates the dataset.
        let bad_query = OecdQuery {
            dataset: "BAD DATASET!".to_string(),
            filter: "AUS".to_string(),
            start_time: "2022".to_string(),
            end_time: "2023".to_string(),
        };
        assert_eq!(
            fetcher
                .fetch(&bad_query)
                .expect_err("invalid dataset must propagate"),
            OecdProviderError::InvalidDataset,
        );
    }
}
