#![cfg(feature = "http")]
//! Real ECB Statistical Data Warehouse HTTP fetcher.
//!
//! Gated by the `http` feature. Talks to the ECB SDW REST API
//! `/data/{flow}/{key}` endpoint via `reqwest`. No authentication is required.
//! The live integration test is additionally gated by `TDW_ECB_LIVE=1` so
//! unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{BASE_URL, EcbDataQuery, EcbObservation, parse_ecb_value};

const USER_AGENT: &str = "tdw-provider-ecb/0.1";

/// Production ECB SDW data fetcher.
#[derive(Clone, Debug)]
pub struct EcbHttpDataFetcher {
    base_url: String,
}

impl Default for EcbHttpDataFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl EcbHttpDataFetcher {
    /// Override the ECB base URL (useful for testing against a local mock
    /// server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `ecb` provider name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<EcbDataQuery, EcbObservation> for EcbHttpDataFetcher {
    const PROVIDER: &'static str = "ecb";
    const ENDPOINT: &'static str = "data";

    fn transform_query(params: Value) -> Result<EcbDataQuery> {
        let flow = params
            .get("flow")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb flow must be a string".to_string()))?;
        let key = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb key must be a string".to_string()))?;
        let start_period = params
            .get("start_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb start_period must be a string".to_string()))?;
        let end_period = params
            .get("end_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb end_period must be a string".to_string()))?;
        EcbDataQuery::new(flow, key, start_period, end_period)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(&self, query: &EcbDataQuery, _creds: &Credentials) -> Result<Bytes> {
        let endpoint = format!(
            "{}/data/{}/{}",
            self.base_url.trim_end_matches('/'),
            query.flow,
            query.key,
        );
        let query_params = [
            ("format", "jsondata"),
            ("startPeriod", query.start_period.as_str()),
            ("endPeriod", query.end_period.as_str()),
        ];

        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("ecb client build: {error}")))?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("ecb extract_data: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "ecb extract_data returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("ecb read body: {error}")))
    }

    fn transform_data(&self, query: &EcbDataQuery, raw: Bytes) -> Result<Vec<EcbObservation>> {
        let v: Value = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("ecb parse_json: {error}")))?;
        parse_ecb_value(&v, &query.flow, &query.key)
            .map_err(|error| Error::Provider(error.to_string()))
    }
}
