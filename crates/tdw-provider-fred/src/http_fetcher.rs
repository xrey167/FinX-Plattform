//! Real FRED backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the St. Louis Fed FRED
//! `series/observations` endpoint directly via `reqwest`. Live calls
//! require `FRED_API_KEY`; the live integration test is additionally
//! gated by `TDW_FRED_LIVE=1` so unattended CI stays offline.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_core::query_params::StandardParams;
use tdw_domain::{MacroSeries, RateObservation, SeriesSearchResult, YieldCurvePoint};

use crate::{
    BASE_URL, FredCatalogQuery, FredObservation, FredSearchQuery, FredSeriesObservationsQuery,
    FredYieldCurveQuery, YIELD_CURVE_LEGS, fred_search_request, series_observations_request,
};

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

/// A decoded `(date, value)` pair, with FRED's missing-value sentinel (`"."`)
/// and blank rows already skipped. Shared by all three fetchers.
struct ParsedObservation {
    date: String,
    value: f64,
    realtime_start: String,
    realtime_end: String,
}

/// Read the required FRED API key from the environment, byte-identical to the
/// inline form used by the original observations fetcher.
fn fred_api_key() -> Result<String> {
    std::env::var(API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Provider(format!("fred api key env {API_KEY_ENV} must be set")))
}

/// Perform a FRED `series/observations` GET for `series_id`, mapping the shared
/// `start_date`/`end_date`/`limit` params onto FRED's request parameters and
/// returning the raw response bytes. `default_limit` is applied when the caller
/// supplied no `limit`.
async fn fetch_series_observations(
    base_url: &str,
    series_id: &str,
    params: &StandardParams,
    default_limit: Option<u32>,
) -> Result<Bytes> {
    let api_key = fred_api_key()?;
    let endpoint = format!("{}/series/observations", base_url.trim_end_matches('/'));
    let mut query_params = vec![
        ("series_id", series_id.to_string()),
        ("api_key", api_key),
        ("file_type", "json".to_string()),
    ];
    let limit = params.limit.unwrap_or(0);
    if limit > 0 {
        query_params.push(("limit", limit.to_string()));
    } else if let Some(limit) = default_limit {
        query_params.push(("limit", limit.to_string()));
    }
    if let Some(start) = params.start_date {
        query_params.push(("observation_start", start.to_string()));
    }
    if let Some(end) = params.end_date {
        query_params.push(("observation_end", end.to_string()));
    }

    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
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

/// Decode a FRED observations envelope, surfacing the API error envelope and
/// skipping missing (`"."`/blank) values. Shared by all three fetchers.
fn parse_observations(raw: &Bytes) -> Result<Vec<ParsedObservation>> {
    let envelope: FredEnvelope = serde_json::from_slice(raw)
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
        rows.push(ParsedObservation {
            date: observation.date,
            value,
            realtime_start: observation.realtime_start,
            realtime_end: observation.realtime_end,
        });
    }
    Ok(rows)
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
        // Merge the fetcher's builder defaults under the caller-supplied
        // standard params, then delegate to the shared observation fetch. The
        // builder's observation_start/end act as defaults when the caller did
        // not supply start_date/end_date.
        let mut params = query.params.clone();
        if params.start_date.is_none()
            && let Some(start) = &self.observation_start
        {
            params.start_date = StandardParams::from_value(&serde_json::json!({
                "start_date": start,
            }))
            .ok()
            .and_then(|p| p.start_date);
        }
        if params.end_date.is_none()
            && let Some(end) = &self.observation_end
        {
            params.end_date = StandardParams::from_value(&serde_json::json!({
                "end_date": end,
            }))
            .ok()
            .and_then(|p| p.end_date);
        }
        fetch_series_observations(&self.base_url, &query.series_id, &params, self.limit).await
    }

    fn transform_data(
        &self,
        query: &FredSeriesObservationsQuery,
        raw: Bytes,
    ) -> Result<Vec<FredObservation>> {
        let parsed = parse_observations(&raw)?;
        Ok(parsed
            .into_iter()
            .map(|observation| FredObservation {
                series_id: query.series_id.clone(),
                date: observation.date,
                value: observation.value,
                realtime_start: observation.realtime_start,
                realtime_end: observation.realtime_end,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// FredHttpMacroSeriesFetcher — normalizes the economy/* cluster to MacroSeries
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FRED fetcher that resolves a catalog `command` to a FRED series
    /// and normalizes observations to [`tdw_domain::MacroSeries`].
    ///
    /// Standardizes the `economy/*` series cluster (cpi, pce, gdp, unemployment,
    /// money measures, survey series) per `docs/roadmap/openbb-surface-domains.md`.
    pub FredHttpMacroSeriesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FredCatalogQuery, MacroSeries> for FredHttpMacroSeriesFetcher {
    const PROVIDER: &'static str = "fred";
    const ENDPOINT: &'static str = "macro_series";

    fn transform_query(params: Value) -> Result<FredCatalogQuery> {
        FredCatalogQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &FredCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_series_observations(self.base_url(), &query.series_id, &query.params, None).await
    }

    fn transform_data(&self, query: &FredCatalogQuery, raw: Bytes) -> Result<Vec<MacroSeries>> {
        let endpoint = query.endpoint();
        let parsed = parse_observations(&raw)?;
        Ok(parsed
            .into_iter()
            .map(|observation| MacroSeries {
                series_id: query.series_id.clone(),
                title: Some(endpoint.title.to_string()),
                date: observation.date,
                value: Some(observation.value),
                country: None,
                frequency: Some(endpoint.frequency.to_string()),
                unit: Some(endpoint.unit.to_string()),
                transform: None,
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// FredHttpRateObservationFetcher — normalizes the fixedincome/* cluster to
// RateObservation
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FRED fetcher that resolves a catalog `command` to a FRED series
    /// and normalizes observations to [`tdw_domain::RateObservation`].
    ///
    /// Standardizes the `fixedincome/rate/*`, `fixedincome/spreads/*`,
    /// `fixedincome/government/*`, `fixedincome/corporate/*`, and bond/mortgage
    /// index series per `docs/roadmap/openbb-surface-domains.md`.
    pub FredHttpRateObservationFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FredCatalogQuery, RateObservation> for FredHttpRateObservationFetcher {
    const PROVIDER: &'static str = "fred";
    const ENDPOINT: &'static str = "rate_observation";

    fn transform_query(params: Value) -> Result<FredCatalogQuery> {
        FredCatalogQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &FredCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_series_observations(self.base_url(), &query.series_id, &query.params, None).await
    }

    fn transform_data(&self, query: &FredCatalogQuery, raw: Bytes) -> Result<Vec<RateObservation>> {
        let endpoint = query.endpoint();
        let maturity = endpoint.maturity_opt().map(str::to_string);
        let parsed = parse_observations(&raw)?;
        Ok(parsed
            .into_iter()
            .map(|observation| RateObservation {
                rate_id: query.series_id.clone(),
                date: observation.date,
                value: Some(observation.value),
                maturity: maturity.clone(),
                currency: Some(endpoint.currency.to_string()),
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// FredHttpSeriesSearchFetcher — economy/fred_search -> SeriesSearchResult
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FRED `series/search` metadata fetcher.
    ///
    /// Standardizes `economy/fred_search`: a free-text query against FRED's
    /// `series/search` endpoint, normalized to [`tdw_domain::SeriesSearchResult`].
    /// This is a discovery endpoint — it returns series metadata, not
    /// observations — so it carries no bronze landing table.
    pub FredHttpSeriesSearchFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct FredSearchEnvelope {
    #[serde(default)]
    seriess: Vec<FredSearchSeries>,
    #[serde(default)]
    error_code: Option<i64>,
    #[serde(default)]
    error_message: Option<String>,
}

#[derive(Deserialize)]
struct FredSearchSeries {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    frequency: Option<String>,
    #[serde(default)]
    units: Option<String>,
    #[serde(default)]
    popularity: Option<i64>,
}

#[async_trait]
impl Fetcher<FredSearchQuery, SeriesSearchResult> for FredHttpSeriesSearchFetcher {
    const PROVIDER: &'static str = "fred";
    const ENDPOINT: &'static str = "fred_search";

    fn transform_query(params: Value) -> Result<FredSearchQuery> {
        FredSearchQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &FredSearchQuery, _creds: &Credentials) -> Result<Bytes> {
        // Reuse the shared request builder for its api-key guard + percent
        // encoding contract, then issue the GET via the shared client.
        fred_search_request(&query.search_text, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = fred_api_key()?;
        let endpoint = format!("{}/series/search", self.base_url().trim_end_matches('/'));
        let mut query_params = vec![
            ("search_text", query.search_text.clone()),
            ("api_key", api_key),
            ("file_type", "json".to_string()),
        ];
        let limit = query.params.limit.unwrap_or(0);
        if limit > 0 {
            query_params.push(("limit", limit.to_string()));
        }
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("fred client: {error}")))?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("fred search: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "fred search returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("fred read body: {error}")))
    }

    fn transform_data(
        &self,
        _query: &FredSearchQuery,
        raw: Bytes,
    ) -> Result<Vec<SeriesSearchResult>> {
        let envelope: FredSearchEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("fred parse_json: {error}")))?;
        if let Some(error_code) = envelope.error_code {
            return Err(Error::Provider(format!(
                "fred api error {error_code}: {}",
                envelope.error_message.unwrap_or_default()
            )));
        }
        let mut rows = Vec::with_capacity(envelope.seriess.len());
        for series in envelope.seriess {
            if series.id.is_empty() {
                continue;
            }
            rows.push(SeriesSearchResult {
                series_id: series.id,
                title: series.title,
                frequency: series.frequency,
                units: series.units,
                popularity: series.popularity,
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FredHttpYieldCurveFetcher — fixedincome/government/yield_curve ->
// YieldCurvePoint (aggregates the four Treasury constant-maturity legs)
// ---------------------------------------------------------------------------

/// One leg's decoded observations, tagged with its tenor, ready to merge into
/// the combined curve. The intermediate combined envelope is a JSON array of
/// these so the `transform_data` path is exercisable from a recorded fixture.
#[derive(serde::Serialize, Deserialize)]
struct YieldCurveLeg {
    maturity: String,
    series_id: String,
    currency: String,
    observations: Vec<YieldCurveLegObservation>,
}

#[derive(serde::Serialize, Deserialize)]
struct YieldCurveLegObservation {
    date: String,
    value: f64,
}

tdw_core::provider_fetcher_struct!(
    /// Production FRED Treasury yield-curve fetcher.
    ///
    /// Standardizes `fixedincome/government/yield_curve` by fetching the four
    /// Treasury constant-maturity series (3m / 2y / 10y / 30y) and merging them
    /// into one [`tdw_domain::YieldCurvePoint`] per `(date, maturity)`. The legs
    /// are fetched sequentially; a recorded combined fixture drives the
    /// offline-testable `transform_data` path.
    pub FredHttpYieldCurveFetcher,
    BASE_URL
);

impl FredHttpYieldCurveFetcher {
    /// Resolve and fetch one curve leg, returning its tagged observations.
    async fn fetch_leg(
        &self,
        command: &str,
        maturity: &str,
        params: &StandardParams,
    ) -> Result<YieldCurveLeg> {
        let endpoint = crate::catalog::resolve(command).ok_or_else(|| {
            Error::Provider(format!("yield_curve leg command not in catalog: {command}"))
        })?;
        let raw =
            fetch_series_observations(self.base_url(), endpoint.series_id, params, None).await?;
        let parsed = parse_observations(&raw)?;
        Ok(YieldCurveLeg {
            maturity: maturity.to_string(),
            series_id: endpoint.series_id.to_string(),
            currency: endpoint.currency.to_string(),
            observations: parsed
                .into_iter()
                .map(|observation| YieldCurveLegObservation {
                    date: observation.date,
                    value: observation.value,
                })
                .collect(),
        })
    }
}

#[async_trait]
impl Fetcher<FredYieldCurveQuery, YieldCurvePoint> for FredHttpYieldCurveFetcher {
    const PROVIDER: &'static str = "fred";
    const ENDPOINT: &'static str = "yield_curve";

    fn transform_query(params: Value) -> Result<FredYieldCurveQuery> {
        FredYieldCurveQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &FredYieldCurveQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let mut legs = Vec::with_capacity(YIELD_CURVE_LEGS.len());
        for (command, maturity) in YIELD_CURVE_LEGS {
            legs.push(self.fetch_leg(command, maturity, &query.params).await?);
        }
        let combined = serde_json::to_vec(&legs)
            .map_err(|error| Error::Provider(format!("yield_curve encode: {error}")))?;
        Ok(Bytes::from(combined))
    }

    fn transform_data(
        &self,
        _query: &FredYieldCurveQuery,
        raw: Bytes,
    ) -> Result<Vec<YieldCurvePoint>> {
        let legs: Vec<YieldCurveLeg> = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yield_curve parse_json: {error}")))?;
        let mut rows = Vec::new();
        for leg in legs {
            for observation in leg.observations {
                rows.push(YieldCurvePoint {
                    curve_id: "us_treasury".to_string(),
                    date: observation.date,
                    maturity: leg.maturity.clone(),
                    value: Some(observation.value),
                    currency: Some(leg.currency.clone()),
                });
            }
        }
        Ok(rows)
    }
}
