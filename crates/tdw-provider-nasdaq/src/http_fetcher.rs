#![cfg(feature = "http")]
//! Real NASDAQ Data Link backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the NASDAQ Data Link
//! `datasets/{database}/{dataset}/data` endpoint directly via
//! `reqwest`. Live calls require `TDW_NASDAQ_API_KEY`; the live
//! integration test is additionally gated by `TDW_NASDAQ_LIVE=1` so
//! unattended CI stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;

use crate::{BASE_URL, NasdaqDataRow, NasdaqDatasetQuery, dataset_request};

const API_KEY_ENV: &str = "TDW_NASDAQ_API_KEY";
const USER_AGENT: &str = "tdw-provider-nasdaq/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production NASDAQ Data Link dataset fetcher.
    pub NasdaqHttpDatasetFetcher,
    BASE_URL
);

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

#[async_trait]
impl Fetcher<NasdaqDatasetQuery, NasdaqDataRow> for NasdaqHttpDatasetFetcher {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "datasets";

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

    async fn extract_data(
        &self,
        query: &NasdaqDatasetQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        dataset_request(&query.database, &query.dataset, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Provider(format!("nasdaq api key env {API_KEY_ENV} must be set"))
            })?;

        let endpoint = format!(
            "{}/datasets/{}/{}/data",
            self.base_url().trim_end_matches('/'),
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

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("nasdaq client: {error}")))?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("nasdaq extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("nasdaq read body: {error}")))
    }

    fn transform_data(&self, query: &NasdaqDatasetQuery, raw: Bytes) -> Result<Vec<NasdaqDataRow>> {
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
