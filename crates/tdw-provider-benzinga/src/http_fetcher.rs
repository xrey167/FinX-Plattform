#![cfg(feature = "http")]
//! Real Benzinga HTTP fetchers for `/news` and `/calendar/earnings`.
//!
//! Gated by the `http` feature. Requires `TDW_BENZINGA_API_KEY` to be set
//! in the environment. Live integration tests are additionally gated by
//! `TDW_BENZINGA_LIVE=1` so unattended CI stays offline.

use reqwest::Client;
use serde::Deserialize;

use crate::{
    API_KEY_ENV, BASE_URL, BenzingaEarningsItem, BenzingaEarningsQuery, BenzingaNewsItem,
    BenzingaNewsQuery, BenzingaProviderError, Result,
};

const USER_AGENT: &str = "tdw-provider-benzinga/0.1";

// ---------------------------------------------------------------------------
// Wire-format deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WireStock {
    name: String,
}

#[derive(Deserialize)]
struct WireNewsItem {
    id: String,
    title: String,
    #[serde(default)]
    teaser: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "publishedDate", default)]
    published_date: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    stocks: Vec<WireStock>,
}

#[derive(Deserialize)]
struct WireEarningsItem {
    id: String,
    date: String,
    ticker: String,
    #[serde(default)]
    eps: String,
    #[serde(rename = "epsEstimate", default)]
    eps_estimate: String,
    #[serde(default)]
    revenue: String,
}

#[derive(Deserialize)]
struct WireEarningsEnvelope {
    #[serde(default)]
    earnings: Vec<WireEarningsItem>,
}

// ---------------------------------------------------------------------------
// HTTP fetchers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(BenzingaProviderError::MissingApiKey)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| BenzingaProviderError::Provider(format!("benzinga client build: {e}")))
}

/// Fetches news articles from the Benzinga `/news` endpoint.
pub struct BenzingaNewsHttpFetcher {
    base_url: String,
}

impl Default for BenzingaNewsHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl BenzingaNewsHttpFetcher {
    /// Override the Benzinga base URL (useful for testing against a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch news articles for the given query.
    ///
    /// Reads `TDW_BENZINGA_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`BenzingaProviderError`] on missing key, network failure, or
    /// a non-2xx HTTP status.
    pub async fn fetch(&self, query: &BenzingaNewsQuery) -> Result<Vec<BenzingaNewsItem>> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/news", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("Authorization", format!("Token {api_key}"))
            .query(&[
                ("tickers", query.symbol.as_str()),
                ("pageSize", &query.page_size.to_string()),
                ("displayOutput", "full"),
            ])
            .send()
            .await
            .map_err(|e| BenzingaProviderError::Provider(format!("benzinga news request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BenzingaProviderError::Provider(format!(
                "benzinga news returned {status}: {body}"
            )));
        }

        let wire: Vec<WireNewsItem> = response
            .json()
            .await
            .map_err(|e| BenzingaProviderError::Provider(format!("benzinga news parse: {e}")))?;

        Ok(wire.into_iter().map(map_news_item).collect())
    }
}

fn map_news_item(wire: WireNewsItem) -> BenzingaNewsItem {
    BenzingaNewsItem {
        id: wire.id,
        title: wire.title,
        teaser: wire.teaser,
        url: wire.url,
        published_date: wire.published_date,
        source: wire.source,
        stocks: wire.stocks.into_iter().map(|s| s.name).collect(),
    }
}

/// Fetches earnings calendar entries from the Benzinga `/calendar/earnings`
/// endpoint.
pub struct BenzingaEarningsHttpFetcher {
    base_url: String,
}

impl Default for BenzingaEarningsHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl BenzingaEarningsHttpFetcher {
    /// Override the Benzinga base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch earnings entries for the given query.
    ///
    /// Reads `TDW_BENZINGA_API_KEY` from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`BenzingaProviderError`] on missing key, network failure, or
    /// a non-2xx HTTP status.
    pub async fn fetch(&self, query: &BenzingaEarningsQuery) -> Result<Vec<BenzingaEarningsItem>> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!("{}/calendar/earnings", self.base_url.trim_end_matches('/'));
        let response = client
            .get(&endpoint)
            .header("Authorization", format!("Token {api_key}"))
            .query(&[
                ("tickers", query.symbol.as_str()),
                ("dateFrom", query.date_from.as_str()),
                ("dateTo", query.date_to.as_str()),
            ])
            .send()
            .await
            .map_err(|e| {
                BenzingaProviderError::Provider(format!("benzinga earnings request: {e}"))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BenzingaProviderError::Provider(format!(
                "benzinga earnings returned {status}: {body}"
            )));
        }

        let envelope: WireEarningsEnvelope = response.json().await.map_err(|e| {
            BenzingaProviderError::Provider(format!("benzinga earnings parse: {e}"))
        })?;

        Ok(envelope
            .earnings
            .into_iter()
            .map(map_earnings_item)
            .collect())
    }
}

fn map_earnings_item(wire: WireEarningsItem) -> BenzingaEarningsItem {
    BenzingaEarningsItem {
        id: wire.id,
        date: wire.date,
        ticker: wire.ticker,
        eps: wire.eps,
        eps_estimate: wire.eps_estimate,
        revenue: wire.revenue,
    }
}
