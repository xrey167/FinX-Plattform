#![cfg(feature = "http")]
//! Real Trading Economics HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Two fetchers are provided:
//!   - [`TradingEconomicsHttpCalendarFetcher`] — economic calendar via
//!     `GET /calendar?c={key}&i=all`
//!   - [`TradingEconomicsHttpIndicatorFetcher`] — country indicator via
//!     `GET /indicators/{country}/{indicator}?c={key}`
//!
//! Live calls require `TDW_TRADING_ECONOMICS_API_KEY`. The live integration
//! test is additionally gated by `TDW_TRADING_ECONOMICS_LIVE=1` so unattended
//! CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    API_KEY_ENV, BASE_URL, TradingEconomicsCalendarEvent, TradingEconomicsCalendarQuery,
    TradingEconomicsError, TradingEconomicsIndicatorQuery, TradingEconomicsIndicatorRow,
};

const USER_AGENT: &str = "tdw-provider-trading-economics/0.1";

// ---------------------------------------------------------------------------
// Internal serde shapes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawCalendarEvent {
    #[serde(rename = "Date", default)]
    date: String,
    #[serde(rename = "Country", default)]
    country: String,
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "Event", default)]
    event: String,
    #[serde(rename = "Actual", default)]
    actual: String,
    #[serde(rename = "Previous", default)]
    previous: String,
    #[serde(rename = "Forecast", default)]
    forecast: String,
    #[serde(rename = "TEForecast", default)]
    te_forecast: String,
    #[serde(rename = "Importance", default)]
    importance: u8,
}

#[derive(Deserialize)]
struct RawIndicatorRow {
    #[serde(rename = "Country", default)]
    country: String,
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "DateTime", default)]
    date_time: String,
    #[serde(rename = "Value", default)]
    value: String,
    #[serde(rename = "Frequency", default)]
    frequency: String,
    #[serde(rename = "HistoricalDataSymbol", default)]
    historical_data_symbol: String,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn te_client() -> std::result::Result<Client, TradingEconomicsError> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| TradingEconomicsError::Provider(format!("te client build: {e}")))
}

fn api_key() -> std::result::Result<String, TradingEconomicsError> {
    let key = std::env::var(API_KEY_ENV)
        .map_err(|_| TradingEconomicsError::Provider(format!("{API_KEY_ENV} not set")))?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(TradingEconomicsError::Provider(format!(
            "{API_KEY_ENV} must not be empty"
        )));
    }
    Ok(key)
}

// ---------------------------------------------------------------------------
// TradingEconomicsHttpCalendarFetcher
// ---------------------------------------------------------------------------

/// Production Trading Economics economic-calendar fetcher.
#[derive(Clone, Debug)]
pub struct TradingEconomicsHttpCalendarFetcher {
    base_url: String,
}

impl Default for TradingEconomicsHttpCalendarFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TradingEconomicsHttpCalendarFetcher {
    /// Override the base URL (useful for tests with a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `trading_economics` / `calendar` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TradingEconomicsCalendarQuery, TradingEconomicsCalendarEvent>
    for TradingEconomicsHttpCalendarFetcher
{
    const PROVIDER: &'static str = "trading_economics";
    const ENDPOINT: &'static str = "calendar";

    fn transform_query(params: Value) -> Result<TradingEconomicsCalendarQuery> {
        let importance_min = params
            .get("importance_min")
            .and_then(Value::as_u64)
            .map(|v| u8::try_from(v).unwrap_or(u8::MAX))
            .unwrap_or(1);
        TradingEconomicsCalendarQuery::new(importance_min)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        _query: &TradingEconomicsCalendarQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}/calendar", self.base_url.trim_end_matches('/'));
        let client = te_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("c", key.as_str()), ("i", "all")])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("te calendar extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "te calendar returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("te calendar read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &TradingEconomicsCalendarQuery,
        raw: Bytes,
    ) -> Result<Vec<TradingEconomicsCalendarEvent>> {
        let events: Vec<RawCalendarEvent> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("te calendar parse_json: {e}")))?;
        let rows = events
            .into_iter()
            .filter(|ev| ev.importance >= query.importance_min)
            .map(|ev| TradingEconomicsCalendarEvent {
                date: ev.date,
                country: ev.country,
                category: ev.category,
                event: ev.event,
                actual: ev.actual,
                previous: ev.previous,
                forecast: ev.forecast,
                te_forecast: ev.te_forecast,
                importance: ev.importance,
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// TradingEconomicsHttpIndicatorFetcher
// ---------------------------------------------------------------------------

/// Production Trading Economics country-indicator fetcher.
#[derive(Clone, Debug)]
pub struct TradingEconomicsHttpIndicatorFetcher {
    base_url: String,
}

impl Default for TradingEconomicsHttpIndicatorFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TradingEconomicsHttpIndicatorFetcher {
    /// Override the base URL (useful for tests with a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `trading_economics` / `indicator` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TradingEconomicsIndicatorQuery, TradingEconomicsIndicatorRow>
    for TradingEconomicsHttpIndicatorFetcher
{
    const PROVIDER: &'static str = "trading_economics";
    const ENDPOINT: &'static str = "indicator";

    fn transform_query(params: Value) -> Result<TradingEconomicsIndicatorQuery> {
        let country = params
            .get("country")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("te country must be a string".to_string()))?;
        let indicator = params
            .get("indicator")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("te indicator must be a string".to_string()))?;
        TradingEconomicsIndicatorQuery::new(country, indicator)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &TradingEconomicsIndicatorQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!(
            "{}/indicators/{}/{}",
            self.base_url.trim_end_matches('/'),
            query.country,
            query.indicator,
        );
        let client = te_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("c", key.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("te indicator extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "te indicator returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("te indicator read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &TradingEconomicsIndicatorQuery,
        raw: Bytes,
    ) -> Result<Vec<TradingEconomicsIndicatorRow>> {
        let rows: Vec<RawIndicatorRow> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("te indicator parse_json: {e}")))?;
        Ok(rows
            .into_iter()
            .map(|r| TradingEconomicsIndicatorRow {
                country: r.country,
                category: r.category,
                date_time: r.date_time,
                value: r.value,
                frequency: r.frequency,
                historical_data_symbol: r.historical_data_symbol,
            })
            .collect())
    }
}
