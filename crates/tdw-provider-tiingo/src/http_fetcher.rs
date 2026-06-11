//! Real Tiingo HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Requires `TDW_TIINGO_API_KEY` at runtime.
//! Live integration tests are additionally gated by `TDW_TIINGO_LIVE=1`.

#![cfg(feature = "http")]

use bytes::Bytes;
use reqwest::Client;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_domain::{MarketDataBar, Ohlcv, TimeGranularity};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{API_KEY_ENV, BASE_URL, TiingoHistoricalQuery, TiingoNewsQuery};

const USER_AGENT: &str = "tdw-provider-tiingo/0.1";

// ---------------------------------------------------------------------------
// Response deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TiingoDailyPrice {
    date: String,
    #[serde(flatten)]
    ohlcv: Ohlcv,
}

#[derive(Deserialize)]
struct TiingoNewsItem {
    id: u64,
    title: String,
    #[serde(rename = "publishedDate")]
    published_date: String,
    url: String,
    source: String,
}

// ---------------------------------------------------------------------------
// Public output type for news
// ---------------------------------------------------------------------------

/// A single Tiingo news article returned by the news fetcher.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TiingoNewsArticle {
    pub id: u64,
    pub title: String,
    pub published_date: String,
    pub url: String,
    pub source: String,
}

// ---------------------------------------------------------------------------
// Historical price fetcher
// ---------------------------------------------------------------------------

/// Provider specification for the Tiingo daily historical price fetcher.
pub struct TiingoHistoricalSpec;

impl ProviderSpec for TiingoHistoricalSpec {
    const PROVIDER: &'static str = "tiingo";
    const ENDPOINT: &'static str = "historical";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "tiingo http client";
    const SEND_ERR: &'static str = "tiingo historical extract_data";
    const RETURNED_ERR: &'static str = "tiingo historical returned";
    const READ_BODY_ERR: &'static str = "tiingo historical read body";

    type Query = TiingoHistoricalQuery;
    type Data = MarketDataBar;

    fn transform_query(params: Value) -> Result<TiingoHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("tiingo symbol must be a non-empty string".to_string())
            })?;
        TiingoHistoricalQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &TiingoHistoricalQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = read_api_key()?;
        let endpoint = format!(
            "{}/daily/{}/prices",
            base_url.trim_end_matches('/'),
            query.symbol,
        );
        let query_params = [("startDate", "2024-01-01".to_string()), ("token", api_key)];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &TiingoHistoricalQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let prices: Vec<TiingoDailyPrice> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tiingo historical parse_json: {e}")))?;
        let mut rows = Vec::with_capacity(prices.len());
        for price in prices {
            rows.push(MarketDataBar {
                symbol: query.symbol.clone(),
                venue: "tiingo".to_string(),
                granularity: TimeGranularity::Day,
                ts: price.date,
                source: "tiingo".to_string(),
                ..price.ohlcv.into_bar_template()
            });
        }
        Ok(rows)
    }
}

/// Production Tiingo daily historical price fetcher.
pub type TiingoHttpHistoricalFetcher = HttpFetcher<TiingoHistoricalSpec>;

// ---------------------------------------------------------------------------
// News fetcher
// ---------------------------------------------------------------------------

/// Provider specification for the Tiingo news feed fetcher.
pub struct TiingoNewsSpec;

impl ProviderSpec for TiingoNewsSpec {
    const PROVIDER: &'static str = "tiingo";
    const ENDPOINT: &'static str = "news";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "tiingo http client";
    const SEND_ERR: &'static str = "tiingo news extract_data";
    const RETURNED_ERR: &'static str = "tiingo news returned";
    const READ_BODY_ERR: &'static str = "tiingo news read body";

    type Query = TiingoNewsQuery;
    type Data = TiingoNewsArticle;

    fn transform_query(params: Value) -> Result<TiingoNewsQuery> {
        let tickers_value = params.get("tickers").ok_or_else(|| {
            Error::InvalidQuery("tiingo news requires a 'tickers' field".to_string())
        })?;
        let tickers: Vec<String> = match tickers_value {
            Value::Array(arr) => arr
                .iter()
                .map(|v| {
                    v.as_str().map(str::to_string).ok_or_else(|| {
                        Error::InvalidQuery("each ticker must be a string".to_string())
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            Value::String(s) => vec![s.clone()],
            _ => {
                return Err(Error::InvalidQuery(
                    "tiingo news 'tickers' must be a string or array".to_string(),
                ));
            }
        };
        let refs: Vec<&str> = tickers.iter().map(String::as_str).collect();
        TiingoNewsQuery::new(&refs).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &TiingoNewsQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = read_api_key()?;
        let tickers_param = query.tickers.join(",").to_ascii_lowercase();
        let endpoint = format!("{}/news", base_url.trim_end_matches('/'));
        let query_params = [("tickers", tickers_param), ("token", api_key)];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(_query: &TiingoNewsQuery, raw: Bytes) -> Result<Vec<TiingoNewsArticle>> {
        let items: Vec<TiingoNewsItem> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("tiingo news parse_json: {e}")))?;
        Ok(items
            .into_iter()
            .map(|item| TiingoNewsArticle {
                id: item.id,
                title: item.title,
                published_date: item.published_date,
                url: item.url,
                source: item.source,
            })
            .collect())
    }
}

/// Production Tiingo news feed fetcher.
pub type TiingoHttpNewsFetcher = HttpFetcher<TiingoNewsSpec>;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, "tiingo")
}
