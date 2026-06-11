//! Real IMF Data Services SDMX-JSON HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public IMF Data Services
//! SDMX-JSON REST API (`http://dataservices.imf.org/REST/SDMX_JSON.svc/`). The
//! API is keyless. Live calls are gated by `TDW_IMF_LIVE=1` in the integration
//! tests.
//!
//! The `CompactData` envelope is deeply nested and irregular: `Series` may be a
//! single object or an array, and each series' `Obs` may likewise be a single
//! object or an array. Every numeric value (`@OBS_VALUE`) is a JSON string. The
//! fetcher parses this defensively and normalizes each observation to a
//! standardized [`tdw_domain::MacroSeries`] row — the raw SDMX shapes never
//! leak past this module.

#![cfg(feature = "http")]

use tdw_core::http_support::prelude::*;
use tdw_domain::MacroSeries;

use crate::{BASE_URL, ImfCompactDataQuery};

const USER_AGENT: &str = "tdw-provider-imf/0.1 (contact@finx.example)";

/// One decoded observation from a single `Series`, tagged with the series
/// identity dimensions read off the `Series` attributes.
struct DecodedObservation {
    series_id: String,
    date: String,
    value: Option<f64>,
    frequency: Option<String>,
    country: Option<String>,
}

/// Decode a single SDMX `@OBS_VALUE` field (a JSON string, occasionally a bare
/// JSON number) as an `f64`. Empty / `"null"` strings are treated as absent.
fn parse_obs_value(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                None
            } else {
                trimmed.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// Read an SDMX string attribute, treating empty / `"null"` as absent.
fn parse_attr(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

/// Normalize a value that SDMX renders as *either* a single object *or* an
/// array of objects into a borrowed slice-like iterator. SDMX uses a bare
/// object when there is exactly one element and an array otherwise; this
/// collapses both into one code path.
fn as_object_seq(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => Vec::new(),
    }
}

/// Build the `series_id` carried into each [`MacroSeries`] row from the SDMX
/// series-identity dimensions, e.g. `IFS.M.US.PMP_IX`. Missing dimensions are
/// elided so the id stays meaningful for partially-attributed series.
fn series_identity(
    database: &str,
    freq: Option<&str>,
    ref_area: Option<&str>,
    indicator: Option<&str>,
) -> String {
    let mut parts = vec![database.to_string()];
    parts.extend(
        [freq, ref_area, indicator]
            .into_iter()
            .flatten()
            .map(str::to_string),
    );
    parts.join(".")
}

/// Decode the nested `CompactData` envelope into a flat list of decoded
/// observations, handling the single-or-array shape at both the `Series` and
/// `Obs` levels.
fn decode_compact_data(raw: &Bytes, database: &str) -> Result<Vec<DecodedObservation>> {
    let root: Value = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("imf parse_json: {error}")))?;

    // IMF surfaces query errors as a JSON object lacking the CompactData node;
    // treat a missing CompactData/DataSet as an empty (not malformed) result so
    // an out-of-range window does not error, but a non-object root does.
    let Value::Object(_) = &root else {
        return Err(Error::Provider(
            "imf parse_json: expected a JSON object envelope".to_string(),
        ));
    };

    let Some(data_set) = root.get("CompactData").and_then(|cd| cd.get("DataSet")) else {
        return Ok(Vec::new());
    };

    let mut rows = Vec::new();
    for series in as_object_seq(data_set.get("Series")) {
        let freq = parse_attr(series.get("@FREQ"));
        let ref_area = parse_attr(series.get("@REF_AREA"));
        let indicator = parse_attr(series.get("@INDICATOR"));
        let series_id = series_identity(
            database,
            freq.as_deref(),
            ref_area.as_deref(),
            indicator.as_deref(),
        );
        for obs in as_object_seq(series.get("Obs")) {
            let Some(date) = parse_attr(obs.get("@TIME_PERIOD")) else {
                continue;
            };
            rows.push(DecodedObservation {
                series_id: series_id.clone(),
                date,
                value: parse_obs_value(obs.get("@OBS_VALUE")),
                frequency: freq.clone(),
                country: ref_area.clone(),
            });
        }
    }
    Ok(rows)
}

/// Build the IMF `CompactData` request query parameters from the query: the
/// `startPeriod`/`endPeriod` year window derived from the shared date params.
fn compact_data_query_params(query: &ImfCompactDataQuery) -> Vec<(String, String)> {
    let mut params = Vec::new();
    if let Some(start) = query.params.start_date {
        params.push(("startPeriod".to_string(), start.year().to_string()));
    }
    if let Some(end) = query.params.end_date {
        params.push(("endPeriod".to_string(), end.year().to_string()));
    }
    params
}

/// Perform an IMF SDMX-JSON `CompactData` GET for `database`/`key` with
/// `params`, returning the raw response bytes.
async fn fetch_compact_data(
    base_url: &str,
    database: &str,
    key: &str,
    params: &[(String, String)],
    ctx: &str,
) -> Result<Bytes> {
    let endpoint = format!(
        "{}/CompactData/{}/{}",
        base_url.trim_end_matches('/'),
        database,
        key
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "imf http client build")?;
    let response = client
        .get(&endpoint)
        .query(params)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} extract_data: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!("{ctx} returned {status}: {body}")));
    }
    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} read body: {error}")))
}

tdw_core::provider_fetcher_struct!(
    /// Production IMF SDMX-JSON `CompactData` fetcher.
    ///
    /// Resolves an `economy/imf/*` catalog `command` to an IMF SDMX database and
    /// normalizes the caller's `key` series observations to
    /// [`tdw_domain::MacroSeries`] rows.
    pub ImfHttpMacroSeriesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<ImfCompactDataQuery, MacroSeries> for ImfHttpMacroSeriesFetcher {
    const PROVIDER: &'static str = "imf";
    const ENDPOINT: &'static str = "macro_series";

    fn transform_query(params: Value) -> Result<ImfCompactDataQuery> {
        ImfCompactDataQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &ImfCompactDataQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        fetch_compact_data(
            self.base_url(),
            &query.database,
            &query.key,
            &compact_data_query_params(query),
            "imf compact_data",
        )
        .await
    }

    fn transform_data(&self, query: &ImfCompactDataQuery, raw: Bytes) -> Result<Vec<MacroSeries>> {
        let endpoint = query.endpoint();
        let limit = query
            .params
            .limit
            .map_or(usize::MAX, |limit| limit as usize);
        let decoded = decode_compact_data(&raw, &query.database)?;
        Ok(decoded
            .into_iter()
            .take(limit)
            .map(|observation| MacroSeries {
                series_id: observation.series_id,
                title: Some(endpoint.description.to_string()),
                date: observation.date,
                value: observation.value,
                country: observation.country,
                frequency: observation.frequency,
                unit: None,
                transform: None,
            })
            .collect())
    }
}
