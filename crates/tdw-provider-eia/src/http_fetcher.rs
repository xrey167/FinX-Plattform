#![cfg(feature = "http")]
//! Real EIA HTTP fetchers for spot-price and natural-gas endpoints.
//!
//! Both fetchers implement the canonical [`tdw_core::Fetcher`] trait and are
//! gated by the `http` Cargo feature. Live calls require `TDW_EIA_API_KEY`;
//! the live integration tests are additionally gated by `TDW_EIA_LIVE=1` so
//! unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};

use crate::{
    API_KEY_ENV, BASE_URL, EiaNaturalGasQuery, EiaNaturalGasRecord, EiaSpotPriceQuery,
    EiaSpotPriceRecord, PROVIDER_ID,
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
        .ok_or_else(|| Error::Provider(format!("eia api key env {API_KEY_ENV} must be set")))
}

fn build_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| Error::Provider(format!("eia build client: {e}")))
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
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `eia` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<EiaSpotPriceQuery, EiaSpotPriceRecord> for EiaHttpSpotPriceFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "spot_price";

    fn transform_query(params: Value) -> Result<EiaSpotPriceQuery> {
        let query: EiaSpotPriceQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia spot-price query: {e}")))?;
        EiaSpotPriceQuery::new(query.commodity, query.length)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &EiaSpotPriceQuery, _creds: &Credentials) -> Result<Bytes> {
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
            .map_err(|e| Error::Provider(format!("eia spot-price request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(Error::Provider(format!(
                "eia spot-price returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("eia spot-price read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &EiaSpotPriceQuery,
        raw: Bytes,
    ) -> Result<Vec<EiaSpotPriceRecord>> {
        let envelope: EiaSpotPriceEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("eia spot-price parse_json: {e}")))?;
        let mut records = Vec::with_capacity(envelope.response.data.len());
        for row in envelope.response.data {
            if row.period.is_empty() {
                return Err(Error::Provider(
                    "eia spot-price row missing period".to_string(),
                ));
            }
            let raw_value = row.value.trim();
            if raw_value.is_empty() || raw_value == "." {
                continue;
            }
            let value = raw_value.parse::<f64>().map_err(|e| {
                Error::Provider(format!(
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
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `eia` provider name.
    #[must_use]
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<EiaNaturalGasQuery, EiaNaturalGasRecord> for EiaHttpNaturalGasFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "natural_gas";

    fn transform_query(params: Value) -> Result<EiaNaturalGasQuery> {
        let query: EiaNaturalGasQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia natural-gas query: {e}")))?;
        EiaNaturalGasQuery::new(query.length).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &EiaNaturalGasQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
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
            .map_err(|e| Error::Provider(format!("eia natural-gas request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(Error::Provider(format!(
                "eia natural-gas returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("eia natural-gas read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &EiaNaturalGasQuery,
        raw: Bytes,
    ) -> Result<Vec<EiaNaturalGasRecord>> {
        let envelope: EiaNaturalGasEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("eia natural-gas parse_json: {e}")))?;
        let mut records = Vec::with_capacity(envelope.response.data.len());
        for row in envelope.response.data {
            if row.period.is_empty() {
                return Err(Error::Provider(
                    "eia natural-gas row missing period".to_string(),
                ));
            }
            let raw_value = row.value.trim();
            if raw_value.is_empty() || raw_value == "." {
                continue;
            }
            let value = raw_value.parse::<f64>().map_err(|e| {
                Error::Provider(format!(
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
}
