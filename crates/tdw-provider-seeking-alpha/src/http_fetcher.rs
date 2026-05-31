#![cfg(feature = "http")]
//! Real Seeking Alpha HTTP fetchers for `/analysis/v2/list` and
//! `/symbols/v1/summary`.
//!
//! Gated by the `http` feature. Requires `TDW_SEEKING_ALPHA_API_KEY` to be
//! set in the environment. Live integration tests are additionally gated by
//! `TDW_SEEKING_ALPHA_LIVE=1` so unattended CI stays offline.

use reqwest::Client;
use serde::Deserialize;

use crate::{
    BASE_URL, RAPIDAPI_HOST, RAPIDAPI_KEY_ENV, Result, SeekingAlphaArticle,
    SeekingAlphaArticlesQuery, SeekingAlphaProviderError, SeekingAlphaRatings,
    SeekingAlphaRatingsQuery,
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
    std::env::var(RAPIDAPI_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(SeekingAlphaProviderError::MissingApiKey)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| {
            SeekingAlphaProviderError::Provider(format!("seeking-alpha client build: {e}"))
        })
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
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch analyst articles for the given query.
    ///
    /// Reads `TDW_SEEKING_ALPHA_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`SeekingAlphaProviderError`] on missing key, network failure,
    /// or a non-2xx HTTP status.
    pub async fn fetch(
        &self,
        query: &SeekingAlphaArticlesQuery,
    ) -> Result<Vec<SeekingAlphaArticle>> {
        let api_key = read_api_key()?;
        let client = build_client()?;
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
            .map_err(|e| {
                SeekingAlphaProviderError::Provider(format!("seeking-alpha articles request: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SeekingAlphaProviderError::Provider(format!(
                "seeking-alpha articles returned {status}: {body}"
            )));
        }

        let envelope: WireArticlesEnvelope = response.json().await.map_err(|e| {
            SeekingAlphaProviderError::Provider(format!("seeking-alpha articles parse: {e}"))
        })?;

        Ok(envelope.data.into_iter().map(map_article).collect())
    }
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
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch ratings for the given query.
    ///
    /// Reads `TDW_SEEKING_ALPHA_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`SeekingAlphaProviderError`] on missing key, network failure,
    /// or a non-2xx HTTP status.
    pub async fn fetch(&self, query: &SeekingAlphaRatingsQuery) -> Result<SeekingAlphaRatings> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/symbols/v1/summary", self.base_url.trim_end_matches('/'));

        let response = client
            .get(&endpoint)
            .header("x-rapidapi-key", api_key)
            .header("x-rapidapi-host", RAPIDAPI_HOST)
            .query(&[("symbols", query.ticker.as_str())])
            .send()
            .await
            .map_err(|e| {
                SeekingAlphaProviderError::Provider(format!("seeking-alpha ratings request: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(SeekingAlphaProviderError::Provider(format!(
                "seeking-alpha ratings returned {status}: {body}"
            )));
        }

        let envelope: WireRatingsEnvelope = response.json().await.map_err(|e| {
            SeekingAlphaProviderError::Provider(format!("seeking-alpha ratings parse: {e}"))
        })?;

        let entry = envelope.data.into_iter().next().ok_or_else(|| {
            SeekingAlphaProviderError::Provider(format!(
                "seeking-alpha ratings response missing entry for {}",
                query.ticker
            ))
        })?;

        Ok(map_ratings(entry))
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
