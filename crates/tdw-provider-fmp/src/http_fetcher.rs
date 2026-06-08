//! Real FMP (Financial Modeling Prep) HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Two fetchers are provided:
//!   - [`FmpHttpHistoricalFetcher`] — daily OHLCV bars via `/historical-price-full/{symbol}`
//!   - [`FmpHttpIncomeFetcher`] — income statements via `/income-statement/{symbol}`
//!
//! Live calls require `TDW_FMP_API_KEY`. The live integration test is
//! additionally gated by `TDW_FMP_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{
    API_KEY_ENV, BASE_URL, FmpError, FmpFundamentalsQuery, FmpHistoricalQuery, FmpIncomeRow,
    FmpStatement,
};

const USER_AGENT: &str = "tdw-provider-fmp/0.1";

// ---------------------------------------------------------------------------
// Internal serde shapes for the FMP API responses
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FmpHistoricalEnvelope {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    historical: Vec<FmpHistoricalBar>,
}

#[derive(Deserialize)]
struct FmpHistoricalBar {
    date: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

#[derive(Deserialize)]
struct FmpIncomeStatementRaw {
    #[serde(default)]
    date: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    revenue: i64,
    #[serde(rename = "grossProfit", default)]
    gross_profit: i64,
    #[serde(rename = "netIncome", default)]
    net_income: i64,
}

// ---------------------------------------------------------------------------
// Helper: build a reqwest Client and read the API key
// ---------------------------------------------------------------------------

fn fmp_client() -> std::result::Result<Client, FmpError> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FmpError::Provider(format!("fmp client build: {e}")))
}

fn api_key() -> std::result::Result<String, FmpError> {
    let key = std::env::var(API_KEY_ENV)
        .map_err(|_| FmpError::Provider(format!("{API_KEY_ENV} not set")))?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(FmpError::Provider(format!(
            "{API_KEY_ENV} must not be empty"
        )));
    }
    Ok(key)
}

// ---------------------------------------------------------------------------
// FmpHttpHistoricalFetcher
// ---------------------------------------------------------------------------

/// Production FMP daily-bar fetcher.
#[derive(Clone, Debug)]
pub struct FmpHttpHistoricalFetcher {
    base_url: String,
}

impl Default for FmpHttpHistoricalFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl FmpHttpHistoricalFetcher {
    /// Override the FMP base URL (useful for tests with a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `fmp` / `equity_historical` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<FmpHistoricalQuery, MarketDataBar> for FmpHttpHistoricalFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<FmpHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))?;
        FmpHistoricalQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!(
            "{}/historical-price-full/{}",
            self.base_url.trim_end_matches('/'),
            query.symbol,
        );
        let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("apikey", api_key.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("fmp extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "fmp historical returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("fmp read body: {e}")))
    }

    fn transform_data(&self, query: &FmpHistoricalQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: FmpHistoricalEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp parse_json: {e}")))?;
        let symbol = envelope.symbol.unwrap_or_else(|| query.symbol.clone());
        let mut rows = Vec::with_capacity(envelope.historical.len());
        for bar in envelope.historical {
            rows.push(MarketDataBar {
                symbol: symbol.clone(),
                venue: "fmp".to_string(),
                granularity: TimeGranularity::Day,
                ts: bar.date,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                source: "fmp".to_string(),
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpIncomeFetcher
// ---------------------------------------------------------------------------

/// Production FMP income-statement fetcher.
#[derive(Clone, Debug)]
pub struct FmpHttpIncomeFetcher {
    base_url: String,
}

impl Default for FmpHttpIncomeFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl FmpHttpIncomeFetcher {
    /// Override the FMP base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the canonical `fmp` / `income_statement` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<FmpFundamentalsQuery, FmpIncomeRow> for FmpHttpIncomeFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "income_statement";

    fn transform_query(params: Value) -> Result<FmpFundamentalsQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))?;
        let statement_str = params
            .get("statement")
            .and_then(Value::as_str)
            .unwrap_or("income");
        let statement = match statement_str {
            "income" => FmpStatement::Income,
            "balance" => FmpStatement::Balance,
            "cashflow" => FmpStatement::Cashflow,
            other => {
                return Err(Error::InvalidQuery(format!(
                    "fmp unknown statement type: {other}"
                )));
            }
        };
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|e| Error::InvalidQuery(format!("fmp limit too large: {e}")))?
            .unwrap_or(5);
        FmpFundamentalsQuery::new(symbol, statement, limit)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpFundamentalsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let path_segment = query.statement.as_path_segment();
        let url = format!(
            "{}/{}/{}",
            self.base_url.trim_end_matches('/'),
            path_segment,
            query.symbol,
        );
        let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("limit", query.limit.to_string()), ("apikey", api_key)])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("fmp income extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "fmp income-statement returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("fmp income read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &FmpFundamentalsQuery,
        raw: Bytes,
    ) -> Result<Vec<FmpIncomeRow>> {
        let statements: Vec<FmpIncomeStatementRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp income parse_json: {e}")))?;
        let rows = statements
            .into_iter()
            .map(|s| FmpIncomeRow {
                symbol: s.symbol.unwrap_or_else(|| query.symbol.clone()),
                date: s.date,
                revenue: s.revenue,
                gross_profit: s.gross_profit,
                net_income: s.net_income,
            })
            .collect();
        Ok(rows)
    }
}
