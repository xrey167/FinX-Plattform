#![cfg(feature = "http")]
//! Real BLS HTTP fetcher backed by `reqwest`.
//!
//! Gated by the `http` feature. Talks to the Bureau of Labor Statistics
//! `publicAPI/v2/timeseries/data` endpoint. An optional API key is read from
//! the `TDW_BLS_API_KEY` environment variable; without it the BLS public
//! rate-limits apply. Live integration tests are additionally gated by
//! `TDW_BLS_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Credentials, Error, Fetcher, Result};

use crate::{API_KEY_ENV, BASE_URL, BlsDataPoint, BlsSeriesQuery, parse_bls_response};

const USER_AGENT: &str = "tdw-provider-bls/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production BLS `timeseries/data` fetcher.
    ///
    /// Submits a POST request to the BLS v2 API and decodes the JSON response
    /// into [`BlsDataPoint`] rows. Use [`BlsHttpTimeSeriesFetcher::default`] for
    /// normal operation; builder methods allow base-URL overriding in tests.
    pub BlsHttpTimeSeriesFetcher,
    BASE_URL
);

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

#[async_trait]
impl Fetcher<BlsSeriesQuery, BlsDataPoint> for BlsHttpTimeSeriesFetcher {
    const PROVIDER: &'static str = "bls";
    const ENDPOINT: &'static str = "timeseries_data";

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

    async fn extract_data(&self, query: &BlsSeriesQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let endpoint = format!("{}/timeseries/data/", self.base_url().trim_end_matches('/'));

        let mut body = json!({
            "seriesid": query.series_ids,
            "startyear": query.start_year.to_string(),
            "endyear": query.end_year.to_string(),
        });

        if let Some(key) = api_key {
            body["registrationkey"] = Value::String(key);
        }

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| Error::Provider(format!("bls client build: {e}")))?;

        let response = client
            .post(&endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("bls extract_data send: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable body>"));
            return Err(Error::Provider(format!(
                "bls extract_data returned {status}: {text}"
            )));
        }

        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("bls read body: {e}")))
    }

    fn transform_data(&self, _query: &BlsSeriesQuery, raw: Bytes) -> Result<Vec<BlsDataPoint>> {
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

// Silence unused-import warning for BlsEnvelope.status when not used.
impl BlsEnvelope {
    #[allow(dead_code)]
    fn is_success(&self) -> bool {
        self.status == "REQUEST_SUCCEEDED"
    }
}
