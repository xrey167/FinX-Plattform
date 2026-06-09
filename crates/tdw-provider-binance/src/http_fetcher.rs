//! Real Binance ticker-price backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Binance's public
//! `/api/v3/ticker/price` endpoint directly via `reqwest`. Live calls
//! are additionally gated by `TDW_BINANCE_LIVE=1` so unattended CI
//! stays offline.

#![cfg(feature = "http")]

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, BinanceTickerPrice, BinanceTickerPriceQuery, ticker_price_request};

const USER_AGENT: &str = "tdw-provider-binance/0.1";

#[derive(Deserialize)]
struct BinanceTickerPriceEnvelope {
    symbol: String,
    price: String,
}

#[derive(Deserialize)]
struct BinanceErrorEnvelope {
    code: i64,
    msg: String,
}

/// Provider specification for the Binance ticker-price fetcher.
pub struct BinanceTickerPriceSpec;

impl ProviderSpec for BinanceTickerPriceSpec {
    const PROVIDER: &'static str = "binance";
    const ENDPOINT: &'static str = "ticker_price";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "binance client";
    const SEND_ERR: &'static str = "binance extract_data";
    const RETURNED_ERR: &'static str = "binance extract_data returned";
    const READ_BODY_ERR: &'static str = "binance read body";

    type Query = BinanceTickerPriceQuery;
    type Data = BinanceTickerPrice;

    fn transform_query(params: Value) -> Result<BinanceTickerPriceQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("binance symbol must be a string".to_string()))?;
        BinanceTickerPriceQuery::new(symbol).map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &BinanceTickerPriceQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        ticker_price_request(&query.symbol).map_err(|error| Error::Provider(error.to_string()))?;
        let endpoint = format!("{}/api/v3/ticker/price", base_url.trim_end_matches('/'));
        let query_params = [("symbol", query.symbol.as_str())];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(
        query: &BinanceTickerPriceQuery,
        raw: Bytes,
    ) -> Result<Vec<BinanceTickerPrice>> {
        if let Ok(error) = serde_json::from_slice::<BinanceErrorEnvelope>(&raw) {
            return Err(Error::Provider(format!(
                "binance api error {}: {}",
                error.code, error.msg
            )));
        }
        let envelope: BinanceTickerPriceEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("binance parse_json: {error}")))?;
        let price = envelope.price.parse::<f64>().map_err(|error| {
            Error::Provider(format!(
                "binance price parse failed for {}: {error}",
                envelope.symbol
            ))
        })?;
        let symbol = if envelope.symbol.is_empty() {
            query.symbol.clone()
        } else {
            envelope.symbol
        };
        Ok(vec![BinanceTickerPrice { symbol, price }])
    }
}

/// Production Binance ticker-price fetcher.
pub type BinanceHttpTickerPriceFetcher = HttpFetcher<BinanceTickerPriceSpec>;
