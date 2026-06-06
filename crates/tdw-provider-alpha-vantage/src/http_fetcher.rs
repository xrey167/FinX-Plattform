#![cfg(feature = "http")]
//! Real Alpha Vantage HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the Alpha Vantage REST API via
//! `reqwest`. Live calls require `TDW_ALPHA_VANTAGE_API_KEY`; the live
//! integration test is additionally gated by `TDW_ALPHA_VANTAGE_LIVE=1` so
//! unattended CI stays offline.

use serde::Deserialize;
use std::collections::HashMap;
use tdw_core::http_support::prelude::*;
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{API_KEY_ENV, AlphaVantageError, AlphaVantageFunction, AlphaVantageQuery, BASE_URL};

const USER_AGENT: &str = "tdw-provider-alpha-vantage/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production Alpha Vantage HTTP fetcher.
    ///
    /// Supports both `TIME_SERIES_DAILY` and `GLOBAL_QUOTE` endpoints.
    /// Auth is read from the `TDW_ALPHA_VANTAGE_API_KEY` environment variable.
    ///
    /// # Rate limits
    ///
    /// The free-tier Alpha Vantage key allows **25 requests per day**. Use a
    /// paid key or cache responses locally for higher throughput.
    pub AlphaVantageHttpFetcher,
    BASE_URL
);

impl AlphaVantageHttpFetcher {
    fn read_api_key() -> Result<String> {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Provider(AlphaVantageError::MissingApiKey.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Serde shapes
// ---------------------------------------------------------------------------

/// One daily bar from the `Time Series (Daily)` object.
#[derive(Deserialize)]
struct AvDailyBar {
    #[serde(rename = "1. open")]
    open: String,
    #[serde(rename = "2. high")]
    high: String,
    #[serde(rename = "3. low")]
    low: String,
    #[serde(rename = "4. close")]
    close: String,
    #[serde(rename = "5. volume")]
    volume: String,
}

/// Top-level envelope for `TIME_SERIES_DAILY`.
#[derive(Deserialize)]
struct AvTimeSeriesDailyEnvelope {
    #[serde(rename = "Time Series (Daily)", default)]
    time_series: HashMap<String, AvDailyBar>,
    #[serde(rename = "Meta Data", default)]
    meta_data: HashMap<String, String>,
    /// Present when AV returns an API error (e.g. rate-limit exceeded).
    #[serde(rename = "Information", default)]
    information: Option<String>,
    /// Present on note/warning responses.
    #[serde(rename = "Note", default)]
    note: Option<String>,
}

/// Top-level envelope for `GLOBAL_QUOTE`.
#[derive(Deserialize)]
struct AvGlobalQuoteEnvelope {
    #[serde(rename = "Global Quote", default)]
    global_quote: HashMap<String, String>,
    #[serde(rename = "Information", default)]
    information: Option<String>,
    #[serde(rename = "Note", default)]
    note: Option<String>,
}

// ---------------------------------------------------------------------------
// Fetcher impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Fetcher<AlphaVantageQuery, MarketDataBar> for AlphaVantageHttpFetcher {
    const PROVIDER: &'static str = "alpha_vantage";
    const ENDPOINT: &'static str = "market_data";

    fn transform_query(params: Value) -> Result<AlphaVantageQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("alpha_vantage symbol must be a string".to_string())
            })?;
        let function_str = params
            .get("function")
            .and_then(Value::as_str)
            .unwrap_or("TIME_SERIES_DAILY");
        let function = match function_str.to_ascii_uppercase().as_str() {
            "TIME_SERIES_DAILY" => AlphaVantageFunction::TimeSeriesDaily,
            "GLOBAL_QUOTE" => AlphaVantageFunction::GlobalQuote,
            other => {
                return Err(Error::InvalidQuery(format!(
                    "alpha_vantage unknown function: {other}"
                )));
            }
        };
        AlphaVantageQuery::new(symbol, function).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &AlphaVantageQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = Self::read_api_key()?;
        let query_params = [
            ("function", query.function.as_api_str().to_string()),
            ("symbol", query.symbol.clone()),
            ("outputsize", "compact".to_string()),
            ("apikey", api_key),
        ];
        let client =
            tdw_core::http_support::build_client(USER_AGENT, "alpha_vantage client build")?;
        let response = client
            .get(self.base_url())
            .query(&query_params)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("alpha_vantage extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "alpha_vantage extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("alpha_vantage read body: {e}")))
    }

    fn transform_data(&self, query: &AlphaVantageQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        match query.function {
            AlphaVantageFunction::TimeSeriesDaily => {
                transform_time_series_daily(&query.symbol, raw)
            }
            AlphaVantageFunction::GlobalQuote => transform_global_quote(raw),
        }
    }
}

// ---------------------------------------------------------------------------
// Transform helpers
// ---------------------------------------------------------------------------

fn transform_time_series_daily(symbol: &str, raw: Bytes) -> Result<Vec<MarketDataBar>> {
    let envelope: AvTimeSeriesDailyEnvelope = serde_json::from_slice(&raw)
        .map_err(|e| Error::Provider(format!("alpha_vantage parse_json: {e}")))?;
    if let Some(info) = envelope.information.or(envelope.note) {
        return Err(Error::Provider(format!(
            "alpha_vantage api message: {info}"
        )));
    }
    let resolved_symbol = envelope
        .meta_data
        .get("2. Symbol")
        .cloned()
        .unwrap_or_else(|| symbol.to_string());
    let mut rows: Vec<MarketDataBar> = envelope
        .time_series
        .into_iter()
        .map(|(date, bar)| {
            let open = parse_f64(&bar.open, "open")?;
            let high = parse_f64(&bar.high, "high")?;
            let low = parse_f64(&bar.low, "low")?;
            let close = parse_f64(&bar.close, "close")?;
            let volume = parse_f64(&bar.volume, "volume")?;
            Ok(MarketDataBar {
                symbol: resolved_symbol.clone(),
                venue: "alpha_vantage".to_string(),
                granularity: TimeGranularity::Day,
                ts: format!("{date}T00:00:00Z"),
                open,
                high,
                low,
                close,
                volume,
                source: "alpha_vantage".to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    rows.sort_by(|a, b| a.ts.cmp(&b.ts));
    Ok(rows)
}

fn transform_global_quote(raw: Bytes) -> Result<Vec<MarketDataBar>> {
    let envelope: AvGlobalQuoteEnvelope = serde_json::from_slice(&raw)
        .map_err(|e| Error::Provider(format!("alpha_vantage parse_json: {e}")))?;
    if let Some(info) = envelope.information.or(envelope.note) {
        return Err(Error::Provider(format!(
            "alpha_vantage api message: {info}"
        )));
    }
    let quote = &envelope.global_quote;
    if quote.is_empty() {
        return Err(Error::Provider(
            "alpha_vantage global_quote returned empty object".to_string(),
        ));
    }
    let symbol = quote.get("01. symbol").cloned().unwrap_or_default();
    let price = quote
        .get("05. price")
        .map(String::as_str)
        .map(|s| parse_f64(s, "price"))
        .transpose()?
        .unwrap_or(0.0);
    let volume = quote
        .get("06. volume")
        .map(String::as_str)
        .map(|s| parse_f64(s, "volume"))
        .transpose()?
        .unwrap_or(0.0);
    Ok(vec![MarketDataBar {
        symbol,
        venue: "alpha_vantage".to_string(),
        granularity: TimeGranularity::Day,
        ts: String::new(),
        open: price,
        high: price,
        low: price,
        close: price,
        volume,
        source: "alpha_vantage".to_string(),
    }])
}

fn parse_f64(value: &str, field: &str) -> Result<f64> {
    value.trim().parse::<f64>().map_err(|e| {
        Error::Provider(format!(
            "alpha_vantage could not parse {field} as f64 ({value:?}): {e}"
        ))
    })
}
