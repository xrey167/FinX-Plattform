#![cfg(feature = "http")]
//! Real EIA HTTP fetchers for spot-price and natural-gas endpoints.
//!
//! Both fetchers implement the canonical [`tdw_core::Fetcher`] trait and are
//! gated by the `http` Cargo feature. Live calls require `TDW_EIA_API_KEY`;
//! the live integration tests are additionally gated by `TDW_EIA_LIVE=1` so
//! unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{
    API_KEY_ENV, BASE_URL, EiaNaturalGasQuery, EiaNaturalGasRecord, EiaSpotPriceQuery,
    EiaSpotPriceRecord, PROVIDER_ID,
};

const USER_AGENT: &str = "tdw-provider-eia/0.1";

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

/// Provider specification for the EIA petroleum spot-price fetcher.
///
/// Calls `GET /petroleum/pri/spt/data/` with daily frequency.
pub struct EiaSpotPriceSpec;

impl ProviderSpec for EiaSpotPriceSpec {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "spot_price";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "eia build client";
    const SEND_ERR: &'static str = "eia spot-price request";
    const RETURNED_ERR: &'static str = "eia spot-price returned";
    const READ_BODY_ERR: &'static str = "eia spot-price read body";
    const UNREADABLE_BODY: Option<&'static str> = Some("<unreadable body>");

    type Query = EiaSpotPriceQuery;
    type Data = EiaSpotPriceRecord;

    fn transform_query(params: Value) -> Result<EiaSpotPriceQuery> {
        let query: EiaSpotPriceQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia spot-price query: {e}")))?;
        EiaSpotPriceQuery::new(query.commodity, query.length)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EiaSpotPriceQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "eia")?;
        let endpoint = format!("{}/petroleum/pri/spt/data/", base_url.trim_end_matches('/'));
        let params = [
            ("api_key", api_key),
            ("frequency", "daily".to_string()),
            ("data[0]", "value".to_string()),
            ("sort[0][column]", "period".to_string()),
            ("sort[0][direction]", "desc".to_string()),
            ("length", query.length.to_string()),
        ];
        Ok(client.get(&endpoint).query(&params))
    }

    fn transform_data(_query: &EiaSpotPriceQuery, raw: Bytes) -> Result<Vec<EiaSpotPriceRecord>> {
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

/// Production EIA petroleum spot-price fetcher.
pub type EiaHttpSpotPriceFetcher = HttpFetcher<EiaSpotPriceSpec>;

// ---------------------------------------------------------------------------
// EiaHttpNaturalGasFetcher
// ---------------------------------------------------------------------------

/// Provider specification for the EIA natural-gas price fetcher.
///
/// Calls `GET /natural-gas/pri/sum/data/` with monthly frequency.
pub struct EiaNaturalGasSpec;

impl ProviderSpec for EiaNaturalGasSpec {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "natural_gas";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "eia build client";
    const SEND_ERR: &'static str = "eia natural-gas request";
    const RETURNED_ERR: &'static str = "eia natural-gas returned";
    const READ_BODY_ERR: &'static str = "eia natural-gas read body";
    const UNREADABLE_BODY: Option<&'static str> = Some("<unreadable body>");

    type Query = EiaNaturalGasQuery;
    type Data = EiaNaturalGasRecord;

    fn transform_query(params: Value) -> Result<EiaNaturalGasQuery> {
        let query: EiaNaturalGasQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia natural-gas query: {e}")))?;
        EiaNaturalGasQuery::new(query.length).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EiaNaturalGasQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "eia")?;
        let endpoint = format!(
            "{}/natural-gas/pri/sum/data/",
            base_url.trim_end_matches('/')
        );
        let params = [
            ("api_key", api_key),
            ("frequency", "monthly".to_string()),
            ("data[0]", "value".to_string()),
            ("length", query.length.to_string()),
        ];
        Ok(client.get(&endpoint).query(&params))
    }

    fn transform_data(_query: &EiaNaturalGasQuery, raw: Bytes) -> Result<Vec<EiaNaturalGasRecord>> {
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

/// Production EIA natural-gas price fetcher.
pub type EiaHttpNaturalGasFetcher = HttpFetcher<EiaNaturalGasSpec>;
