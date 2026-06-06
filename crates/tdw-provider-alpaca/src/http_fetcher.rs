//! Real Alpaca stock-bars backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Alpaca's historical stock
//! bars endpoint directly via `reqwest`. Live calls require
//! `APCA_API_KEY_ID` and `APCA_API_SECRET_KEY`; the live integration
//! test is additionally gated by `TDW_ALPACA_LIVE=1` so unattended CI
//! stays offline.

use std::collections::BTreeMap;

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{
    API_KEY_HEADER, API_SECRET_HEADER, AlpacaStockBarsQuery, BASE_URL, stock_bars_request,
};

const API_KEY_ENV: &str = "APCA_API_KEY_ID";
const API_SECRET_ENV: &str = "APCA_API_SECRET_KEY";
const USER_AGENT: &str = "tdw-provider-alpaca/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production Alpaca stock-bars fetcher.
    pub AlpacaHttpStockBarsFetcher,
    BASE_URL
);

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

#[async_trait]
impl Fetcher<AlpacaStockBarsQuery, MarketDataBar> for AlpacaHttpStockBarsFetcher {
    const PROVIDER: &'static str = "alpaca";
    const ENDPOINT: &'static str = "stock_bars";

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

    async fn extract_data(
        &self,
        query: &AlpacaStockBarsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        stock_bars_request(&query.symbol, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = env_secret(API_KEY_ENV).ok_or_else(|| {
            Error::Provider(format!("alpaca api key env {API_KEY_ENV} must be set"))
        })?;
        let api_secret = env_secret(API_SECRET_ENV).ok_or_else(|| {
            Error::Provider(format!(
                "alpaca api secret env {API_SECRET_ENV} must be set"
            ))
        })?;
        let endpoint = format!("{}/v2/stocks/bars", self.base_url().trim_end_matches('/'));
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

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("alpaca client: {error}")))?;
        let response = client
            .get(&endpoint)
            .header(API_KEY_HEADER, api_key)
            .header(API_SECRET_HEADER, api_secret)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("alpaca extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "alpaca extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("alpaca read body: {error}")))
    }

    fn transform_data(
        &self,
        query: &AlpacaStockBarsQuery,
        raw: Bytes,
    ) -> Result<Vec<MarketDataBar>> {
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

fn env_secret(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
