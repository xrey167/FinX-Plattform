//! Real Alpaca stock-bars backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Alpaca's historical stock
//! bars endpoint directly via `reqwest`. Live calls require
//! `APCA_API_KEY_ID` and `APCA_API_SECRET_KEY`; the live integration
//! test is additionally gated by `TDW_ALPACA_LIVE=1` so unattended CI
//! stays offline.

#![cfg(feature = "http")]

use std::collections::BTreeMap;

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{
    API_KEY_HEADER, API_SECRET_HEADER, AlpacaStockBarsQuery, BASE_URL, stock_bars_request,
};

const API_KEY_ENV: &str = "APCA_API_KEY_ID";
const API_SECRET_ENV: &str = "APCA_API_SECRET_KEY";
const USER_AGENT: &str = "tdw-provider-alpaca/0.1";

#[derive(Deserialize)]
struct AlpacaEnvelope {
    #[serde(default)]
    bars: BTreeMap<String, Vec<AlpacaBar>>,
    #[serde(default)]
    code: Option<u64>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct AlpacaBar {
    #[serde(rename = "t")]
    timestamp: String,
    #[serde(rename = "o")]
    open: f64,
    #[serde(rename = "h")]
    high: f64,
    #[serde(rename = "l")]
    low: f64,
    #[serde(rename = "c")]
    close: f64,
    #[serde(rename = "v")]
    volume: f64,
}

/// Provider specification for the Alpaca stock-bars fetcher.
pub struct AlpacaStockBarsSpec;

impl ProviderSpec for AlpacaStockBarsSpec {
    const PROVIDER: &'static str = "alpaca";
    const ENDPOINT: &'static str = "stock_bars";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "alpaca client";
    const SEND_ERR: &'static str = "alpaca extract_data";
    const RETURNED_ERR: &'static str = "alpaca extract_data returned";
    const READ_BODY_ERR: &'static str = "alpaca read body";

    type Query = AlpacaStockBarsQuery;
    type Data = MarketDataBar;

    fn transform_query(params: Value) -> Result<AlpacaStockBarsQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("alpaca symbol must be a string".to_string()))?;
        let start = params
            .get("start")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("alpaca start must be YYYY-MM-DD".to_string()))?;
        let end = params
            .get("end")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("alpaca end must be YYYY-MM-DD".to_string()))?;
        let timeframe = params
            .get("timeframe")
            .and_then(Value::as_str)
            .unwrap_or("1Day");
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|error| Error::InvalidQuery(format!("alpaca limit too large: {error}")))?
            .unwrap_or(1_000);
        let feed = params.get("feed").and_then(Value::as_str);

        AlpacaStockBarsQuery::new(symbol, start, end)
            .and_then(|query| query.with_timeframe(timeframe))
            .and_then(|query| query.with_limit(limit))
            .and_then(|query| query.with_feed(feed))
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &AlpacaStockBarsQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        stock_bars_request(&query.symbol, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "alpaca")?;
        let api_secret =
            tdw_core::http_support::read_required_key(API_SECRET_ENV, "alpaca-secret")?;
        let endpoint = format!("{}/v2/stocks/bars", base_url.trim_end_matches('/'));
        let mut query_params = vec![
            ("symbols", query.symbol.clone()),
            ("timeframe", query.timeframe.clone()),
            ("start", query.start.clone()),
            ("end", query.end.clone()),
            ("limit", query.limit.to_string()),
        ];
        if let Some(feed) = &query.feed {
            query_params.push(("feed", feed.clone()));
        }

        Ok(client
            .get(&endpoint)
            .header(API_KEY_HEADER, api_key)
            .header(API_SECRET_HEADER, api_secret)
            .query(&query_params))
    }

    fn transform_data(query: &AlpacaStockBarsQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: AlpacaEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("alpaca parse_json: {error}")))?;
        if envelope.bars.is_empty() && (envelope.code.is_some() || envelope.message.is_some()) {
            return Err(Error::Provider(format!(
                "alpaca api error {}: {}",
                envelope.code.unwrap_or_default(),
                envelope.message.unwrap_or_default()
            )));
        }
        let bars = envelope
            .bars
            .get(&query.symbol)
            .ok_or_else(|| Error::Provider(format!("alpaca response missing {}", query.symbol)))?;
        let mut rows = Vec::with_capacity(bars.len());
        for bar in bars {
            rows.push(MarketDataBar {
                symbol: query.symbol.clone(),
                venue: "alpaca".to_string(),
                granularity: TimeGranularity::Day,
                ts: bar.timestamp.clone(),
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                source: "alpaca".to_string(),
            });
        }
        Ok(rows)
    }
}

/// Production Alpaca stock-bars fetcher.
pub type AlpacaHttpStockBarsFetcher = HttpFetcher<AlpacaStockBarsSpec>;
