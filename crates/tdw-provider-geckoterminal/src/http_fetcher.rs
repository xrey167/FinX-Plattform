//! Real GeckoTerminal HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the public GeckoTerminal REST API
//! (no auth required). Live integration tests are additionally gated by
//! `TDW_GECKOTERMINAL_LIVE=1`.

#![cfg(feature = "http")]

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{ACCEPT_HEADER, BASE_URL, DexPool, GeckoTerminalPoolQuery};

const USER_AGENT: &str = "tdw-provider-geckoterminal/0.1";

// ---------------------------------------------------------------------------
// Internal serde types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GeckoEnvelope {
    data: GeckoData,
}

#[derive(Deserialize)]
struct GeckoData {
    id: String,
    attributes: GeckoPoolAttributes,
}

#[derive(Deserialize)]
struct GeckoPoolAttributes {
    name: String,
    #[serde(default)]
    base_token_price_usd: Option<String>,
    #[serde(default)]
    quote_token_price_usd: Option<String>,
    #[serde(default)]
    reserve_in_usd: Option<String>,
    #[serde(default)]
    volume_usd: Option<GeckoVolumeUsd>,
    #[serde(default)]
    price_change_percentage: Option<GeckoPriceChange>,
    #[serde(default)]
    pool_created_at: Option<String>,
}

#[derive(Deserialize)]
struct GeckoVolumeUsd {
    #[serde(default)]
    h24: Option<String>,
    #[serde(default)]
    h6: Option<String>,
    #[serde(default)]
    h1: Option<String>,
}

#[derive(Deserialize)]
struct GeckoPriceChange {
    #[serde(default)]
    h24: Option<String>,
    #[serde(default)]
    h6: Option<String>,
    #[serde(default)]
    h1: Option<String>,
}

// ---------------------------------------------------------------------------
// List envelope (trending / token-pools)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GeckoListEnvelope {
    data: Vec<GeckoData>,
}

// ---------------------------------------------------------------------------
// Fetcher impl
// ---------------------------------------------------------------------------

/// Provider specification for the GeckoTerminal pool fetcher.
///
/// Uses the public GeckoTerminal REST API; no API key is required.
pub struct GeckoTerminalPoolSpec;

impl ProviderSpec for GeckoTerminalPoolSpec {
    const PROVIDER: &'static str = "geckoterminal";
    const ENDPOINT: &'static str = "pool";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "geckoterminal client";
    const SEND_ERR: &'static str = "geckoterminal extract_data";
    const RETURNED_ERR: &'static str = "geckoterminal pool returned";
    const READ_BODY_ERR: &'static str = "geckoterminal read body";

    type Query = GeckoTerminalPoolQuery;
    type Data = DexPool;

    fn transform_query(params: serde_json::Value) -> Result<GeckoTerminalPoolQuery> {
        let network = params
            .get("network")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("geckoterminal network must be a string".to_string())
            })?;
        let pool_address = params
            .get("pool_address")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("geckoterminal pool_address must be a string".to_string())
            })?;
        GeckoTerminalPoolQuery::new(network, pool_address)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &GeckoTerminalPoolQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let url = format!(
            "{}/networks/{}/pools/{}",
            base_url.trim_end_matches('/'),
            query.network,
            query.pool_address,
        );
        Ok(client.get(&url).header("Accept", ACCEPT_HEADER))
    }

    fn transform_data(query: &GeckoTerminalPoolQuery, raw: Bytes) -> Result<Vec<DexPool>> {
        let envelope: GeckoEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("geckoterminal parse_json: {e}")))?;
        Ok(vec![gecko_data_to_pool(
            envelope.data,
            &query.network,
            &query.pool_address,
        )])
    }
}

/// Production GeckoTerminal pool fetcher.
///
/// Uses the public GeckoTerminal REST API; no API key is required.
pub type GeckoTerminalHttpFetcher = HttpFetcher<GeckoTerminalPoolSpec>;

// ---------------------------------------------------------------------------
// Helper: convert a GeckoData record into a DexPool
// ---------------------------------------------------------------------------

fn gecko_data_to_pool(data: GeckoData, network: &str, pool_address: &str) -> DexPool {
    let attr = data.attributes;
    let base_token_price_usd = attr
        .base_token_price_usd
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok());
    let quote_token_price_usd = attr
        .quote_token_price_usd
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok());
    let reserve_in_usd = attr
        .reserve_in_usd
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok());

    let (volume_usd_h24, volume_usd_h6, volume_usd_h1) = attr
        .volume_usd
        .map(|v| {
            (
                v.h24.as_deref().and_then(|s| s.parse::<f64>().ok()),
                v.h6.as_deref().and_then(|s| s.parse::<f64>().ok()),
                v.h1.as_deref().and_then(|s| s.parse::<f64>().ok()),
            )
        })
        .unwrap_or((None, None, None));

    let (price_change_h24, price_change_h6, price_change_h1) = attr
        .price_change_percentage
        .map(|p| {
            (
                p.h24.as_deref().and_then(|s| s.parse::<f64>().ok()),
                p.h6.as_deref().and_then(|s| s.parse::<f64>().ok()),
                p.h1.as_deref().and_then(|s| s.parse::<f64>().ok()),
            )
        })
        .unwrap_or((None, None, None));

    DexPool {
        id: data.id,
        name: attr.name,
        network: network.to_string(),
        pool_address: pool_address.to_string(),
        base_token_price_usd,
        quote_token_price_usd,
        reserve_in_usd,
        volume_usd_h24,
        volume_usd_h6,
        volume_usd_h1,
        price_change_h24,
        price_change_h6,
        price_change_h1,
        pool_created_at: attr.pool_created_at,
    }
}

// ---------------------------------------------------------------------------
// Trending and token-pools helpers (not Fetcher impls — stand-alone fns)
// ---------------------------------------------------------------------------

/// Fetch trending pools for `network` directly, returning raw [`Bytes`].
///
/// # Errors
///
/// Returns [`tdw_core::Error`] on HTTP or I/O failure.
pub async fn fetch_trending_raw(base_url: &str, network: &str) -> Result<Bytes> {
    let url = format!(
        "{}/networks/{}/trending_pools",
        base_url.trim_end_matches('/'),
        network,
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "geckoterminal client")?;
    let response = client
        .get(&url)
        .header("Accept", ACCEPT_HEADER)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("geckoterminal trending: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "geckoterminal trending returned {status}: {body}"
        )));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("geckoterminal trending body: {e}")))
}

/// Parse a trending-pools or token-pools response `Bytes` into a list of
/// [`DexPool`] records.
///
/// `network` is used to populate the [`DexPool::network`] field (the API
/// response does not repeat it).
///
/// # Errors
///
/// Returns [`tdw_core::Error::Provider`] when JSON parsing fails.
pub fn parse_pool_list(raw: Bytes, network: &str) -> Result<Vec<DexPool>> {
    let envelope: GeckoListEnvelope = serde_json::from_slice(&raw)
        .map_err(|e| Error::Provider(format!("geckoterminal parse_pool_list: {e}")))?;
    Ok(envelope
        .data
        .into_iter()
        .map(|d| {
            // The id is `{network}_{address}` — extract the address part.
            let pool_address = d.id.split_once('_').map(|x| x.1).unwrap_or("").to_string();
            gecko_data_to_pool(d, network, &pool_address)
        })
        .collect())
}

/// Fetch pools for `token_address` on `network`, returning raw [`Bytes`].
///
/// # Errors
///
/// Returns [`tdw_core::Error`] on HTTP or I/O failure.
pub async fn fetch_token_pools_raw(
    base_url: &str,
    network: &str,
    token_address: &str,
) -> Result<Bytes> {
    let url = format!(
        "{}/networks/{}/tokens/{}/pools",
        base_url.trim_end_matches('/'),
        network,
        token_address,
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "geckoterminal client")?;
    let response = client
        .get(&url)
        .header("Accept", ACCEPT_HEADER)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("geckoterminal token_pools: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "geckoterminal token_pools returned {status}: {body}"
        )));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("geckoterminal token_pools body: {e}")))
}
