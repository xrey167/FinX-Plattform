#![cfg(feature = "http")]
//! Real Benzinga HTTP fetchers for `/news` and `/calendar/earnings`.
//!
//! Gated by the `http` feature. Both fetchers implement the canonical
//! [`tdw_core::Fetcher`] trait: [`Fetcher::extract_data`] performs the HTTP
//! call and returns the raw response bytes, while [`Fetcher::transform_data`]
//! parses those bytes into row models offline. Requires `TDW_BENZINGA_API_KEY`
//! to be set in the environment. Live integration tests are additionally
//! gated by `TDW_BENZINGA_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    API_KEY_ENV, BASE_URL, BenzingaEarningsItem, BenzingaEarningsQuery, BenzingaNewsItem,
    BenzingaNewsQuery, BenzingaProviderError,
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
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Provider(BenzingaProviderError::MissingApiKey.to_string()))
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| Error::Provider(format!("benzinga client build: {e}")))
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

// ---------------------------------------------------------------------------
// News fetcher
// ---------------------------------------------------------------------------

/// Fetches news articles from the Benzinga `/news` endpoint.
#[derive(Clone, Debug)]
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

    /// Registry entry advertised under the canonical `benzinga` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<BenzingaNewsQuery, BenzingaNewsItem> for BenzingaNewsHttpFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "news";

    fn transform_query(params: Value) -> Result<BenzingaNewsQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("benzinga symbol must be a string".to_string()))?;
        let page_size = params
            .get("page_size")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| {
                Error::InvalidQuery("benzinga page_size must be an integer".to_string())
            })?;
        BenzingaNewsQuery::new(symbol, page_size).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &BenzingaNewsQuery, _creds: &Credentials) -> Result<Bytes> {
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
            .map_err(|e| Error::Provider(format!("benzinga news request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "benzinga news returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("benzinga news read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &BenzingaNewsQuery,
        raw: Bytes,
    ) -> Result<Vec<BenzingaNewsItem>> {
        let wire: Vec<WireNewsItem> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("benzinga news parse: {e}")))?;
        Ok(wire.into_iter().map(map_news_item).collect())
    }
}

// ---------------------------------------------------------------------------
// Earnings fetcher
// ---------------------------------------------------------------------------

/// Fetches earnings calendar entries from the Benzinga `/calendar/earnings`
/// endpoint.
#[derive(Clone, Debug)]
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

    /// Registry entry advertised under the canonical `benzinga` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<BenzingaEarningsQuery, BenzingaEarningsItem> for BenzingaEarningsHttpFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "earnings";

    fn transform_query(params: Value) -> Result<BenzingaEarningsQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("benzinga symbol must be a string".to_string()))?;
        let date_from = params
            .get("date_from")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("benzinga date_from must be a string".to_string())
            })?;
        let date_to = params
            .get("date_to")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("benzinga date_to must be a string".to_string()))?;
        BenzingaEarningsQuery::new(symbol, date_from, date_to)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &BenzingaEarningsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
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
            .map_err(|e| Error::Provider(format!("benzinga earnings request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "benzinga earnings returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("benzinga earnings read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &BenzingaEarningsQuery,
        raw: Bytes,
    ) -> Result<Vec<BenzingaEarningsItem>> {
        let envelope: WireEarningsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("benzinga earnings parse: {e}")))?;
        Ok(envelope
            .earnings
            .into_iter()
            .map(map_earnings_item)
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Offline unit tests for transform_data (no network)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const NEWS_FIXTURE: &[u8] = br#"[
        {
            "id": "abc123",
            "title": "Apple Q1 Beats Estimates",
            "teaser": "Apple reported strong Q1 results.",
            "url": "https://example.benzinga.com/apple-q1",
            "publishedDate": "2024-01-25T09:30:00Z",
            "source": "Benzinga",
            "stocks": [{ "name": "AAPL" }]
        },
        {
            "id": "abc124",
            "title": "Apple Guidance Raised",
            "teaser": "Management raised full-year guidance.",
            "url": "https://example.benzinga.com/apple-guidance",
            "publishedDate": "2024-01-26T10:00:00Z",
            "source": "Benzinga",
            "stocks": [{ "name": "AAPL" }]
        }
    ]"#;

    const EARNINGS_FIXTURE: &[u8] = br#"{
        "earnings": [
            {
                "id": "e1",
                "date": "2024-01-25",
                "ticker": "AAPL",
                "eps": "2.18",
                "epsEstimate": "2.10",
                "revenue": "119575000000"
            }
        ]
    }"#;

    #[test]
    fn news_transform_data_parses_fixture_offline() {
        let fetcher = BenzingaNewsHttpFetcher::default();
        let query =
            BenzingaNewsQuery::new("AAPL", 5).unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .transform_data(&query, Bytes::from_static(NEWS_FIXTURE))
            .unwrap_or_else(|e| panic!("transform must succeed: {e}"));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "abc123");
        assert_eq!(rows[0].title, "Apple Q1 Beats Estimates");
        assert_eq!(rows[0].source, "Benzinga");
        assert_eq!(rows[0].published_date, "2024-01-25T09:30:00Z");
        assert_eq!(rows[0].stocks, vec!["AAPL".to_string()]);
        assert_eq!(rows[1].id, "abc124");
    }

    #[test]
    fn earnings_transform_data_parses_fixture_offline() {
        let fetcher = BenzingaEarningsHttpFetcher::default();
        let query = BenzingaEarningsQuery::new("AAPL", "2024-01-01", "2024-12-31")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = fetcher
            .transform_data(&query, Bytes::from_static(EARNINGS_FIXTURE))
            .unwrap_or_else(|e| panic!("transform must succeed: {e}"));

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "e1");
        assert_eq!(rows[0].ticker, "AAPL");
        assert_eq!(rows[0].eps, "2.18");
        assert_eq!(rows[0].eps_estimate, "2.10");
        assert_eq!(rows[0].revenue, "119575000000");
    }

    #[test]
    fn provider_and_endpoint_constants_are_unique() {
        assert_eq!(
            <BenzingaNewsHttpFetcher as Fetcher<BenzingaNewsQuery, BenzingaNewsItem>>::PROVIDER,
            "benzinga"
        );
        assert_eq!(
            <BenzingaNewsHttpFetcher as Fetcher<BenzingaNewsQuery, BenzingaNewsItem>>::ENDPOINT,
            "news"
        );
        assert_eq!(
            <BenzingaEarningsHttpFetcher as Fetcher<
                BenzingaEarningsQuery,
                BenzingaEarningsItem,
            >>::ENDPOINT,
            "earnings"
        );
    }
}
