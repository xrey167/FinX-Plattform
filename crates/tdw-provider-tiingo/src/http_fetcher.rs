//! Real Tiingo HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Requires `TDW_TIINGO_API_KEY` at runtime.
//! Live integration tests are additionally gated by `TDW_TIINGO_LIVE=1`.

#![cfg(feature = "http")]

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{API_KEY_ENV, BASE_URL, TiingoHistoricalQuery, TiingoNewsQuery, TiingoProviderError};

const USER_AGENT: &str = "tdw-provider-tiingo/0.1";

// ---------------------------------------------------------------------------
// Response deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TiingoDailyPrice {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Deserialize)]
struct TiingoNewsItem {
    id: u64,
    title: String,
    #[serde(rename = "publishedDate")]
    published_date: String,
    url: String,
    source: String,
}

// ---------------------------------------------------------------------------
// Public output type for news
// ---------------------------------------------------------------------------

/// A single Tiingo news article returned by the news fetcher.
#[derive(Clone, Debug, PartialEq)]
pub struct TiingoNewsArticle {
    pub id: u64,
    pub title: String,
    pub published_date: String,
    pub url: String,
    pub source: String,
}

// ---------------------------------------------------------------------------
// Historical price fetcher
// ---------------------------------------------------------------------------

/// Production Tiingo daily historical price fetcher.
#[derive(Clone, Debug)]
pub struct TiingoHttpHistoricalFetcher {
    base_url: String,
}

impl Default for TiingoHttpHistoricalFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TiingoHttpHistoricalFetcher {
    /// Override the Tiingo base URL (useful for tests against a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `tiingo` provider name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TiingoHistoricalQuery, MarketDataBar> for TiingoHttpHistoricalFetcher {
    const PROVIDER: &'static str = "tiingo";
    const ENDPOINT: &'static str = "historical";

    fn transform_query(params: Value) -> Result<TiingoHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("tiingo symbol must be a non-empty string".to_string())
            })?;
        TiingoHistoricalQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &TiingoHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let endpoint = format!(
            "{}/daily/{}/prices",
            self.base_url.trim_end_matches('/'),
            query.symbol,
        );
        let query_params = [("startDate", "2024-01-01".to_string()), ("token", api_key)];
        let client = build_client()?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("tiingo historical extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "tiingo historical returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("tiingo historical read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &TiingoHistoricalQuery,
        raw: Bytes,
    ) -> Result<Vec<MarketDataBar>> {
        let prices: Vec<TiingoDailyPrice> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tiingo historical parse_json: {e}")))?;
        let mut rows = Vec::with_capacity(prices.len());
        for price in prices {
            rows.push(MarketDataBar {
                symbol: query.symbol.clone(),
                venue: "tiingo".to_string(),
                granularity: TimeGranularity::Day,
                ts: price.date,
                open: price.open,
                high: price.high,
                low: price.low,
                close: price.close,
                volume: price.volume,
                source: "tiingo".to_string(),
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// News fetcher
// ---------------------------------------------------------------------------

/// Production Tiingo news feed fetcher.
#[derive(Clone, Debug)]
pub struct TiingoHttpNewsFetcher {
    base_url: String,
}

impl Default for TiingoHttpNewsFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TiingoHttpNewsFetcher {
    /// Override the Tiingo base URL (useful for tests against a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `tiingo` provider name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TiingoNewsQuery, TiingoNewsArticle> for TiingoHttpNewsFetcher {
    const PROVIDER: &'static str = "tiingo";
    const ENDPOINT: &'static str = "news";

    fn transform_query(params: Value) -> Result<TiingoNewsQuery> {
        let tickers_value = params.get("tickers").ok_or_else(|| {
            Error::InvalidQuery("tiingo news requires a 'tickers' field".to_string())
        })?;
        let tickers: Vec<String> = match tickers_value {
            Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        Error::InvalidQuery("each ticker must be a string".to_string())
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            Value::String(s) => vec![s.clone()],
            _ => {
                return Err(Error::InvalidQuery(
                    "tiingo news 'tickers' must be a string or array".to_string(),
                ));
            }
        };
        let refs: Vec<&str> = tickers.iter().map(String::as_str).collect();
        TiingoNewsQuery::new(&refs).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &TiingoNewsQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let tickers_param = query.tickers.join(",").to_ascii_lowercase();
        let endpoint = format!("{}/news", self.base_url.trim_end_matches('/'));
        let query_params = [("tickers", tickers_param), ("token", api_key)];
        let client = build_client()?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("tiingo news extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "tiingo news returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("tiingo news read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &TiingoNewsQuery,
        raw: Bytes,
    ) -> Result<Vec<TiingoNewsArticle>> {
        let items: Vec<TiingoNewsItem> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tiingo news parse_json: {e}")))?;
        Ok(items
            .into_iter()
            .map(|item| TiingoNewsArticle {
                id: item.id,
                title: item.title,
                published_date: item.published_date,
                url: item.url,
                source: item.source,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            Error::Provider(
                TiingoProviderError::Provider(format!("{API_KEY_ENV} not set")).to_string(),
            )
        })
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| Error::Provider(format!("tiingo http client: {e}")))
}
