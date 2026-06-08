#![cfg(feature = "http")]
//! Real CCData HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the CCData REST API via `reqwest`.
//! Live calls require `TDW_CCDATA_API_KEY`; the live integration test is
//! additionally gated by `TDW_CCDATA_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{API_KEY_ENV, BASE_URL, CCDataAssetQuery, CCDataError, CCDataOhlcvQuery};

const USER_AGENT: &str = "tdw-provider-ccdata/0.1";

/// Production CCData HTTP fetcher for daily OHLCV data.
///
/// Auth is read from the `TDW_CCDATA_API_KEY` environment variable and sent as
/// the `authorization: Apikey {key}` header.
#[derive(Clone, Debug)]
pub struct CCDataHttpFetcher {
    base_url: String,
}

impl Default for CCDataHttpFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl CCDataHttpFetcher {
    /// Override the CCData base URL (useful in tests).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `ccdata` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }

    fn read_api_key() -> Result<String> {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Provider(CCDataError::MissingApiKey.to_string()))
    }

    fn build_client() -> Result<Client> {
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("ccdata client build: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Serde shapes — OHLCV endpoint
// ---------------------------------------------------------------------------

/// One entry from the CCData daily OHLCV response.
#[derive(Deserialize)]
struct CcOhlcvEntry {
    #[serde(rename = "TIMESTAMP")]
    timestamp: i64,
    #[serde(rename = "OPEN")]
    open: f64,
    #[serde(rename = "HIGH")]
    high: f64,
    #[serde(rename = "LOW")]
    low: f64,
    #[serde(rename = "CLOSE")]
    close: f64,
    #[serde(rename = "VOLUME")]
    volume: f64,
}

/// Inner `Data` object of the OHLCV response.
#[derive(Deserialize)]
struct CcOhlcvData {
    #[serde(rename = "Entries", default)]
    entries: Vec<CcOhlcvEntry>,
    #[serde(rename = "Instrument", default)]
    instrument: String,
}

/// Top-level OHLCV response envelope.
#[derive(Deserialize)]
struct CcOhlcvEnvelope {
    #[serde(rename = "Data")]
    data: CcOhlcvData,
}

// ---------------------------------------------------------------------------
// Serde shapes — asset metadata endpoint
// ---------------------------------------------------------------------------

/// Inner `Data` object of the asset metadata response.
#[derive(Deserialize)]
struct CcAssetData {
    #[serde(rename = "ID")]
    id: u64,
    #[serde(rename = "SYMBOL")]
    symbol: String,
    #[serde(rename = "NAME")]
    name: String,
    #[serde(rename = "ASSET_TYPE")]
    asset_type: String,
    #[serde(rename = "LAUNCH_DATE", default)]
    launch_date: i64,
    #[serde(rename = "CIRCULATING_SUPPLY", default)]
    circulating_supply: f64,
    #[serde(rename = "MARKET_CAP_USD", default)]
    market_cap_usd: f64,
}

/// Top-level asset metadata response envelope.
#[derive(Deserialize)]
struct CcAssetEnvelope {
    #[serde(rename = "Data")]
    data: CcAssetData,
}

// ---------------------------------------------------------------------------
// Fetcher impl — OHLCV
// ---------------------------------------------------------------------------

#[async_trait]
impl Fetcher<CCDataOhlcvQuery, MarketDataBar> for CCDataHttpFetcher {
    const PROVIDER: &'static str = "ccdata";
    const ENDPOINT: &'static str = "crypto_ohlcv";

    fn transform_query(params: Value) -> Result<CCDataOhlcvQuery> {
        let market = params
            .get("market")
            .and_then(Value::as_str)
            .unwrap_or("ccix");
        let instrument = params
            .get("instrument")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ccdata instrument must be a string".to_string()))?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .try_into()
            .unwrap_or(30_u32);
        CCDataOhlcvQuery::new(market, instrument, limit)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &CCDataOhlcvQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = Self::read_api_key()?;
        let url = format!("{}/spot/v1/historical/days", self.base_url);
        let client = Self::build_client()?;
        let response = client
            .get(&url)
            .header("authorization", format!("Apikey {api_key}"))
            .query(&[
                ("market", query.market.as_str()),
                ("instrument", query.instrument.as_str()),
                ("limit", &query.limit.to_string()),
            ])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("ccdata extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "ccdata extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("ccdata read body: {e}")))
    }

    fn transform_data(&self, query: &CCDataOhlcvQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: CcOhlcvEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("ccdata parse_json (ohlcv): {e}")))?;
        let instrument = if envelope.data.instrument.is_empty() {
            query.instrument.clone()
        } else {
            envelope.data.instrument.clone()
        };
        let mut rows: Vec<MarketDataBar> = envelope
            .data
            .entries
            .into_iter()
            .map(|entry| {
                let ts = format_unix_ts(entry.timestamp);
                MarketDataBar {
                    symbol: instrument.clone(),
                    venue: "ccdata".to_string(),
                    granularity: TimeGranularity::Day,
                    ts,
                    open: entry.open,
                    high: entry.high,
                    low: entry.low,
                    close: entry.close,
                    volume: entry.volume,
                    source: "ccdata".to_string(),
                }
            })
            .collect();
        rows.sort_by(|a, b| a.ts.cmp(&b.ts));
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Asset metadata fetcher — separate zero-sized wrapper so it can register
// under a distinct endpoint name.
// ---------------------------------------------------------------------------

/// CCData HTTP fetcher for asset metadata (symbol → name, supply, market cap).
#[derive(Clone, Debug, Default)]
pub struct CCDataAssetHttpFetcher {
    base_url: String,
}

impl CCDataAssetHttpFetcher {
    /// Override the CCData base URL (useful in tests).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn effective_base_url(&self) -> &str {
        if self.base_url.is_empty() {
            BASE_URL
        } else {
            &self.base_url
        }
    }

    fn read_api_key() -> Result<String> {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Provider(CCDataError::MissingApiKey.to_string()))
    }

    fn build_client() -> Result<Client> {
        Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("ccdata client build: {e}")))
    }
}

/// Domain model for a single CCData asset metadata row.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CCDataAssetRow {
    pub id: u64,
    pub symbol: String,
    pub name: String,
    pub asset_type: String,
    pub launch_date: i64,
    pub circulating_supply: f64,
    pub market_cap_usd: f64,
}

#[async_trait]
impl Fetcher<CCDataAssetQuery, CCDataAssetRow> for CCDataAssetHttpFetcher {
    const PROVIDER: &'static str = "ccdata";
    const ENDPOINT: &'static str = "crypto_asset";

    fn transform_query(params: Value) -> Result<CCDataAssetQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("asset_symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("ccdata asset symbol must be a string".to_string())
            })?;
        CCDataAssetQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &CCDataAssetQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = Self::read_api_key()?;
        let url = format!("{}/asset/v1/data/by/symbol", self.effective_base_url());
        let client = Self::build_client()?;
        let response = client
            .get(&url)
            .header("authorization", format!("Apikey {api_key}"))
            .query(&[("asset_symbol", query.symbol.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("ccdata extract_data (asset): {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "ccdata extract_data (asset) returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("ccdata read body (asset): {e}")))
    }

    fn transform_data(&self, _query: &CCDataAssetQuery, raw: Bytes) -> Result<Vec<CCDataAssetRow>> {
        let envelope: CcAssetEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("ccdata parse_json (asset): {e}")))?;
        let d = envelope.data;
        Ok(vec![CCDataAssetRow {
            id: d.id,
            symbol: d.symbol,
            name: d.name,
            asset_type: d.asset_type,
            launch_date: d.launch_date,
            circulating_supply: d.circulating_supply,
            market_cap_usd: d.market_cap_usd,
        }])
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert a Unix timestamp (seconds) to an ISO-8601 UTC string.
fn format_unix_ts(ts: i64) -> String {
    // Manual conversion: avoid pulling in chrono just for this.
    // Days since Unix epoch → date.
    let seconds = ts.unsigned_abs();
    let days = seconds / 86_400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}-{m:02}-{d:02}T00:00:00Z")
}

/// Convert days-since-Unix-epoch to (year, month, day) via the proleptic
/// Gregorian calendar algorithm (Fliegel & Van Flandern, CACM 1968).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Shift epoch from 1970-01-01 to the Julian Day Number base used below.
    let jd = days + 2_440_588;
    let l = jd + 68_569;
    let n = 4 * l / 146_097;
    let l = l - (146_097 * n).div_ceil(4);
    let year_i = 4_000 * (l + 1) / 1_461_001;
    let l = l - 1_461 * year_i / 4 + 31;
    let month_i = 80 * l / 2_447;
    let day = l - 2_447 * month_i / 80;
    let l = month_i / 11;
    let month = month_i + 2 - 12 * l;
    let year = 100 * (n - 49) + year_i + l;
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_unix_ts_converts_known_epoch() {
        // 2024-01-01T00:00:00Z → 1704067200
        assert_eq!(format_unix_ts(1_704_067_200), "2024-01-01T00:00:00Z");
        // 1970-01-01T00:00:00Z → 0
        assert_eq!(format_unix_ts(0), "1970-01-01T00:00:00Z");
    }
}
