#![cfg(feature = "http")]
//! Real Seeking Alpha HTTP fetchers for `/analysis/v2/list` and
//! `/symbols/v1/summary`, implemented against the canonical
//! [`tdw_core::Fetcher`] trait.
//!
//! Gated by the `http` feature. Requires `TDW_SEEKING_ALPHA_API_KEY` to be
//! set in the environment. Live integration tests are additionally gated by
//! `TDW_SEEKING_ALPHA_LIVE=1` so unattended CI stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;

use crate::{
    BASE_URL, RAPIDAPI_HOST, RAPIDAPI_KEY_ENV, SeekingAlphaArticle, SeekingAlphaArticlesQuery,
    SeekingAlphaRatings, SeekingAlphaRatingsQuery,
};

const USER_AGENT: &str = "tdw-provider-seeking-alpha/0.1";

// ---------------------------------------------------------------------------
// Wire-format deserialization structs — articles
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WireArticleAttributes {
    title: String,
    #[serde(rename = "publishOn", default)]
    publish_on: String,
    #[serde(rename = "isLockedPro", default)]
    is_locked_pro: bool,
    #[serde(rename = "commentCount", default)]
    comment_count: u32,
}

#[derive(Deserialize)]
struct WireArticle {
    id: String,
    attributes: WireArticleAttributes,
}

#[derive(Deserialize)]
struct WireArticlesEnvelope {
    #[serde(default)]
    data: Vec<WireArticle>,
}

// ---------------------------------------------------------------------------
// Wire-format deserialization structs — ratings
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WireRatingsAttributes {
    #[serde(default)]
    quant_rating: f64,
    #[serde(default)]
    authors_rating: f64,
    #[serde(default)]
    sell_side_rating: f64,
    #[serde(default)]
    quant_rating_change: String,
}

#[derive(Deserialize)]
struct WireRatingsEntry {
    id: String,
    attributes: WireRatingsAttributes,
}

#[derive(Deserialize)]
struct WireRatingsEnvelope {
    #[serde(default)]
    data: Vec<WireRatingsEntry>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(RAPIDAPI_KEY_ENV, "seeking-alpha")
}

fn map_article(wire: WireArticle) -> SeekingAlphaArticle {
    SeekingAlphaArticle {
        id: wire.id,
        title: wire.attributes.title,
        publish_on: wire.attributes.publish_on,
        is_locked_pro: wire.attributes.is_locked_pro,
        comment_count: wire.attributes.comment_count,
    }
}

fn map_ratings(wire: WireRatingsEntry) -> SeekingAlphaRatings {
    SeekingAlphaRatings {
        ticker: wire.id,
        quant_rating: wire.attributes.quant_rating,
        authors_rating: wire.attributes.authors_rating,
        sell_side_rating: wire.attributes.sell_side_rating,
        quant_rating_change: wire.attributes.quant_rating_change,
    }
}

// ---------------------------------------------------------------------------
// Articles HTTP fetcher
// ---------------------------------------------------------------------------

/// Fetches analyst articles from the Seeking Alpha `/analysis/v2/list`
/// endpoint via RapidAPI.
pub struct SeekingAlphaArticlesHttpFetcher {
    base_url: String,
}

impl Default for SeekingAlphaArticlesHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl SeekingAlphaArticlesHttpFetcher {
    /// Override the base URL (useful for testing against a mock server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `seeking-alpha`
    /// provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<SeekingAlphaArticlesQuery, SeekingAlphaArticle> for SeekingAlphaArticlesHttpFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "articles";

    fn transform_query(params: Value) -> Result<SeekingAlphaArticlesQuery> {
        let ticker = params
            .get("ticker")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("seeking-alpha ticker must be a string".to_string())
            })?;
        let size = params.get("size").and_then(Value::as_u64).ok_or_else(|| {
            Error::InvalidQuery("seeking-alpha size must be an integer".to_string())
        })?;
        let size = u32::try_from(size)
            .map_err(|_| Error::InvalidQuery("seeking-alpha size out of range".to_string()))?;
        SeekingAlphaArticlesQuery::new(ticker, size)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &SeekingAlphaArticlesQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let client =
            tdw_core::http_support::build_client(USER_AGENT, "seeking-alpha client build")?;
        let endpoint = format!("{}/analysis/v2/list", self.base_url.trim_end_matches('/'));

        let response = client
            .get(&endpoint)
            .header("x-rapidapi-key", api_key)
            .header("x-rapidapi-host", RAPIDAPI_HOST)
            .query(&[
                ("id", query.ticker.as_str()),
                ("size", &query.size.to_string()),
                ("number", "1"),
            ])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("seeking-alpha articles request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "seeking-alpha articles returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("seeking-alpha articles read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &SeekingAlphaArticlesQuery,
        raw: Bytes,
    ) -> Result<Vec<SeekingAlphaArticle>> {
        let envelope: WireArticlesEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("seeking-alpha articles parse: {e}")))?;
        Ok(envelope.data.into_iter().map(map_article).collect())
    }
}

// ---------------------------------------------------------------------------
// Ratings HTTP fetcher
// ---------------------------------------------------------------------------

/// Fetches stock ratings from the Seeking Alpha `/symbols/v1/summary`
/// endpoint via RapidAPI.
pub struct SeekingAlphaRatingsHttpFetcher {
    base_url: String,
}

impl Default for SeekingAlphaRatingsHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl SeekingAlphaRatingsHttpFetcher {
    /// Override the base URL (useful for testing against a mock server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `seeking-alpha`
    /// provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<SeekingAlphaRatingsQuery, SeekingAlphaRatings> for SeekingAlphaRatingsHttpFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "ratings";

    fn transform_query(params: Value) -> Result<SeekingAlphaRatingsQuery> {
        let ticker = params
            .get("ticker")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("seeking-alpha ticker must be a string".to_string())
            })?;
        SeekingAlphaRatingsQuery::new(ticker)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &SeekingAlphaRatingsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let client =
            tdw_core::http_support::build_client(USER_AGENT, "seeking-alpha client build")?;
        let endpoint = format!("{}/symbols/v1/summary", self.base_url.trim_end_matches('/'));

        let response = client
            .get(&endpoint)
            .header("x-rapidapi-key", api_key)
            .header("x-rapidapi-host", RAPIDAPI_HOST)
            .query(&[("symbols", query.ticker.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("seeking-alpha ratings request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "seeking-alpha ratings returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("seeking-alpha ratings read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &SeekingAlphaRatingsQuery,
        raw: Bytes,
    ) -> Result<Vec<SeekingAlphaRatings>> {
        let envelope: WireRatingsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("seeking-alpha ratings parse: {e}")))?;

        let entry = envelope.data.into_iter().next().ok_or_else(|| {
            Error::Provider(format!(
                "seeking-alpha ratings response missing entry for {}",
                query.ticker
            ))
        })?;

        Ok(vec![map_ratings(entry)])
    }
}
