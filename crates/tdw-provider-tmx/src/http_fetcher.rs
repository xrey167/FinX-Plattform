#![cfg(feature = "http")]
//! Real TMX Money HTTP backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature so the workspace test set stays offline by
//! default. The live integration tests are additionally gated by the env
//! var `TDW_TMX_LIVE=1` so unattended CI runs do not hit TMX Money by
//! accident.
//!
//! Two fetchers are provided:
//!
//! * [`TmxHttpQuoteFetcher`] — single-symbol quote (`equity_quote`).
//! * [`TmxHttpBatchQuoteFetcher`] — multi-symbol quote
//!   (`equity_batch_quote`).
//!
//! Both share the same base URL and `User-Agent` header. Use
//! `with_base_url` to point them at a local cassette server during
//! integration tests.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;

use crate::{TmxBatchQuoteQuery, TmxQuote, TmxQuoteQuery, parse_quote_response, validate_symbol};

const BASE_URL: &str = "https://app.tmxmoney.com/apphub/quotes/api";
const USER_AGENT: &str = "tdw-provider-tmx/0.1";

// ── helpers ────────────────────────────────────────────────────────────────

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|error| Error::Provider(format!("tmx client build: {error}")))
}

async fn get_bytes(client: &Client, url: &str) -> Result<Bytes> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("tmx request: {error}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!("tmx returned {status}: {body}")));
    }

    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("tmx read body: {error}")))
}

fn quotes_to_domain(quotes: Vec<TmxQuote>) -> Vec<EquityHistoricalData> {
    quotes
        .into_iter()
        .map(|q| EquityHistoricalData {
            symbol: q.symbol,
            date: String::new(), // TMX quote endpoint is real-time, no bar date
            open: q.open,
            high: q.high,
            low: q.low,
            close: q.close,
            volume: q.volume,
        })
        .collect()
}

// ── single-symbol fetcher ──────────────────────────────────────────────────

/// Production TMX single-symbol equity quote fetcher.
///
/// Maps to the `/quotes/getquote?listId=TSX&type=EQUITY&symbol={sym}`
/// endpoint. Output rows carry the domain type
/// [`tdw_domain::EquityHistoricalData`] so the fetcher integrates with
/// the existing `tdw-core` registry without introducing a new domain
/// model.
#[derive(Clone, Debug)]
pub struct TmxHttpQuoteFetcher {
    base_url: String,
}

impl Default for TmxHttpQuoteFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TmxHttpQuoteFetcher {
    /// Override the TMX base URL. Useful for pointing the fetcher at a
    /// cassette server during integration tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the `tmx` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TmxQuoteQuery, EquityHistoricalData> for TmxHttpQuoteFetcher {
    const PROVIDER: &'static str = "tmx";
    const ENDPOINT: &'static str = "equity_quote";

    fn transform_query(params: Value) -> Result<TmxQuoteQuery> {
        TmxQuoteQuery::from_params(&params).map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(&self, query: &TmxQuoteQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/quotes/getquote?listId=TSX&type=EQUITY&symbol={}",
            self.base_url, query.symbol
        );
        let client = build_client()?;
        get_bytes(&client, &url).await
    }

    fn transform_data(
        &self,
        _query: &TmxQuoteQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        let quotes =
            parse_quote_response(&raw).map_err(|error| Error::Provider(error.to_string()))?;
        Ok(quotes_to_domain(quotes))
    }
}

// ── batch fetcher ──────────────────────────────────────────────────────────

/// Production TMX batch equity quote fetcher (up to 10 symbols).
///
/// Maps to the
/// `/quotes/getsquote?listId=TSX&type=EQUITY&symbols={s1},{s2},...`
/// endpoint.
#[derive(Clone, Debug)]
pub struct TmxHttpBatchQuoteFetcher {
    base_url: String,
}

impl Default for TmxHttpBatchQuoteFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl TmxHttpBatchQuoteFetcher {
    /// Override the TMX base URL. Useful for pointing the fetcher at a
    /// cassette server during integration tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the `tmx` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TmxBatchQuoteQuery, EquityHistoricalData> for TmxHttpBatchQuoteFetcher {
    const PROVIDER: &'static str = "tmx";
    const ENDPOINT: &'static str = "equity_batch_quote";

    fn transform_query(params: Value) -> Result<TmxBatchQuoteQuery> {
        TmxBatchQuoteQuery::from_params(&params)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &TmxBatchQuoteQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let symbols = query.symbols.join(",");
        let url = format!(
            "{}/quotes/getsquote?listId=TSX&type=EQUITY&symbols={}",
            self.base_url, symbols
        );
        let client = build_client()?;
        get_bytes(&client, &url).await
    }

    fn transform_data(
        &self,
        _query: &TmxBatchQuoteQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        let quotes =
            parse_quote_response(&raw).map_err(|error| Error::Provider(error.to_string()))?;
        Ok(quotes_to_domain(quotes))
    }
}

// ── helpers exported for tests ─────────────────────────────────────────────

/// Build a cassette [`Bytes`] payload from a slice of raw quote maps.
///
/// Each entry is a `serde_json::Value` object matching the TMX
/// `getquote` envelope `results[]` shape. Used by the cassette
/// integration tests to assemble synthetic responses without touching
/// the network.
#[must_use]
pub fn cassette_bytes(results: serde_json::Value) -> Bytes {
    Bytes::from(
        serde_json::json!({ "results": results })
            .to_string()
            .into_bytes(),
    )
}

/// Validate a symbol using the public helper — re-exported for tests.
pub fn validated(raw: &str) -> Result<String> {
    validate_symbol(raw).map_err(|error| Error::InvalidQuery(error.to_string()))
}

// ── extension trait for test ergonomics ────────────────────────────────────

#[allow(dead_code)]
trait ExpectOk<T> {
    fn expect_err_is_none(self, msg: &str) -> T;
}

impl<T, E: std::fmt::Debug> ExpectOk<T> for std::result::Result<T, E> {
    fn expect_err_is_none(self, msg: &str) -> T {
        self.unwrap_or_else(|error| panic!("{msg}: {error:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_entries_have_correct_provider_and_endpoints() {
        let single = TmxHttpQuoteFetcher::registry_entry();
        assert_eq!(single.provider, "tmx");
        assert_eq!(single.endpoint, "equity_quote");

        let batch = TmxHttpBatchQuoteFetcher::registry_entry();
        assert_eq!(batch.provider, "tmx");
        assert_eq!(batch.endpoint, "equity_batch_quote");
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact literal round-trips
    fn single_fetcher_transform_data_decodes_cassette() {
        let fetcher = TmxHttpQuoteFetcher::default();
        let query = TmxHttpQuoteFetcher::transform_query(json!({ "symbol": "TD" }))
            .expect_err_is_none("query should parse");

        let raw = cassette_bytes(json!([{
            "symbol": "TD",
            "exchange": "TSX",
            "lastPrice": 79.50,
            "change": 0.25,
            "changePercent": 0.32,
            "volume": 3_500_000u64,
            "marketCap": 145_200_000_000.0f64,
            "open": 79.25,
            "high": 79.85,
            "low": 79.10,
            "close": 79.50
        }]));

        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "TD");
        assert_eq!(rows[0].close, 79.50);
        assert_eq!(rows[0].volume, 3_500_000);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn batch_fetcher_transform_data_decodes_multiple_rows() {
        let fetcher = TmxHttpBatchQuoteFetcher::default();
        let query = TmxHttpBatchQuoteFetcher::transform_query(json!({ "symbols": ["TD", "RY"] }))
            .expect_err_is_none("query should parse");

        let raw = cassette_bytes(json!([
            {
                "symbol": "TD", "exchange": "TSX",
                "lastPrice": 79.50, "change": 0.25, "changePercent": 0.32,
                "volume": 3_500_000u64, "marketCap": null,
                "open": 79.25, "high": 79.85, "low": 79.10, "close": 79.50
            },
            {
                "symbol": "RY", "exchange": "TSX",
                "lastPrice": 130.0, "change": -0.50, "changePercent": -0.38,
                "volume": 2_100_000u64, "marketCap": null,
                "open": 130.5, "high": 131.0, "low": 129.5, "close": 130.0
            }
        ]));

        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].symbol, "RY");
        assert_eq!(rows[1].close, 130.0);
    }

    #[test]
    fn single_fetcher_transform_query_validates_symbol() {
        assert!(TmxHttpQuoteFetcher::transform_query(json!({ "symbol": "TD" })).is_ok());
        assert!(TmxHttpQuoteFetcher::transform_query(json!({ "symbol": "TD?" })).is_err());
        assert!(TmxHttpQuoteFetcher::transform_query(json!({})).is_err());
    }

    #[test]
    fn batch_fetcher_transform_query_validates_symbols() {
        assert!(
            TmxHttpBatchQuoteFetcher::transform_query(json!({ "symbols": ["TD", "RY"] })).is_ok()
        );
        assert!(
            TmxHttpBatchQuoteFetcher::transform_query(
                json!({ "symbols": ["A","B","C","D","E","F","G","H","I","J","K"] })
            )
            .is_err()
        );
    }
}
