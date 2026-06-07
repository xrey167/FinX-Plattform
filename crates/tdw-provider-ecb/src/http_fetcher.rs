#![cfg(feature = "http")]
//! Real ECB Statistical Data Warehouse HTTP fetcher.
//!
//! Gated by the `http` feature. Talks to the ECB SDW REST API
//! `/data/{flow}/{key}` endpoint via `reqwest`. No authentication is required.
//! The live integration test is additionally gated by `TDW_ECB_LIVE=1` so
//! unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, EcbDataQuery, EcbObservation, parse_ecb_value};

const USER_AGENT: &str = "tdw-provider-ecb/0.1";

/// Provider specification for the ECB SDW data fetcher.
pub struct EcbDataSpec;

impl ProviderSpec for EcbDataSpec {
    const PROVIDER: &'static str = "ecb";
    const ENDPOINT: &'static str = "data";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "ecb client build";
    const SEND_ERR: &'static str = "ecb extract_data";
    const RETURNED_ERR: &'static str = "ecb extract_data returned";
    const READ_BODY_ERR: &'static str = "ecb read body";

    type Query = EcbDataQuery;
    type Data = EcbObservation;

    fn transform_query(params: Value) -> Result<EcbDataQuery> {
        let flow = params
            .get("flow")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb flow must be a string".to_string()))?;
        let key = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb key must be a string".to_string()))?;
        let start_period = params
            .get("start_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb start_period must be a string".to_string()))?;
        let end_period = params
            .get("end_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb end_period must be a string".to_string()))?;
        EcbDataQuery::new(flow, key, start_period, end_period)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EcbDataQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let endpoint = format!(
            "{}/data/{}/{}",
            base_url.trim_end_matches('/'),
            query.flow,
            query.key,
        );
        let query_params = [
            ("format", "jsondata"),
            ("startPeriod", query.start_period.as_str()),
            ("endPeriod", query.end_period.as_str()),
        ];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &EcbDataQuery, raw: Bytes) -> Result<Vec<EcbObservation>> {
        let v: Value = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("ecb parse_json: {error}")))?;
        parse_ecb_value(&v, &query.flow, &query.key)
            .map_err(|error| Error::Provider(error.to_string()))
    }
}

/// Production ECB SDW data fetcher.
pub type EcbHttpDataFetcher = HttpFetcher<EcbDataSpec>;
