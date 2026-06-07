//! Real Finnhub HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Two fetchers are provided:
//!   - [`FinnhubHttpProfileFetcher`] — company profile via `/stock/profile2`
//!   - [`FinnhubHttpQuoteSnapshotFetcher`] — last-price quote via `/quote`
//!
//! Live calls require `TDW_FINNHUB_API_KEY`. The live integration test is
//! additionally gated by `TDW_FINNHUB_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_domain::{CompanyProfile, QuoteSnapshot};

use crate::{API_KEY_ENV, BASE_URL, FinnhubError, FinnhubProfileQuery, FinnhubQuoteQuery};

const USER_AGENT: &str = "tdw-provider-finnhub/0.1";

// ---------------------------------------------------------------------------
// Internal serde shapes for the Finnhub API responses
// ---------------------------------------------------------------------------

/// Wire shape for `/stock/profile2` response.
#[derive(Deserialize)]
struct FinnhubProfileRaw {
    #[serde(default)]
    ticker: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    currency: String,
    #[serde(default)]
    exchange: String,
    #[serde(default)]
    logo: String,
    #[serde(rename = "marketCapitalization", default)]
    market_capitalization: f64,
}

/// Wire shape for `/quote` response.
#[derive(Deserialize)]
struct FinnhubQuoteRaw {
    /// Current price.
    #[serde(default)]
    c: f64,
    /// Change (absolute).
    #[serde(default)]
    d: f64,
    /// Change percent.
    #[serde(default)]
    dp: f64,
    /// Previous close.
    #[serde(default)]
    pc: f64,
    /// Unix timestamp in seconds.
    #[serde(default)]
    t: i64,
}

// ---------------------------------------------------------------------------
// Helper: build a reqwest Client and read the API key
// ---------------------------------------------------------------------------

fn finnhub_client() -> std::result::Result<Client, FinnhubError> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FinnhubError::Provider(format!("finnhub client build: {e}")))
}

fn api_key() -> std::result::Result<String, FinnhubError> {
    let key = std::env::var(API_KEY_ENV)
        .map_err(|_| FinnhubError::Provider(format!("{API_KEY_ENV} not set")))?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(FinnhubError::Provider(format!(
            "{API_KEY_ENV} must not be empty"
        )));
    }
    Ok(key)
}

// ---------------------------------------------------------------------------
// FinnhubHttpProfileFetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Finnhub company-profile fetcher.
    pub FinnhubHttpProfileFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FinnhubProfileQuery, CompanyProfile> for FinnhubHttpProfileFetcher {
    const PROVIDER: &'static str = "finnhub";
    const ENDPOINT: &'static str = "company_profile";

    fn transform_query(params: Value) -> Result<FinnhubProfileQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("finnhub symbol must be a string".to_string()))?;
        FinnhubProfileQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FinnhubProfileQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}/stock/profile2", self.base_url().trim_end_matches('/'),);
        let client = finnhub_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[
                ("symbol", query.symbol.as_str()),
                ("token", api_key.as_str()),
            ])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("finnhub profile extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "finnhub profile returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("finnhub profile read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &FinnhubProfileQuery,
        raw: Bytes,
    ) -> Result<Vec<CompanyProfile>> {
        let profile: FinnhubProfileRaw = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("finnhub profile parse_json: {e}")))?;
        // An empty ticker in the response means no data was found for the symbol.
        if profile.ticker.is_empty() && profile.name.is_empty() {
            return Ok(vec![]);
        }
        let ticker = if profile.ticker.is_empty() {
            query.symbol.clone()
        } else {
            profile.ticker
        };
        Ok(vec![CompanyProfile {
            ticker,
            name: profile.name,
            currency: profile.currency,
            exchange: profile.exchange,
            logo_url: profile.logo,
            market_cap_millions: profile.market_capitalization,
        }])
    }
}

// ---------------------------------------------------------------------------
// FinnhubHttpQuoteSnapshotFetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production Finnhub last-price quote-snapshot fetcher.
    pub FinnhubHttpQuoteSnapshotFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FinnhubQuoteQuery, QuoteSnapshot> for FinnhubHttpQuoteSnapshotFetcher {
    const PROVIDER: &'static str = "finnhub";
    const ENDPOINT: &'static str = "quote_snapshot";

    fn transform_query(params: Value) -> Result<FinnhubQuoteQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("finnhub symbol must be a string".to_string()))?;
        FinnhubQuoteQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &FinnhubQuoteQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
        let url = format!("{}/quote", self.base_url().trim_end_matches('/'));
        let client = finnhub_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[
                ("symbol", query.symbol.as_str()),
                ("token", api_key.as_str()),
            ])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("finnhub quote extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "finnhub quote returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("finnhub quote read body: {e}")))
    }

    fn transform_data(&self, query: &FinnhubQuoteQuery, raw: Bytes) -> Result<Vec<QuoteSnapshot>> {
        let quote: FinnhubQuoteRaw = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("finnhub quote parse_json: {e}")))?;
        Ok(vec![QuoteSnapshot {
            symbol: query.symbol.clone(),
            current_price: quote.c,
            change: quote.d,
            change_percent: quote.dp,
            prev_close: quote.pc,
            // Finnhub returns seconds; convert to milliseconds for the domain type.
            ts_ms: quote.t * 1_000,
        }])
    }
}
