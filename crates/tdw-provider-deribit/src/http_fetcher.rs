#![cfg(feature = "http")]
//! Real Deribit HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Deribit's public REST API directly
//! via `reqwest`. No API key is required for public endpoints. Live calls are
//! additionally gated by `TDW_DERIBIT_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    BASE_URL, DeribitFundingQuery, DeribitFundingRecord, DeribitGreeks, DeribitInstrument,
    DeribitInstrumentsQuery, DeribitKind, DeribitOrderBook, DeribitOrderBookQuery,
    DeribitProviderError, funding_request_path, instruments_request_path, order_book_request_path,
};

const USER_AGENT: &str = "tdw-provider-deribit/0.1";

// ---------------------------------------------------------------------------
// Serde response envelopes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DeribitEnvelope<T> {
    result: T,
}

#[derive(Deserialize)]
struct RawInstrument {
    instrument_name: String,
    kind: String,
    strike: Option<f64>,
    expiration_timestamp: Option<u64>,
    option_type: Option<String>,
    is_active: bool,
}

#[derive(Deserialize)]
struct RawOrderBook {
    instrument_name: String,
    bid_iv: Option<f64>,
    ask_iv: Option<f64>,
    mark_iv: Option<f64>,
    underlying_price: Option<f64>,
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
    greeks: Option<RawGreeks>,
}

#[derive(Deserialize)]
struct RawGreeks {
    delta: f64,
    gamma: f64,
    vega: f64,
    theta: f64,
}

#[derive(Deserialize)]
struct RawFundingRecord {
    timestamp: u64,
    interest: f64,
    index_price: f64,
}

// ---------------------------------------------------------------------------
// Instruments fetcher
// ---------------------------------------------------------------------------

/// Production Deribit instruments fetcher for `/public/get_instruments`.
#[derive(Clone, Debug)]
pub struct DeribitHttpInstrumentsFetcher {
    base_url: String,
}

impl Default for DeribitHttpInstrumentsFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl DeribitHttpInstrumentsFetcher {
    /// Override the Deribit base URL (useful for testing against a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `deribit` / `instruments` slot.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<DeribitInstrumentsQuery, DeribitInstrument> for DeribitHttpInstrumentsFetcher {
    const PROVIDER: &'static str = "deribit";
    const ENDPOINT: &'static str = "instruments";

    fn transform_query(params: Value) -> Result<DeribitInstrumentsQuery> {
        let currency = params
            .get("currency")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("deribit currency must be a string".to_string()))?;
        let kind = params
            .get("kind")
            .and_then(Value::as_str)
            .map(parse_kind)
            .transpose()?
            .unwrap_or(DeribitKind::Option);
        DeribitInstrumentsQuery::new(currency, kind).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &DeribitInstrumentsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let path = instruments_request_path(&query.currency, &query.kind)
            .map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));

        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("deribit instruments client build: {e}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("deribit instruments request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "deribit instruments returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("deribit instruments read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &DeribitInstrumentsQuery,
        raw: Bytes,
    ) -> Result<Vec<DeribitInstrument>> {
        let envelope: DeribitEnvelope<Vec<RawInstrument>> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("deribit instruments parse_json: {e}")))?;

        let instruments = envelope
            .result
            .into_iter()
            .map(|r| DeribitInstrument {
                instrument_name: r.instrument_name,
                kind: r.kind,
                strike: r.strike,
                expiration_timestamp: r.expiration_timestamp,
                option_type: r.option_type,
                is_active: r.is_active,
            })
            .collect();

        Ok(instruments)
    }
}

// ---------------------------------------------------------------------------
// Order book fetcher
// ---------------------------------------------------------------------------

/// Production Deribit order-book fetcher for `/public/get_order_book`.
#[derive(Clone, Debug)]
pub struct DeribitHttpOrderBookFetcher {
    base_url: String,
}

impl Default for DeribitHttpOrderBookFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl DeribitHttpOrderBookFetcher {
    /// Override the Deribit base URL (useful for testing against a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `deribit` / `order_book` slot.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<DeribitOrderBookQuery, DeribitOrderBook> for DeribitHttpOrderBookFetcher {
    const PROVIDER: &'static str = "deribit";
    const ENDPOINT: &'static str = "order_book";

    fn transform_query(params: Value) -> Result<DeribitOrderBookQuery> {
        let instrument_name = params
            .get("instrument_name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("deribit instrument_name must be a string".to_string())
            })?;
        let depth = params
            .get("depth")
            .and_then(Value::as_u64)
            .map(|d| d as u32)
            .unwrap_or(5);
        DeribitOrderBookQuery::new(instrument_name, depth)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &DeribitOrderBookQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let path = order_book_request_path(&query.instrument_name, query.depth)
            .map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));

        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("deribit order_book client build: {e}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("deribit order_book request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "deribit order_book returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("deribit order_book read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &DeribitOrderBookQuery,
        raw: Bytes,
    ) -> Result<Vec<DeribitOrderBook>> {
        let envelope: DeribitEnvelope<RawOrderBook> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("deribit order_book parse_json: {e}")))?;

        let r = envelope.result;
        Ok(vec![DeribitOrderBook {
            instrument_name: r.instrument_name,
            bid_iv: r.bid_iv,
            ask_iv: r.ask_iv,
            mark_iv: r.mark_iv,
            underlying_price: r.underlying_price,
            bids: r.bids,
            asks: r.asks,
            greeks: r.greeks.map(|g| DeribitGreeks {
                delta: g.delta,
                gamma: g.gamma,
                vega: g.vega,
                theta: g.theta,
            }),
        }])
    }
}

// ---------------------------------------------------------------------------
// Funding rate history fetcher
// ---------------------------------------------------------------------------

/// Production Deribit funding-rate-history fetcher for
/// `/public/get_funding_rate_history`.
#[derive(Clone, Debug)]
pub struct DeribitHttpFundingFetcher {
    base_url: String,
}

impl Default for DeribitHttpFundingFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl DeribitHttpFundingFetcher {
    /// Override the Deribit base URL (useful for testing against a local mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `deribit` / `funding_rate` slot.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<DeribitFundingQuery, DeribitFundingRecord> for DeribitHttpFundingFetcher {
    const PROVIDER: &'static str = "deribit";
    const ENDPOINT: &'static str = "funding_rate";

    fn transform_query(params: Value) -> Result<DeribitFundingQuery> {
        let instrument_name = params
            .get("instrument_name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("deribit instrument_name must be a string".to_string())
            })?;
        let start_ms = params
            .get("start_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::InvalidQuery("deribit start_ms must be a u64".to_string()))?;
        let end_ms = params
            .get("end_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::InvalidQuery("deribit end_ms must be a u64".to_string()))?;
        let count = params
            .get("count")
            .and_then(Value::as_u64)
            .map(|c| c as u32)
            .unwrap_or(100);
        DeribitFundingQuery::new(instrument_name, start_ms, end_ms, count)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &DeribitFundingQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let path = funding_request_path(
            &query.instrument_name,
            query.start_ms,
            query.end_ms,
            query.count,
        )
        .map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));

        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("deribit funding client build: {e}")))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("deribit funding request: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "deribit funding returned {status}: {body}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("deribit funding read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &DeribitFundingQuery,
        raw: Bytes,
    ) -> Result<Vec<DeribitFundingRecord>> {
        let envelope: DeribitEnvelope<Vec<RawFundingRecord>> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("deribit funding parse_json: {e}")))?;

        let records = envelope
            .result
            .into_iter()
            .map(|r| DeribitFundingRecord {
                timestamp: r.timestamp,
                interest: r.interest,
                index_price: r.index_price,
            })
            .collect();

        Ok(records)
    }
}

// ---------------------------------------------------------------------------
// Error mapping helper
// ---------------------------------------------------------------------------

impl From<DeribitProviderError> for Error {
    fn from(e: DeribitProviderError) -> Self {
        Error::Provider(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn parse_kind(s: &str) -> Result<DeribitKind> {
    match s {
        "option" => Ok(DeribitKind::Option),
        "future" => Ok(DeribitKind::Future),
        "perpetual" => Ok(DeribitKind::Perpetual),
        "future_combo" => Ok(DeribitKind::FutureCombo),
        "option_combo" => Ok(DeribitKind::OptionCombo),
        other => Err(Error::InvalidQuery(format!(
            "deribit unknown kind: {other}"
        ))),
    }
}
