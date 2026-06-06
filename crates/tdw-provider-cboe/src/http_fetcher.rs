#![cfg(feature = "http")]
//! Real CBOE HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to CBOE's public CDN endpoints directly
//! via `reqwest`. No API key is required. Live calls are additionally gated by
//! `TDW_CBOE_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    BASE_URL, CboeIndexQuery, CboeOptionsQuery, CboeProviderError, index_request_path,
    options_request_path,
};

const USER_AGENT: &str = "tdw-provider-cboe/0.1";

// ---------------------------------------------------------------------------
// Serde response shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OptionsEnvelope {
    data: OptionsData,
}

#[derive(Deserialize)]
struct OptionsData {
    options: Vec<RawOptionContract>,
}

#[derive(Deserialize)]
struct RawOptionContract {
    option: String,
    bid: f64,
    ask: f64,
    iv: f64,
    delta: f64,
    gamma: f64,
    theta: f64,
    open_interest: u64,
}

#[derive(Deserialize)]
struct IndexEnvelope {
    data: IndexData,
}

#[derive(Deserialize)]
struct IndexData {
    symbol: String,
    price: f64,
    change: f64,
    volume: u64,
}

// ---------------------------------------------------------------------------
// Domain output types (re-exported from lib.rs)
// ---------------------------------------------------------------------------

use crate::{CboeIndexQuote, CboeOptionContract};

// ---------------------------------------------------------------------------
// Options fetcher
// ---------------------------------------------------------------------------

/// Production CBOE delayed options chain fetcher.
#[derive(Clone, Debug)]
pub struct CboeHttpOptionsFetcher {
    base_url: String,
}

impl Default for CboeHttpOptionsFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl CboeHttpOptionsFetcher {
    /// Override the CBOE base URL (useful for testing against a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `cboe` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<CboeOptionsQuery, CboeOptionContract> for CboeHttpOptionsFetcher {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "options";

    fn transform_query(params: Value) -> Result<CboeOptionsQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("cboe options symbol must be a string".to_string())
            })?;
        CboeOptionsQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &CboeOptionsQuery, _creds: &Credentials) -> Result<Bytes> {
        let path =
            options_request_path(&query.symbol).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("cboe options client build: {e}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("cboe options request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "cboe options returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("cboe options read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &CboeOptionsQuery,
        raw: Bytes,
    ) -> Result<Vec<CboeOptionContract>> {
        let envelope: OptionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("cboe options parse_json: {e}")))?;

        let contracts = envelope
            .data
            .options
            .into_iter()
            .map(|r| CboeOptionContract {
                option: r.option,
                bid: r.bid,
                ask: r.ask,
                iv: r.iv,
                delta: r.delta,
                gamma: r.gamma,
                theta: r.theta,
                open_interest: r.open_interest,
            })
            .collect();

        Ok(contracts)
    }
}

// ---------------------------------------------------------------------------
// Index fetcher
// ---------------------------------------------------------------------------

/// Production CBOE US-index quote fetcher.
#[derive(Clone, Debug)]
pub struct CboeHttpIndexFetcher {
    base_url: String,
}

impl Default for CboeHttpIndexFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl CboeHttpIndexFetcher {
    /// Override the CBOE base URL (useful for testing against a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `cboe` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<CboeIndexQuery, CboeIndexQuote> for CboeHttpIndexFetcher {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "index_quotes";

    fn transform_query(params: Value) -> Result<CboeIndexQuery> {
        let index = params
            .get("index")
            .or_else(|| params.get("symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("cboe index must be a string".to_string()))?;
        CboeIndexQuery::new(index).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &CboeIndexQuery, _creds: &Credentials) -> Result<Bytes> {
        let path = index_request_path(&query.index).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("cboe index client build: {e}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("cboe index request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "cboe index returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("cboe index read body: {e}")))
    }

    fn transform_data(&self, _query: &CboeIndexQuery, raw: Bytes) -> Result<Vec<CboeIndexQuote>> {
        let envelope: IndexEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("cboe index parse_json: {e}")))?;

        Ok(vec![CboeIndexQuote {
            symbol: envelope.data.symbol,
            price: envelope.data.price,
            change: envelope.data.change,
            volume: envelope.data.volume,
        }])
    }
}

// ---------------------------------------------------------------------------
// Error mapping helper (unused directly but kept for completeness)
// ---------------------------------------------------------------------------

impl From<CboeProviderError> for Error {
    fn from(e: CboeProviderError) -> Self {
        Error::Provider(e.to_string())
    }
}
