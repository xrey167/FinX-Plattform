#![cfg(feature = "http")]
//! Real NASDAQ Data Link backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the NASDAQ Data Link
//! `datasets/{database}/{dataset}/data` endpoint directly via
//! `reqwest`. Live calls require `TDW_NASDAQ_API_KEY`; the live
//! integration test is additionally gated by `TDW_NASDAQ_LIVE=1` so
//! unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, NasdaqDataRow, NasdaqDatasetQuery, dataset_request};

const API_KEY_ENV: &str = "TDW_NASDAQ_API_KEY";
const USER_AGENT: &str = "tdw-provider-nasdaq/0.1";

/// Top-level API envelope for the dataset data endpoint.
#[derive(Deserialize)]
struct NasdaqEnvelope {
    #[serde(default)]
    dataset_data: Option<NasdaqDatasetData>,
    /// Present when the API returns an error.
    #[serde(default)]
    quandl_error: Option<NasdaqError>,
}

#[derive(Deserialize)]
struct NasdaqDatasetData {
    #[serde(default)]
    column_names: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<Value>>,
}

#[derive(Deserialize)]
struct NasdaqError {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Provider specification for the NASDAQ Data Link dataset fetcher.
pub struct NasdaqDatasetSpec;

impl ProviderSpec for NasdaqDatasetSpec {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "datasets";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "nasdaq client";
    const SEND_ERR: &'static str = "nasdaq extract_data";
    const RETURNED_ERR: &'static str = "nasdaq extract_data returned";
    const READ_BODY_ERR: &'static str = "nasdaq read body";

    type Query = NasdaqDatasetQuery;
    type Data = NasdaqDataRow;

    fn transform_query(params: Value) -> Result<NasdaqDatasetQuery> {
        let database = params
            .get("database")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("nasdaq database must be a string".to_string()))?;
        let dataset = params
            .get("dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("nasdaq dataset must be a string".to_string()))?;

        let mut query = NasdaqDatasetQuery::new(database, dataset)
            .map_err(|error| Error::InvalidQuery(error.to_string()))?;

        if let Some(start) = params.get("start_date").and_then(Value::as_str) {
            query = query
                .with_start_date(start)
                .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        }
        if let Some(end) = params.get("end_date").and_then(Value::as_str) {
            query = query
                .with_end_date(end)
                .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        }

        Ok(query)
    }

    fn build_request(
        base_url: &str,
        query: &NasdaqDatasetQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        dataset_request(&query.database, &query.dataset, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "nasdaq")?;

        let endpoint = format!(
            "{}/datasets/{}/{}/data",
            base_url.trim_end_matches('/'),
            query.database,
            query.dataset,
        );

        let mut query_params: Vec<(&str, String)> = vec![("api_key", api_key)];
        if let Some(start) = &query.start_date {
            query_params.push(("start_date", start.clone()));
        }
        if let Some(end) = &query.end_date {
            query_params.push(("end_date", end.clone()));
        }

        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &NasdaqDatasetQuery, raw: Bytes) -> Result<Vec<NasdaqDataRow>> {
        let envelope: NasdaqEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("nasdaq parse_json: {error}")))?;

        if let Some(api_error) = envelope.quandl_error {
            let code = api_error.code.unwrap_or_default();
            let message = api_error.message.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq api error {code}: {message}"
            )));
        }

        let dataset_data = envelope.dataset_data.ok_or_else(|| {
            Error::Provider("nasdaq response missing dataset_data field".to_string())
        })?;

        let mut rows = Vec::with_capacity(dataset_data.data.len());
        for data_row in dataset_data.data {
            rows.push(NasdaqDataRow {
                database: query.database.clone(),
                dataset: query.dataset.clone(),
                column_names: dataset_data.column_names.clone(),
                values: data_row,
            });
        }
        Ok(rows)
    }
}

/// Production NASDAQ Data Link dataset fetcher.
pub type NasdaqHttpDatasetFetcher = HttpFetcher<NasdaqDatasetSpec>;
