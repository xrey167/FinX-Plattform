//! Real Binance ticker-price backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Binance's public
//! `/api/v3/ticker/price` endpoint directly via `reqwest`. Live calls
//! are additionally gated by `TDW_BINANCE_LIVE=1` so unattended CI
//! stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;

use crate::{BASE_URL, BinanceTickerPrice, BinanceTickerPriceQuery, ticker_price_request};

const USER_AGENT: &str = "tdw-provider-binance/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production Binance ticker-price fetcher.
    pub BinanceHttpTickerPriceFetcher,
    BASE_URL
);

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

#[async_trait]
impl Fetcher<BinanceTickerPriceQuery, BinanceTickerPrice> for BinanceHttpTickerPriceFetcher {
    const PROVIDER: &'static str = "binance";
    const ENDPOINT: &'static str = "ticker_price";

    fn transform_query(params: Value) -> Result<BinanceTickerPriceQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("binance symbol must be a string".to_string()))?;
        BinanceTickerPriceQuery::new(symbol).map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &BinanceTickerPriceQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        ticker_price_request(&query.symbol).map_err(|error| Error::Provider(error.to_string()))?;
        let endpoint = format!(
            "{}/api/v3/ticker/price",
            self.base_url().trim_end_matches('/')
        );
        let query_params = [("symbol", query.symbol.as_str())];

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("binance client: {error}")))?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("binance extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "binance extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("binance read body: {error}")))
    }

    fn transform_data(
        &self,
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
