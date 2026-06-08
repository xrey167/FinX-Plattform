#![cfg(feature = "http")]
//! Real Adanos HTTP fetchers for `/sentiment/stocks/{ticker}`,
//! `/trending/stocks`, and `/polymarket/events`.
//!
//! Gated by the `http` feature. Each fetcher implements the canonical
//! [`tdw_core::Fetcher`] trait: `extract_data` performs the HTTP call and
//! returns the raw response [`Bytes`], `transform_data` parses those bytes
//! into row models. Requires `TDW_ADANOS_API_KEY` to be set in the
//! environment. Live integration tests are additionally gated by
//! `TDW_ADANOS_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    API_KEY_ENV, AdanosMentions, AdanosPolymarketEvent, AdanosPolymarketQuery,
    AdanosSentimentQuery, AdanosSentimentResult, AdanosSentimentSources, AdanosTrendingItem,
    AdanosTrendingQuery, BASE_URL, PROVIDER_ID,
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
        .ok_or_else(|| Error::Provider(format!("adanos api key env {API_KEY_ENV} must be set")))
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Provider(format!("adanos client build: {e}")))
}

async fn read_body(response: reqwest::Response, label: &str) -> Result<Bytes> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "adanos {label} returned {status}: {body}"
        )));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("adanos {label} read body: {e}")))
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

fn map_trending_item(wire: WireTrendingItem) -> AdanosTrendingItem {
    AdanosTrendingItem {
        ticker: wire.ticker,
        buzz_score: wire.buzz_score,
        sentiment_score: wire.sentiment_score,
        mentions: wire.mentions,
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
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `adanos` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<AdanosSentimentQuery, AdanosSentimentResult> for AdanosSentimentHttpFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "sentiment";

    fn transform_query(params: Value) -> Result<AdanosSentimentQuery> {
        let query: AdanosSentimentQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("adanos sentiment query: {e}")))?;
        AdanosSentimentQuery::new(&query.ticker).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &AdanosSentimentQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
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
            .map_err(|e| Error::Provider(format!("adanos sentiment request: {e}")))?;
        read_body(response, "sentiment").await
    }

    fn transform_data(
        &self,
        _query: &AdanosSentimentQuery,
        raw: Bytes,
    ) -> Result<Vec<AdanosSentimentResult>> {
        let wire: WireSentimentResult = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("adanos sentiment parse: {e}")))?;
        Ok(vec![map_sentiment(wire)])
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
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `adanos` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<AdanosTrendingQuery, AdanosTrendingItem> for AdanosTrendingHttpFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "trending";

    fn transform_query(params: Value) -> Result<AdanosTrendingQuery> {
        let query: AdanosTrendingQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("adanos trending query: {e}")))?;
        AdanosTrendingQuery::new(query.limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &AdanosTrendingQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/trending/stocks", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("X-API-Key", &api_key)
            .query(&[("limit", query.limit.to_string())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("adanos trending request: {e}")))?;
        read_body(response, "trending").await
    }

    fn transform_data(
        &self,
        _query: &AdanosTrendingQuery,
        raw: Bytes,
    ) -> Result<Vec<AdanosTrendingItem>> {
        let wire: WireTrendingEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("adanos trending parse: {e}")))?;
        Ok(wire.trending.into_iter().map(map_trending_item).collect())
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
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `adanos` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<AdanosPolymarketQuery, AdanosPolymarketEvent> for AdanosPolymarketHttpFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "polymarket";

    fn transform_query(params: Value) -> Result<AdanosPolymarketQuery> {
        let query: AdanosPolymarketQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("adanos polymarket query: {e}")))?;
        AdanosPolymarketQuery::new(query.limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &AdanosPolymarketQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/polymarket/events", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("X-API-Key", &api_key)
            .query(&[("limit", query.limit.to_string())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("adanos polymarket request: {e}")))?;
        read_body(response, "polymarket").await
    }

    fn transform_data(
        &self,
        _query: &AdanosPolymarketQuery,
        raw: Bytes,
    ) -> Result<Vec<AdanosPolymarketEvent>> {
        let wire: WirePolymarketEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("adanos polymarket parse: {e}")))?;
        Ok(wire.events.into_iter().map(map_polymarket_event).collect())
    }
}

// ---------------------------------------------------------------------------
// Offline unit tests (no network): exercise transform_data on fixture bytes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentiment_transform_data_parses_fixture() {
        let raw = Bytes::from_static(
            br#"{
                "ticker": "AAPL",
                "timestamp": 1704153600,
                "sentimentScore": 0.72,
                "buzzScore": 85,
                "sources": { "reddit": 0.68, "twitter": 0.75, "news": 0.74 },
                "mentions": { "reddit": 1250, "twitter": 8500, "news": 45 },
                "trend": "bullish"
            }"#,
        );
        let fetcher = AdanosSentimentHttpFetcher::default();
        let query =
            AdanosSentimentQuery::new("AAPL").unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|e| panic!("transform must succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ticker, "AAPL");
        assert_eq!(rows[0].buzz_score, 85);
        assert_eq!(rows[0].trend, "bullish");
        assert_eq!(rows[0].mentions.twitter, 8500);
    }

    #[test]
    fn trending_transform_data_parses_two_rows() {
        let raw = Bytes::from_static(
            br#"{
                "timestamp": 1704153600,
                "trending": [
                    { "ticker": "NVDA", "buzzScore": 98, "sentimentScore": 0.85, "mentions": 25000 },
                    { "ticker": "AAPL", "buzzScore": 85, "sentimentScore": 0.72, "mentions": 18000 }
                ]
            }"#,
        );
        let fetcher = AdanosTrendingHttpFetcher::default();
        let query =
            AdanosTrendingQuery::new(10).unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|e| panic!("transform must succeed: {e}"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ticker, "NVDA");
        assert_eq!(rows[0].buzz_score, 98);
        assert_eq!(rows[1].ticker, "AAPL");
    }

    #[test]
    fn polymarket_transform_data_parses_one_event() {
        let raw = Bytes::from_static(
            br#"{
                "events": [
                    {
                        "id": "abc123",
                        "title": "Will BTC exceed $100k by Dec 2024?",
                        "probability": 0.42,
                        "volume": 1500000.0,
                        "expiresAt": "2024-12-31T23:59:59Z"
                    }
                ]
            }"#,
        );
        let fetcher = AdanosPolymarketHttpFetcher::default();
        let query =
            AdanosPolymarketQuery::new(5).unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|e| panic!("transform must succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "abc123");
        assert_eq!(rows[0].expires_at, "2024-12-31T23:59:59Z");
    }

    #[test]
    fn transform_query_validates_sentiment_ticker() {
        let ok = AdanosSentimentHttpFetcher::transform_query(serde_json::json!({"ticker": "aapl"}))
            .unwrap_or_else(|e| panic!("valid query must parse: {e}"));
        assert_eq!(ok.ticker, "AAPL");
        assert!(
            AdanosSentimentHttpFetcher::transform_query(serde_json::json!({"ticker": ""})).is_err()
        );
    }
}
