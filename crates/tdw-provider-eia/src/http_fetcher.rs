#![cfg(feature = "http")]
//! Real EIA HTTP fetchers for spot-price and natural-gas endpoints.
//!
//! Both fetchers are gated by the `http` Cargo feature. Live calls
//! require `TDW_EIA_API_KEY`; the live integration tests are additionally
//! gated by `TDW_EIA_LIVE=1` so unattended CI stays offline.

use reqwest::Client;
use serde::Deserialize;

use crate::{
    API_KEY_ENV, BASE_URL, EiaNaturalGasQuery, EiaNaturalGasRecord, EiaProviderError,
    EiaSpotPriceQuery, EiaSpotPriceRecord, Result,
};

const USER_AGENT: &str = "tdw-provider-eia/0.1";

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(EiaProviderError::MissingApiKey)
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| EiaProviderError::Provider(format!("eia build client: {e}")))
}

// ---------------------------------------------------------------------------
// Serde shapes for /petroleum/pri/spt/data/
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EiaSpotPriceEnvelope {
    response: EiaSpotPriceResponse,
}

#[derive(Deserialize)]
struct EiaSpotPriceResponse {
    #[serde(default)]
    data: Vec<EiaSpotPriceRaw>,
}

#[derive(Deserialize)]
struct EiaSpotPriceRaw {
    #[serde(default)]
    period: String,
    #[serde(rename = "product-name", default)]
    product_name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    units: String,
}

// ---------------------------------------------------------------------------
// Serde shapes for /natural-gas/pri/sum/data/
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EiaNaturalGasEnvelope {
    response: EiaNaturalGasResponse,
}

#[derive(Deserialize)]
struct EiaNaturalGasResponse {
    #[serde(default)]
    data: Vec<EiaNaturalGasRaw>,
}

#[derive(Deserialize)]
struct EiaNaturalGasRaw {
    #[serde(default)]
    period: String,
    #[serde(rename = "series-description", default)]
    series_description: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    units: String,
}

// ---------------------------------------------------------------------------
// EiaHttpSpotPriceFetcher
// ---------------------------------------------------------------------------

/// Production EIA petroleum spot-price fetcher.
///
/// Calls `GET /petroleum/pri/spt/data/` with daily frequency.
#[derive(Clone, Debug)]
pub struct EiaHttpSpotPriceFetcher {
    base_url: String,
}

impl Default for EiaHttpSpotPriceFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl EiaHttpSpotPriceFetcher {
    /// Override the EIA base URL (useful for tests / staging).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch and decode spot-price records for the given query.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError`] on missing API key, HTTP failure, or
    /// parse failure.
    pub async fn fetch(&self, query: &EiaSpotPriceQuery) -> Result<Vec<EiaSpotPriceRecord>> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!(
            "{}/petroleum/pri/spt/data/",
            self.base_url.trim_end_matches('/')
        );
        let params = [
            ("api_key", api_key),
            ("frequency", "daily".to_string()),
            ("data[0]", "value".to_string()),
            ("sort[0][column]", "period".to_string()),
            ("sort[0][direction]", "desc".to_string()),
            ("length", query.length.to_string()),
        ];
        let response = client
            .get(&endpoint)
            .query(&params)
            .send()
            .await
            .map_err(|e| EiaProviderError::Provider(format!("eia spot-price request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(EiaProviderError::Provider(format!(
                "eia spot-price returned {status}: {body}"
            )));
        }
        let raw_bytes = response
            .bytes()
            .await
            .map_err(|e| EiaProviderError::Provider(format!("eia spot-price read body: {e}")))?;
        parse_spot_price_bytes(&raw_bytes)
    }

    /// Decode a raw JSON byte slice into spot-price records.
    ///
    /// Exposed separately so cassette tests can call it without HTTP.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError::Provider`] on JSON parse failures or
    /// non-numeric value strings.
    pub fn parse_bytes(&self, raw: &[u8]) -> Result<Vec<EiaSpotPriceRecord>> {
        parse_spot_price_bytes(raw)
    }
}

fn parse_spot_price_bytes(raw: &[u8]) -> Result<Vec<EiaSpotPriceRecord>> {
    let envelope: EiaSpotPriceEnvelope = serde_json::from_slice(raw)
        .map_err(|e| EiaProviderError::Provider(format!("eia spot-price parse_json: {e}")))?;
    let mut records = Vec::with_capacity(envelope.response.data.len());
    for row in envelope.response.data {
        if row.period.is_empty() {
            return Err(EiaProviderError::Provider(
                "eia spot-price row missing period".to_string(),
            ));
        }
        let raw_value = row.value.trim();
        if raw_value.is_empty() || raw_value == "." {
            continue;
        }
        let value = raw_value.parse::<f64>().map_err(|e| {
            EiaProviderError::Provider(format!(
                "eia spot-price value parse failed for {}: {e}",
                row.period
            ))
        })?;
        records.push(EiaSpotPriceRecord {
            period: row.period,
            product_name: row.product_name,
            value,
            units: row.units,
        });
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// EiaHttpNaturalGasFetcher
// ---------------------------------------------------------------------------

/// Production EIA natural-gas price fetcher.
///
/// Calls `GET /natural-gas/pri/sum/data/` with monthly frequency.
#[derive(Clone, Debug)]
pub struct EiaHttpNaturalGasFetcher {
    base_url: String,
}

impl Default for EiaHttpNaturalGasFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl EiaHttpNaturalGasFetcher {
    /// Override the EIA base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Fetch and decode natural-gas price records.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError`] on missing API key, HTTP failure, or
    /// parse failure.
    pub async fn fetch(&self, query: &EiaNaturalGasQuery) -> Result<Vec<EiaNaturalGasRecord>> {
        let api_key = read_api_key()?;
        let client = build_client()?;
        let endpoint = format!(
            "{}/natural-gas/pri/sum/data/",
            self.base_url.trim_end_matches('/')
        );
        let params = [
            ("api_key", api_key),
            ("frequency", "monthly".to_string()),
            ("data[0]", "value".to_string()),
            ("length", query.length.to_string()),
        ];
        let response = client
            .get(&endpoint)
            .query(&params)
            .send()
            .await
            .map_err(|e| EiaProviderError::Provider(format!("eia natural-gas request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(EiaProviderError::Provider(format!(
                "eia natural-gas returned {status}: {body}"
            )));
        }
        let raw_bytes = response
            .bytes()
            .await
            .map_err(|e| EiaProviderError::Provider(format!("eia natural-gas read body: {e}")))?;
        parse_natural_gas_bytes(&raw_bytes)
    }

    /// Decode a raw JSON byte slice into natural-gas records.
    ///
    /// Exposed separately so cassette tests can call it without HTTP.
    ///
    /// # Errors
    ///
    /// Returns [`EiaProviderError::Provider`] on JSON parse failures or
    /// non-numeric value strings.
    pub fn parse_bytes(&self, raw: &[u8]) -> Result<Vec<EiaNaturalGasRecord>> {
        parse_natural_gas_bytes(raw)
    }
}

fn parse_natural_gas_bytes(raw: &[u8]) -> Result<Vec<EiaNaturalGasRecord>> {
    let envelope: EiaNaturalGasEnvelope = serde_json::from_slice(raw)
        .map_err(|e| EiaProviderError::Provider(format!("eia natural-gas parse_json: {e}")))?;
    let mut records = Vec::with_capacity(envelope.response.data.len());
    for row in envelope.response.data {
        if row.period.is_empty() {
            return Err(EiaProviderError::Provider(
                "eia natural-gas row missing period".to_string(),
            ));
        }
        let raw_value = row.value.trim();
        if raw_value.is_empty() || raw_value == "." {
            continue;
        }
        let value = raw_value.parse::<f64>().map_err(|e| {
            EiaProviderError::Provider(format!(
                "eia natural-gas value parse failed for {}: {e}",
                row.period
            ))
        })?;
        records.push(EiaNaturalGasRecord {
            period: row.period,
            series_description: row.series_description,
            value,
            units: row.units,
        });
    }
    Ok(records)
}
