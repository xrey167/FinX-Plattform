#![cfg(feature = "http")]
//! Real Glassnode HTTP backend implementing [`tdw_core::Fetcher`].
//!
//! Gated by the `http` feature. Reads `TDW_GLASSNODE_API_KEY` from the
//! environment and appends it as the `api_key` query parameter. Live
//! integration tests are additionally gated by `TDW_GLASSNODE_LIVE=1`.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{API_KEY_ENV, BASE_URL, GlassnodeDataPoint, GlassnodeMetric, GlassnodeMetricQuery};

const USER_AGENT: &str = "tdw-provider-glassnode/0.1";

/// Raw API response element — `[{"t": <unix>, "v": <f64>}]`.
#[derive(Deserialize)]
struct RawPoint {
    t: i64,
    v: f64,
}

/// Production Glassnode metric fetcher.
#[derive(Clone, Debug)]
pub struct GlassnodeHttpFetcher {
    base_url: String,
}

impl Default for GlassnodeHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl GlassnodeHttpFetcher {
    /// Create a new fetcher pointing at the canonical Glassnode base URL.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the base URL (useful for testing against a mock server).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `glassnode` provider
    /// name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<GlassnodeMetricQuery, GlassnodeDataPoint> for GlassnodeHttpFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "metric";

    fn transform_query(params: Value) -> Result<GlassnodeMetricQuery> {
        let asset = params
            .get("asset")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("glassnode asset must be a string".to_string()))?;
        let interval = params
            .get("interval")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("glassnode interval must be a string".to_string())
            })?;
        let metric: GlassnodeMetric = params
            .get("metric")
            .ok_or_else(|| Error::InvalidQuery("glassnode metric is required".to_string()))
            .and_then(|value| {
                serde_json::from_value(value.clone())
                    .map_err(|error| Error::InvalidQuery(format!("glassnode metric: {error}")))
            })?;
        GlassnodeMetricQuery::new(asset, metric, interval)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &GlassnodeMetricQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Provider(format!("glassnode api key env {API_KEY_ENV} must be set"))
            })?;

        let url = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            query.metric.api_path()
        );

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("glassnode client: {error}")))?;
        let response = client
            .get(&url)
            .query(&[
                ("a", query.asset.as_str()),
                ("i", query.interval.as_str()),
                ("api_key", api_key.as_str()),
            ])
            .send()
            .await
            .map_err(|error| Error::Provider(format!("glassnode extract_data: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "glassnode extract_data returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("glassnode read body: {error}")))
    }

    fn transform_data(
        &self,
        query: &GlassnodeMetricQuery,
        raw: Bytes,
    ) -> Result<Vec<GlassnodeDataPoint>> {
        let points: Vec<RawPoint> = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("glassnode json parse: {error}")))?;

        let rows = points
            .into_iter()
            .map(|point| GlassnodeDataPoint {
                timestamp: point.t,
                value: point.v,
                asset: query.asset.clone(),
                metric: query.metric.clone(),
            })
            .collect();

        Ok(rows)
    }
}
