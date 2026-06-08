//! Real FRED backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the St. Louis Fed FRED
//! `series/observations` endpoint directly via `reqwest`. Live calls
//! require `FRED_API_KEY`; the live integration test is additionally
//! gated by `TDW_FRED_LIVE=1` so unattended CI stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;

use crate::{BASE_URL, FredObservation, FredSeriesObservationsQuery, series_observations_request};

const API_KEY_ENV: &str = "FRED_API_KEY";
const USER_AGENT: &str = "tdw-provider-fred/0.1";

/// Production FRED `series/observations` fetcher.
#[derive(Clone, Debug)]
pub struct FredHttpSeriesObservationsFetcher {
    base_url: String,
    observation_start: Option<String>,
    observation_end: Option<String>,
    limit: Option<u32>,
}

impl Default for FredHttpSeriesObservationsFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
            observation_start: None,
            observation_end: None,
            limit: Some(1_000),
        }
    }
}

impl FredHttpSeriesObservationsFetcher {
    /// Override the FRED base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Restrict observations returned by FRED to dates on or after
    /// `YYYY-MM-DD`.
    pub fn with_observation_start(mut self, observation_start: impl Into<String>) -> Self {
        self.observation_start = Some(observation_start.into());
        self
    }

    /// Restrict observations returned by FRED to dates on or before
    /// `YYYY-MM-DD`.
    pub fn with_observation_end(mut self, observation_end: impl Into<String>) -> Self {
        self.observation_end = Some(observation_end.into());
        self
    }

    /// Override FRED's result limit. The default keeps live calls
    /// bounded while still covering normal dashboard queries.
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Registry entry advertised under the canonical `fred` provider
    /// name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[derive(Deserialize)]
struct FredEnvelope {
    #[serde(default)]
    observations: Vec<FredRawObservation>,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct FredRawObservation {
    #[serde(default)]
    realtime_start: String,
    #[serde(default)]
    realtime_end: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    value: String,
}

#[async_trait]
impl Fetcher<FredSeriesObservationsQuery, FredObservation> for FredHttpSeriesObservationsFetcher {
    const PROVIDER: &'static str = "fred";
    const ENDPOINT: &'static str = "series_observations";

    fn transform_query(params: Value) -> Result<FredSeriesObservationsQuery> {
        // Shared normalization parses series_id alongside the standard
        // start_date/end_date/limit parameters in one pass.
        FredSeriesObservationsQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &FredSeriesObservationsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        series_observations_request(&query.series_id, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = std::env::var(API_KEY_ENV)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Provider(format!("fred api key env {API_KEY_ENV} must be set"))
            })?;
        let endpoint = format!(
            "{}/series/observations",
            self.base_url.trim_end_matches('/')
        );
        let mut query_params = vec![
            ("series_id", query.series_id.clone()),
            ("api_key", api_key),
            ("file_type", "json".to_string()),
        ];
        // Caller-supplied standard params (parsed once by the shared
        // normalization) take precedence over the fetcher's configured
        // defaults; FRED maps them onto observation_start/observation_end/limit.
        let limit = query.params.limit.unwrap_or(0);
        if limit > 0 {
            query_params.push(("limit", limit.to_string()));
        } else if let Some(limit) = self.limit {
            query_params.push(("limit", limit.to_string()));
        }
        if let Some(start) = query.params.start_date {
            query_params.push(("observation_start", start.to_string()));
        } else if let Some(observation_start) = &self.observation_start {
            query_params.push(("observation_start", observation_start.clone()));
        }
        if let Some(end) = query.params.end_date {
            query_params.push(("observation_end", end.to_string()));
        } else if let Some(observation_end) = &self.observation_end {
            query_params.push(("observation_end", observation_end.clone()));
        }

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("fred client: {error}")))?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("fred extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "fred extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("fred read body: {error}")))
    }

    fn transform_data(
        &self,
        query: &FredSeriesObservationsQuery,
        raw: Bytes,
    ) -> Result<Vec<FredObservation>> {
        let envelope: FredEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("fred parse_json: {error}")))?;
        if let Some(error_code) = envelope.error_code {
            return Err(Error::Provider(format!(
                "fred api error {error_code}: {}",
                envelope.error_message.unwrap_or_default()
            )));
        }

        let mut rows = Vec::with_capacity(envelope.observations.len());
        for observation in envelope.observations {
            if observation.date.is_empty() {
                return Err(Error::Provider("fred observation missing date".to_string()));
            }
            let raw_value = observation.value.trim();
            if raw_value.is_empty() || raw_value == "." {
                continue;
            }
            let value = raw_value.parse::<f64>().map_err(|error| {
                Error::Provider(format!(
                    "fred observation value parse failed for {}: {error}",
                    observation.date
                ))
            })?;
            rows.push(FredObservation {
                series_id: query.series_id.clone(),
                date: observation.date,
                value,
                realtime_start: observation.realtime_start,
                realtime_end: observation.realtime_end,
            });
        }
        Ok(rows)
    }
}
