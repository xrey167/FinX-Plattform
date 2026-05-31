#![cfg(feature = "http")]
//! Real Adanos HTTP fetchers for `/sentiment/stocks/{ticker}`,
//! `/trending/stocks`, and `/polymarket/events`.
//!
//! Gated by the `http` feature. Requires `TDW_ADANOS_API_KEY` to be set in
//! the environment. Live integration tests are additionally gated by
//! `TDW_ADANOS_LIVE=1` so unattended CI stays offline.

use reqwest::Client;
use serde::Deserialize;

use crate::{
    API_KEY_ENV, AdanosMentions, AdanosPolymarketEvent, AdanosPolymarketQuery,
    AdanosPolymarketResult, AdanosProviderError, AdanosSentimentQuery, AdanosSentimentResult,
    AdanosSentimentSources, AdanosTrendingItem, AdanosTrendingQuery, AdanosTrendingResult,
    BASE_URL, Result,
};

const USER_AGENT: &str = "tdw-provider-adanos/0.1";

// ---------------------------------------------------------------------------
// Wire-format deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WireSentimentSources {
    reddit: f64,
    twitter: f64,
    news: f64,
}

#[derive(Deserialize)]
struct WireMentions {
    reddit: u64,
    twitter: u64,
    news: u64,
}

#[derive(Deserialize)]
struct WireSentimentResult {
    ticker: String,
    timestamp: i64,
    #[serde(rename = "sentimentScore")]
    sentiment_score: f64,
    #[serde(rename = "buzzScore")]
    buzz_score: u32,
    sources: WireSentimentSources,
    mentions: WireMentions,
    trend: String,
}

#[derive(Deserialize)]
struct WireTrendingItem {
    ticker: String,
    #[serde(rename = "buzzScore")]
    buzz_score: u32,
    #[serde(rename = "sentimentScore")]
    sentiment_score: f64,
    mentions: u64,
}

#[derive(Deserialize)]
struct WireTrendingEnvelope {
    timestamp: i64,
    #[serde(default)]
    trending: Vec<WireTrendingItem>,
}

#[derive(Deserialize)]
struct WirePolymarketEvent {
    id: String,
    title: String,
    probability: f64,
    volume: f64,
    #[serde(rename = "expiresAt")]
    expires_at: String,
}

#[derive(Deserialize)]
struct WirePolymarketEnvelope {
    #[serde(default)]
    events: Vec<WirePolymarketEvent>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(AdanosProviderError::MissingApiKey)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| AdanosProviderError::Provider(format!("adanos client build: {e}")))
}

// ---------------------------------------------------------------------------
// Sentiment fetcher
// ---------------------------------------------------------------------------

/// Fetches stock sentiment from the Adanos `/sentiment/stocks/{ticker}`
/// endpoint.
pub struct AdanosSentimentHttpFetcher {
    base_url: String,
}

impl Default for AdanosSentimentHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl AdanosSentimentHttpFetcher {
    /// Override the Adanos base URL (useful for testing against a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch sentiment for the given ticker.
    ///
    /// Reads `TDW_ADANOS_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] on missing key, network failure, or a
    /// non-2xx HTTP status.
    pub async fn fetch(&self, query: &AdanosSentimentQuery) -> Result<AdanosSentimentResult> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!(
            "{}/sentiment/stocks/{}",
            self.base_url.trim_end_matches('/'),
            query.ticker
        );
        let response = client
            .get(&endpoint)
            .header("X-API-Key", &api_key)
            .send()
            .await
            .map_err(|e| AdanosProviderError::Provider(format!("adanos sentiment request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AdanosProviderError::Provider(format!(
                "adanos sentiment returned {status}: {body}"
            )));
        }

        let wire: WireSentimentResult = response
            .json()
            .await
            .map_err(|e| AdanosProviderError::Provider(format!("adanos sentiment parse: {e}")))?;

        Ok(map_sentiment(wire))
    }
}

fn map_sentiment(wire: WireSentimentResult) -> AdanosSentimentResult {
    AdanosSentimentResult {
        ticker: wire.ticker,
        timestamp: wire.timestamp,
        sentiment_score: wire.sentiment_score,
        buzz_score: wire.buzz_score,
        sources: AdanosSentimentSources {
            reddit: wire.sources.reddit,
            twitter: wire.sources.twitter,
            news: wire.sources.news,
        },
        mentions: AdanosMentions {
            reddit: wire.mentions.reddit,
            twitter: wire.mentions.twitter,
            news: wire.mentions.news,
        },
        trend: wire.trend,
    }
}

// ---------------------------------------------------------------------------
// Trending fetcher
// ---------------------------------------------------------------------------

/// Fetches trending stocks from the Adanos `/trending/stocks` endpoint.
pub struct AdanosTrendingHttpFetcher {
    base_url: String,
}

impl Default for AdanosTrendingHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl AdanosTrendingHttpFetcher {
    /// Override the Adanos base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch trending stocks for the given query.
    ///
    /// Reads `TDW_ADANOS_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] on missing key, network failure, or a
    /// non-2xx HTTP status.
    pub async fn fetch(&self, query: &AdanosTrendingQuery) -> Result<AdanosTrendingResult> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/trending/stocks", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("X-API-Key", &api_key)
            .query(&[("limit", query.limit.to_string())])
            .send()
            .await
            .map_err(|e| AdanosProviderError::Provider(format!("adanos trending request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AdanosProviderError::Provider(format!(
                "adanos trending returned {status}: {body}"
            )));
        }

        let wire: WireTrendingEnvelope = response
            .json()
            .await
            .map_err(|e| AdanosProviderError::Provider(format!("adanos trending parse: {e}")))?;

        Ok(AdanosTrendingResult {
            timestamp: wire.timestamp,
            trending: wire.trending.into_iter().map(map_trending_item).collect(),
        })
    }
}

fn map_trending_item(wire: WireTrendingItem) -> AdanosTrendingItem {
    AdanosTrendingItem {
        ticker: wire.ticker,
        buzz_score: wire.buzz_score,
        sentiment_score: wire.sentiment_score,
        mentions: wire.mentions,
    }
}

// ---------------------------------------------------------------------------
// Polymarket fetcher
// ---------------------------------------------------------------------------

/// Fetches Polymarket events from the Adanos `/polymarket/events` endpoint.
pub struct AdanosPolymarketHttpFetcher {
    base_url: String,
}

impl Default for AdanosPolymarketHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl AdanosPolymarketHttpFetcher {
    /// Override the Adanos base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch Polymarket events for the given query.
    ///
    /// Reads `TDW_ADANOS_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`AdanosProviderError`] on missing key, network failure, or a
    /// non-2xx HTTP status.
    pub async fn fetch(&self, query: &AdanosPolymarketQuery) -> Result<AdanosPolymarketResult> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/polymarket/events", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("X-API-Key", &api_key)
            .query(&[("limit", query.limit.to_string())])
            .send()
            .await
            .map_err(|e| {
                AdanosProviderError::Provider(format!("adanos polymarket request: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AdanosProviderError::Provider(format!(
                "adanos polymarket returned {status}: {body}"
            )));
        }

        let wire: WirePolymarketEnvelope = response
            .json()
            .await
            .map_err(|e| AdanosProviderError::Provider(format!("adanos polymarket parse: {e}")))?;

        Ok(AdanosPolymarketResult {
            events: wire.events.into_iter().map(map_polymarket_event).collect(),
        })
    }
}

fn map_polymarket_event(wire: WirePolymarketEvent) -> AdanosPolymarketEvent {
    AdanosPolymarketEvent {
        id: wire.id,
        title: wire.title,
        probability: wire.probability,
        volume: wire.volume,
        expires_at: wire.expires_at,
    }
}
