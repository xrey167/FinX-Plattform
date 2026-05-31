#![cfg(feature = "http")]
//! Real Tradier REST backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the Tradier v1 API using
//! `reqwest`. All requests include `Authorization: Bearer {key}` and
//! `Accept: application/json`. The API key is read from the environment
//! variable `TDW_TRADIER_API_KEY`. Live integration tests are additionally
//! gated by `TDW_TRADIER_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{EquityHistoricalData, Quote};

use crate::{API_KEY_ENV, BASE_URL, TradierOptionsQuery, TradierQuoteQuery};

const USER_AGENT: &str = "tdw-provider-tradier/0.1";

// ---------------------------------------------------------------------------
// Quote fetcher
// ---------------------------------------------------------------------------

/// Production Tradier real-time quote fetcher.
#[derive(Clone, Debug)]
pub struct TradierHttpQuoteFetcher {
    base_url: String,
}

impl Default for TradierHttpQuoteFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TradierHttpQuoteFetcher {
    /// Override the Tradier base URL (useful for cassette tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `tradier` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

// ---------------------------------------------------------------------------
// Serde response shapes — Tradier quotes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TradierQuoteEnvelope {
    quotes: TradierQuotesWrapper,
}

#[derive(Deserialize)]
struct TradierQuotesWrapper {
    quote: TradierQuoteRaw,
}

#[derive(Deserialize)]
struct TradierQuoteRaw {
    symbol: String,
    #[allow(dead_code)]
    last: f64,
    bid: f64,
    ask: f64,
    #[allow(dead_code)]
    volume: u64,
    #[allow(dead_code)]
    open: Option<f64>,
    #[allow(dead_code)]
    high: Option<f64>,
    #[allow(dead_code)]
    low: Option<f64>,
    #[allow(dead_code)]
    close: Option<f64>,
    #[allow(dead_code)]
    change: Option<f64>,
}

#[async_trait]
impl Fetcher<TradierQuoteQuery, Quote> for TradierHttpQuoteFetcher {
    const PROVIDER: &'static str = "tradier";
    const ENDPOINT: &'static str = "quote";

    fn transform_query(params: Value) -> Result<TradierQuoteQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("tradier quote: `symbol` must be a string".to_string())
            })?;
        TradierQuoteQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &TradierQuoteQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let url = format!("{}/markets/quotes", self.base_url.trim_end_matches('/'));
        let client = build_client()?;
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Accept", "application/json")
            .query(&[("symbols", query.symbol.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "tradier quote returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(e.to_string()))
    }

    fn transform_data(&self, _query: &TradierQuoteQuery, raw: Bytes) -> Result<Vec<Quote>> {
        let envelope: TradierQuoteEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tradier quote parse: {e}")))?;
        let raw_quote = envelope.quotes.quote;
        Ok(vec![Quote {
            symbol: raw_quote.symbol,
            venue: "tradier".to_string(),
            ts: "".to_string(),
            bid: raw_quote.bid,
            ask: raw_quote.ask,
            bid_size: 0.0,
            ask_size: 0.0,
        }])
    }
}

// ---------------------------------------------------------------------------
// Options chain fetcher
// ---------------------------------------------------------------------------

/// Production Tradier options chain fetcher.
#[derive(Clone, Debug)]
pub struct TradierHttpOptionsFetcher {
    base_url: String,
}

impl Default for TradierHttpOptionsFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TradierHttpOptionsFetcher {
    /// Override the Tradier base URL (useful for cassette tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `tradier` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

// ---------------------------------------------------------------------------
// Serde response shapes — Tradier options chain
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TradierOptionsEnvelope {
    options: TradierOptionsWrapper,
}

#[derive(Deserialize)]
struct TradierOptionsWrapper {
    option: Vec<TradierOptionRaw>,
}

#[derive(Deserialize)]
struct TradierOptionRaw {
    symbol: String,
    #[allow(dead_code)]
    option_type: String,
    strike: f64,
    bid: f64,
    ask: f64,
    #[allow(dead_code)]
    open_interest: u64,
    #[allow(dead_code)]
    volume: u64,
    expiration_date: String,
}

/// Domain row produced by the options chain fetcher.
///
/// We re-use `EquityHistoricalData` here as the closest available domain type
/// (strike → close, bid/ask captured as open/high). A dedicated options domain
/// type can replace this once it exists in `tdw-domain`.
#[async_trait]
impl Fetcher<TradierOptionsQuery, EquityHistoricalData> for TradierHttpOptionsFetcher {
    const PROVIDER: &'static str = "tradier";
    const ENDPOINT: &'static str = "options_chain";

    fn transform_query(params: Value) -> Result<TradierOptionsQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("tradier options_chain: `symbol` must be a string".to_string())
            })?;
        let expiration = params
            .get("expiration")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery(
                    "tradier options_chain: `expiration` must be YYYY-MM-DD".to_string(),
                )
            })?;
        TradierOptionsQuery::new(symbol, expiration).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &TradierOptionsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = read_api_key()?;
        let url = format!(
            "{}/markets/options/chains",
            self.base_url.trim_end_matches('/')
        );
        let client = build_client()?;
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Accept", "application/json")
            .query(&[
                ("symbol", query.symbol.as_str()),
                ("expiration", query.expiration.as_str()),
                ("greeks", "false"),
            ])
            .send()
            .await
            .map_err(|e| Error::Provider(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "tradier options_chain returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(e.to_string()))
    }

    fn transform_data(
        &self,
        _query: &TradierOptionsQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        let envelope: TradierOptionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tradier options_chain parse: {e}")))?;
        let rows = envelope
            .options
            .option
            .into_iter()
            .map(|opt| EquityHistoricalData {
                symbol: opt.symbol,
                date: opt.expiration_date,
                open: opt.bid,
                high: opt.ask,
                low: opt.strike,
                close: opt.strike,
                volume: 0,
            })
            .collect();
        Ok(rows)
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
        .ok_or_else(|| Error::Provider(format!("tradier api key env {API_KEY_ENV} must be set")))
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| Error::Provider(format!("tradier http client: {e}")))
}
