#![cfg(feature = "http")]
//! Real BLS HTTP fetcher backed by `reqwest`.
//!
//! Gated by the `http` feature. Talks to the Bureau of Labor Statistics
//! `publicAPI/v2/timeseries/data` endpoint. An optional API key is read from
//! the `TDW_BLS_API_KEY` environment variable; without it the BLS public
//! rate-limits apply. Live integration tests are additionally gated by
//! `TDW_BLS_LIVE=1` so unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{API_KEY_ENV, BASE_URL, BlsDataPoint, BlsSeriesQuery, parse_bls_response};

const USER_AGENT: &str = "tdw-provider-bls/0.1";

/// Thin wrapper around the raw BLS JSON envelope used only for deserialization.
#[derive(Deserialize)]
struct BlsEnvelope {
    #[serde(default)]
    status: String,
    /// Full `Results` object; kept as a raw `Value` so we can hand it to the
    /// shared [`parse_bls_response`] helper.
    #[serde(default)]
    #[allow(dead_code)]
    message: Vec<String>,
}

/// Provider specification for the BLS `timeseries/data` fetcher.
///
/// Submits a POST request to the BLS v2 API and decodes the JSON response
/// into [`BlsDataPoint`] rows. Use [`BlsHttpTimeSeriesFetcher::default`] for
/// normal operation; builder methods allow base-URL overriding in tests.
pub struct BlsTimeSeriesSpec;

impl ProviderSpec for BlsTimeSeriesSpec {
    const PROVIDER: &'static str = "bls";
    const ENDPOINT: &'static str = "timeseries_data";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "bls client build";
    const SEND_ERR: &'static str = "bls extract_data send";
    const RETURNED_ERR: &'static str = "bls extract_data returned";
    const READ_BODY_ERR: &'static str = "bls read body";
    const UNREADABLE_BODY: Option<&'static str> = Some("<unreadable body>");

    type Query = BlsSeriesQuery;
    type Data = BlsDataPoint;

    fn transform_query(params: Value) -> Result<BlsSeriesQuery> {
        let series_ids: Vec<String> = params
            .get("series_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidQuery("bls series_ids must be a JSON array".to_string()))?
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    Error::InvalidQuery(format!("bls series_ids[{i}] must be a string"))
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let start_year = params
            .get("start_year")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::InvalidQuery("bls start_year must be a number".to_string()))?
            as u16;

        let end_year = params
            .get("end_year")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::InvalidQuery("bls end_year must be a number".to_string()))?
            as u16;

        BlsSeriesQuery::new(series_ids, start_year, end_year)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &BlsSeriesQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_optional_key(API_KEY_ENV);

        let endpoint = format!("{}/timeseries/data/", base_url.trim_end_matches('/'));

        let mut body = json!({
            "seriesid": query.series_ids,
            "startyear": query.start_year.to_string(),
            "endyear": query.end_year.to_string(),
        });

        if let Some(key) = api_key {
            body["registrationkey"] = Value::String(key);
        }

        Ok(client.post(&endpoint).json(&body))
    }

    fn transform_data(_query: &BlsSeriesQuery, raw: Bytes) -> Result<Vec<BlsDataPoint>> {
        let body: Value = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("bls parse json: {e}")))?;

        // Propagate any top-level message strings as a combined error.
        if let Some(msgs) = body.get("message").and_then(Value::as_array) {
            let combined: String = msgs
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("; ");
            if !combined.is_empty() {
                // Surface as a warning in the provider error; callers can
                // inspect `BlsProviderError` for specifics.
                let status = body
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("UNKNOWN");
                if status != "REQUEST_SUCCEEDED" {
                    return Err(Error::Provider(format!("bls api message: {combined}")));
                }
            }
        }

        parse_bls_response(&body).map_err(|e| Error::Provider(e.to_string()))
    }
}

/// Production BLS `timeseries/data` fetcher.
pub type BlsHttpTimeSeriesFetcher = HttpFetcher<BlsTimeSeriesSpec>;

// Silence unused-import warning for BlsEnvelope.status when not used.
impl BlsEnvelope {
    #[allow(dead_code)]
    fn is_success(&self) -> bool {
        self.status == "REQUEST_SUCCEEDED"
    }
}
