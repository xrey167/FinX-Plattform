#![cfg(feature = "http")]
//! Real OECD SDMX-JSON backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the OECD Statistics SDMX-JSON
//! `data` endpoint directly via `reqwest`. No API key is required.
//!
//! The live integration test is additionally gated by `TDW_OECD_LIVE=1`
//! so unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, OecdObservation, OecdQuery, sdmx_data_url};

const USER_AGENT: &str = "tdw-provider-oecd/0.1";

// ── SDMX-JSON deserialisation types ──────────────────────────────────────────

/// Top-level OECD SDMX-JSON envelope.
///
/// Shape:
/// ```json
/// {
///   "dataSets": [{ "observations": { "0:0:0:0": [1234.5, 0, null] } }],
///   "structure": {
///     "dimensions": {
///       "observation": [{ "id": "TIME_PERIOD",
///                         "values": [{ "id": "2023-Q1", "name": "2023-Q1" }] }]
///     }
///   }
/// }
/// ```
#[derive(Deserialize)]
struct SdmxEnvelope {
    #[serde(rename = "dataSets", default)]
    data_sets: Vec<SdmxDataSet>,
    #[serde(default)]
    structure: Option<SdmxStructure>,
}

#[derive(Deserialize)]
struct SdmxDataSet {
    #[serde(default)]
    observations: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct SdmxStructure {
    #[serde(default)]
    dimensions: Option<SdmxDimensions>,
}

#[derive(Deserialize)]
struct SdmxDimensions {
    #[serde(default)]
    observation: Vec<SdmxDimension>,
}

#[derive(Deserialize)]
struct SdmxDimension {
    #[serde(default)]
    id: String,
    #[serde(default)]
    values: Vec<SdmxDimensionValue>,
}

#[derive(Deserialize)]
struct SdmxDimensionValue {
    #[serde(default)]
    id: String,
}

// ── Provider spec ─────────────────────────────────────────────────────────────

/// Provider specification for the OECD SDMX-JSON data fetcher.
///
/// No authentication is required. The base URL can be overridden for
/// testing against a local mock server.
pub struct OecdDataSpec;

impl ProviderSpec for OecdDataSpec {
    const PROVIDER: &'static str = "oecd";
    const ENDPOINT: &'static str = "sdmx_data";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "oecd client build";
    const SEND_ERR: &'static str = "oecd extract_data request";
    const RETURNED_ERR: &'static str = "oecd extract_data returned";
    const READ_BODY_ERR: &'static str = "oecd read body";

    type Query = OecdQuery;
    type Data = OecdObservation;

    fn transform_query(params: Value) -> Result<OecdQuery> {
        let get_str = |key: &str| -> Result<String> {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    Error::InvalidQuery(format!("oecd query field `{key}` must be a string"))
                })
        };
        let dataset = get_str("dataset")?;
        let filter = get_str("filter")?;
        let start_time = get_str("start_time")?;
        let end_time = get_str("end_time")?;
        OecdQuery::new(&dataset, &filter, &start_time, &end_time)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &OecdQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let base = base_url.trim_end_matches('/');
        let url = format!(
            "{}/{}/{}/OECD?startTime={}&endTime={}&contentType=application/json",
            base, query.dataset, query.filter, query.start_time, query.end_time,
        );
        Ok(client.get(&url))
    }

    fn transform_data(query: &OecdQuery, raw: Bytes) -> Result<Vec<OecdObservation>> {
        let envelope: SdmxEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("oecd parse_json: {e}")))?;

        // Extract the TIME_PERIOD dimension values so we can map observation
        // indices to period labels.
        let time_values: Vec<String> = envelope
            .structure
            .as_ref()
            .and_then(|s| s.dimensions.as_ref())
            .map(|d| {
                d.observation
                    .iter()
                    .find(|dim| dim.id == "TIME_PERIOD")
                    .map(|dim| dim.values.iter().map(|v| v.id.clone()).collect())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let data_set = match envelope.data_sets.into_iter().next() {
            Some(ds) => ds,
            None => return Ok(vec![]),
        };

        let mut rows = Vec::new();
        for (obs_key, obs_val) in &data_set.observations {
            // Each observation is an array; index 0 is the numeric value.
            let value = match obs_val.get(0).and_then(Value::as_f64) {
                Some(v) => v,
                None => continue, // null or missing — skip
            };

            // The TIME_PERIOD index is the last positional component of the
            // colon-separated key (e.g. "0:0:0:3" → index 3).
            let period = obs_key
                .split(':')
                .next_back()
                .and_then(|idx| idx.parse::<usize>().ok())
                .and_then(|idx| time_values.get(idx))
                .cloned()
                .unwrap_or_else(|| obs_key.clone());

            rows.push(OecdObservation {
                dataset: query.dataset.clone(),
                key: obs_key.clone(),
                period,
                value,
            });
        }

        // Stable output order: sort by key string so tests are deterministic.
        rows.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(rows)
    }
}

/// Production OECD SDMX-JSON data fetcher.
pub type OecdHttpDataFetcher = HttpFetcher<OecdDataSpec>;

// ── URL helper used by the canonical sdmx_data_url fn ────────────────────────

/// Validate that `sdmx_data_url` still works when called from http feature
/// code. (Compile-time guard only; the real URL builder lives in lib.rs.)
#[allow(dead_code)]
fn _assert_sdmx_data_url_accessible(q: &OecdQuery) -> crate::Result<String> {
    sdmx_data_url(q)
}
