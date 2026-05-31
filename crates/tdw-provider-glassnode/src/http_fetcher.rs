#![cfg(feature = "http")]
//! Real Glassnode HTTP backend.
//!
//! Gated by the `http` feature. Reads `TDW_GLASSNODE_API_KEY` from the
//! environment and appends it as the `api_key` query parameter. Live
//! integration tests are additionally gated by `TDW_GLASSNODE_LIVE=1`.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;

use crate::{
    API_KEY_ENV, BASE_URL, GlassnodeDataPoint, GlassnodeMetricQuery, GlassnodeProviderError, Result,
};

const USER_AGENT: &str = "tdw-provider-glassnode/0.1";

/// Raw API response element — `[{"t": <unix>, "v": <f64>}]`.
#[derive(Deserialize)]
struct RawPoint {
    t: i64,
    v: f64,
}

/// Production Glassnode HTTP fetcher.
#[derive(Clone, Debug)]
pub struct GlassnodeHttpFetcher {
    base_url: String,
    client: Client,
}

impl GlassnodeHttpFetcher {
    /// Create a new fetcher, building a shared `reqwest::Client`.
    ///
    /// # Errors
    ///
    /// Returns [`GlassnodeProviderError::Provider`] if the HTTP client cannot
    /// be constructed (e.g. missing TLS backend).
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| GlassnodeProviderError::Provider(format!("client build: {e}")))?;
        Ok(Self {
            base_url: BASE_URL.to_string(),
            client,
        })
    }

    /// Override the base URL (useful for testing against a mock server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch raw bytes from the Glassnode API for the given query.
    ///
    /// # Errors
    ///
    /// Returns [`GlassnodeProviderError::MissingApiKey`] when
    /// `TDW_GLASSNODE_API_KEY` is absent or empty, and
    /// [`GlassnodeProviderError::Provider`] for any HTTP or I/O error.
    pub async fn fetch_raw(&self, query: &GlassnodeMetricQuery) -> Result<Bytes> {
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or(GlassnodeProviderError::MissingApiKey)?;

        let url = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            query.metric.api_path()
        );

        let response = self
            .client
            .get(&url)
            .query(&[
                ("a", query.asset.as_str()),
                ("i", query.interval.as_str()),
                ("api_key", api_key.as_str()),
            ])
            .send()
            .await
            .map_err(|e| GlassnodeProviderError::Provider(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable body>"));
            return Err(GlassnodeProviderError::Provider(format!(
                "glassnode returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| GlassnodeProviderError::Provider(format!("read body: {e}")))
    }

    /// Decode raw API bytes into typed data points.
    ///
    /// # Errors
    ///
    /// Returns [`GlassnodeProviderError::Provider`] if JSON parsing fails.
    pub fn decode(
        &self,
        query: &GlassnodeMetricQuery,
        raw: Bytes,
    ) -> Result<Vec<GlassnodeDataPoint>> {
        let points: Vec<RawPoint> = serde_json::from_slice(&raw)
            .map_err(|e| GlassnodeProviderError::Provider(format!("json parse: {e}")))?;

        let rows = points
            .into_iter()
            .map(|p| GlassnodeDataPoint {
                timestamp: p.t,
                value: p.v,
                asset: query.asset.clone(),
                metric: query.metric.clone(),
            })
            .collect();

        Ok(rows)
    }

    /// Convenience method: fetch and decode in one call.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Self::fetch_raw`] and [`Self::decode`].
    pub async fn fetch(&self, query: &GlassnodeMetricQuery) -> Result<Vec<GlassnodeDataPoint>> {
        let raw = self.fetch_raw(query).await?;
        self.decode(query, raw)
    }
}
