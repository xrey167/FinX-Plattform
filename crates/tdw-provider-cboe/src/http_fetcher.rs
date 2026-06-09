#![cfg(feature = "http")]
//! Real CBOE HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to CBOE's public CDN endpoints directly
//! via `reqwest`. No API key is required. Live calls are additionally gated by
//! `TDW_CBOE_LIVE=1` so unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

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

/// Provider specification for the CBOE delayed options chain fetcher.
pub struct CboeOptionsSpec;

impl ProviderSpec for CboeOptionsSpec {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "options";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "cboe options client build";
    const SEND_ERR: &'static str = "cboe options request";
    const RETURNED_ERR: &'static str = "cboe options returned";
    const READ_BODY_ERR: &'static str = "cboe options read body";

    type Query = CboeOptionsQuery;
    type Data = CboeOptionContract;

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

    fn build_request(
        base_url: &str,
        query: &CboeOptionsQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let path =
            options_request_path(&query.symbol).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", base_url.trim_end_matches('/'));
        Ok(client.get(&url))
    }

    fn transform_data(_query: &CboeOptionsQuery, raw: Bytes) -> Result<Vec<CboeOptionContract>> {
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

/// Production CBOE delayed options chain fetcher.
pub type CboeHttpOptionsFetcher = HttpFetcher<CboeOptionsSpec>;

// ---------------------------------------------------------------------------
// Index fetcher
// ---------------------------------------------------------------------------

/// Provider specification for the CBOE US-index quote fetcher.
pub struct CboeIndexSpec;

impl ProviderSpec for CboeIndexSpec {
    const PROVIDER: &'static str = "cboe";
    const ENDPOINT: &'static str = "index_quotes";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "cboe index client build";
    const SEND_ERR: &'static str = "cboe index request";
    const RETURNED_ERR: &'static str = "cboe index returned";
    const READ_BODY_ERR: &'static str = "cboe index read body";

    type Query = CboeIndexQuery;
    type Data = CboeIndexQuote;

    fn transform_query(params: Value) -> Result<CboeIndexQuery> {
        let index = params
            .get("index")
            .or_else(|| params.get("symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("cboe index must be a string".to_string()))?;
        CboeIndexQuery::new(index).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &CboeIndexQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let path = index_request_path(&query.index).map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}{path}", base_url.trim_end_matches('/'));
        Ok(client.get(&url))
    }

    fn transform_data(_query: &CboeIndexQuery, raw: Bytes) -> Result<Vec<CboeIndexQuote>> {
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

/// Production CBOE US-index quote fetcher.
pub type CboeHttpIndexFetcher = HttpFetcher<CboeIndexSpec>;

// ---------------------------------------------------------------------------
// Error mapping helper (unused directly but kept for completeness)
// ---------------------------------------------------------------------------

impl From<CboeProviderError> for Error {
    fn from(e: CboeProviderError) -> Self {
        Error::Provider(e.to_string())
    }
}
