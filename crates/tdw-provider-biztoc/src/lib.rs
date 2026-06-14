#![forbid(unsafe_code)]
//! Offline types and validation for the `BizToc` business/world-news provider
//! (OpenBB-parity item **P4W12**).
//!
//! The real HTTP fetcher lives in [`http_fetcher`] and is only compiled when the
//! `http` feature is enabled. Everything in this module works without any
//! network access. `BizToc` is served through `RapidAPI`: requests carry an
//! `X-RapidAPI-Key` (read from `BIZTOC_API_KEY`, a free `RapidAPI` key) and an
//! `X-RapidAPI-Host` header. The provider normalizes the latest business/world
//! news stream onto the standard `news/world` model.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::BiztocWorldNewsFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provider identifier used in the registry.
pub const PROVIDER_ID: &str = "biztoc";
/// Public base URL for the `BizToc` `RapidAPI` endpoint.
pub const BASE_URL: &str = "https://biztoc.p.rapidapi.com";
/// Environment variable carrying the free `RapidAPI` key.
pub const API_KEY_ENV: &str = "BIZTOC_API_KEY";
/// `RapidAPI` host header value `BizToc` expects.
pub const RAPIDAPI_HOST: &str = "biztoc.p.rapidapi.com";

/// Maximum allowed page size for the latest-news request.
pub const MAX_PAGE_SIZE: u32 = 100;

/// Shared result type for this crate.
pub type Result<T> = std::result::Result<T, BiztocProviderError>;

/// Errors produced by the `BizToc` provider's offline layer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BiztocProviderError {
    /// `page_size` was outside the supported `1..=100` range.
    #[error("biztoc page_size must be between 1 and {MAX_PAGE_SIZE}")]
    InvalidPageSize,
    /// The `BIZTOC_API_KEY` env var was not set when a live call was attempted.
    #[error("biztoc api key env BIZTOC_API_KEY must be set")]
    MissingApiKey,
}

/// Query parameters for the `BizToc` latest business/world-news request.
///
/// Standardizes the `news/world` route's `BizToc` leg: a symbol-free general
/// business/world news stream. `page_size` caps how many articles to return.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BiztocWorldNewsQuery {
    /// Number of articles to return (1–100).
    pub page_size: u32,
}

impl BiztocWorldNewsQuery {
    /// Construct and validate a world-news query.
    ///
    /// # Errors
    ///
    /// Returns [`BiztocProviderError::InvalidPageSize`] when `page_size` is
    /// outside `1..=100`.
    pub const fn new(page_size: u32) -> Result<Self> {
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(BiztocProviderError::InvalidPageSize);
        }
        Ok(Self { page_size })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_news_query_validates_page_size() {
        let query =
            BiztocWorldNewsQuery::new(20).unwrap_or_else(|error| panic!("should build: {error}"));
        assert_eq!(query.page_size, 20);
        assert_eq!(
            BiztocWorldNewsQuery::new(0).expect_err("zero page_size should fail"),
            BiztocProviderError::InvalidPageSize
        );
        assert_eq!(
            BiztocWorldNewsQuery::new(MAX_PAGE_SIZE + 1).expect_err("over-max should fail"),
            BiztocProviderError::InvalidPageSize
        );
        assert!(BiztocWorldNewsQuery::new(MAX_PAGE_SIZE).is_ok());
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert!(
            BiztocProviderError::MissingApiKey
                .to_string()
                .contains("BIZTOC_API_KEY")
        );
        assert!(
            BiztocProviderError::InvalidPageSize
                .to_string()
                .contains("100")
        );
    }
}
