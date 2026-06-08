#![cfg(feature = "http")]
//! Real AkShare historical OHLCV backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the AkShare community REST bridge
//! directly via `reqwest`. No API key is required. Live calls are additionally
//! gated by `TDW_AKSHARE_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{AkShareMarket, AkShareQuery, BASE_URL};

const USER_AGENT: &str = "tdw-provider-akshare/0.1";

// ---------------------------------------------------------------------------
// Public fetcher struct
// ---------------------------------------------------------------------------

/// Production AkShare historical-bar fetcher (no API key needed).
#[derive(Clone, Debug)]
pub struct AkShareHttpFetcher {
    base_url: String,
}

impl Default for AkShareHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl AkShareHttpFetcher {
    /// Override the AkShare base URL (useful for tests against a local mirror).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `akshare` provider name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

// ---------------------------------------------------------------------------
// Serde response shapes (Chinese field names from the AkShare API)
// ---------------------------------------------------------------------------

/// A single bar row returned by `/api/public/stock_zh_a_hist` or
/// `/api/public/stock_hk_hist`.
#[derive(Deserialize)]
struct AkShareBar {
    #[serde(rename = "日期")]
    date: String,
    #[serde(rename = "开盘")]
    open: f64,
    #[serde(rename = "收盘")]
    close: f64,
    #[serde(rename = "最高")]
    high: f64,
    #[serde(rename = "最低")]
    low: f64,
    #[serde(rename = "成交量")]
    volume: f64,
}

// ---------------------------------------------------------------------------
// Fetcher impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Fetcher<AkShareQuery, MarketDataBar> for AkShareHttpFetcher {
    const PROVIDER: &'static str = "akshare";
    const ENDPOINT: &'static str = "hist";

    fn transform_query(params: Value) -> Result<AkShareQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("akshare symbol must be a string".to_string()))?;
        let market_str = params
            .get("market")
            .and_then(Value::as_str)
            .unwrap_or("AShares");
        let market = match market_str {
            "HongKong" | "hk" | "HK" => AkShareMarket::HongKong,
            _ => AkShareMarket::AShares,
        };
        let start_date = params
            .get("start_date")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("akshare start_date must be YYYYMMDD".to_string())
            })?;
        let end_date = params
            .get("end_date")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("akshare end_date must be YYYYMMDD".to_string()))?;

        AkShareQuery::new(symbol, market, start_date, end_date)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &AkShareQuery, _creds: &Credentials) -> Result<Bytes> {
        let endpoint = format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            query.market.endpoint_path(),
        );
        let body = serde_json::json!({
            "symbol": query.symbol,
            "period": "daily",
            "start_date": query.start_date,
            "end_date": query.end_date,
            "adjust": ""
        });

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Provider(format!("akshare client build: {e}")))?;
        let response = client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("akshare extract_data: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "akshare extract_data returned {status}: {text}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("akshare read body: {e}")))
    }

    fn transform_data(&self, query: &AkShareQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let bars: Vec<AkShareBar> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("akshare parse_json: {e}")))?;

        let venue = query.market.venue().to_string();
        let mut rows = Vec::with_capacity(bars.len());
        for bar in bars {
            // AkShare returns dates as "YYYY-MM-DD"; append T00:00:00Z for the
            // canonical ts format used across the platform.
            let ts = format!("{}T00:00:00Z", bar.date);
            rows.push(MarketDataBar {
                symbol: query.symbol.clone(),
                venue: venue.clone(),
                granularity: TimeGranularity::Day,
                ts,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                source: "akshare".to_string(),
            });
        }
        Ok(rows)
    }
}
