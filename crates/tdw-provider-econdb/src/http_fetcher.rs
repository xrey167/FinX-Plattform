//! Real `EconDB` public-API HTTP fetcher for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public `EconDB` API
//! (`https://www.econdb.com/api/series/{TICKER}/?format=json`). Public series are
//! keyless; an optional free token (read from `TDW_ECONDB_API_KEY` via the shared
//! `read_optional_key` helper) raises limits / unlocks series and is omitted when
//! the env var is unset. Live calls are gated by `TDW_ECONDB_LIVE=1` in the
//! integration tests.
//!
//! The series response is a single object carrying metadata (`ticker`,
//! `description`, `geography`, `frequency`, `units`) plus a `data` payload that
//! `EconDB` renders in either of two shapes: an object with parallel `dates` +
//! `values` arrays, or a list of `{date, value}` records. The fetcher parses both
//! defensively and normalizes each observation to a standardized
//! [`tdw_domain::MacroSeries`] row — the raw `EconDB` shapes never leak past this
//! module. Values may be JSON numbers or numeric strings and are coerced to
//! `f64`; a missing / null value yields a row with `value: None` (the date is
//! never dropped).

#![cfg(feature = "http")]

use tdw_core::http_support::prelude::*;
use tdw_domain::MacroSeries;

use crate::{API_KEY_ENV, BASE_URL, EcondbSeriesQuery};

const USER_AGENT: &str = "tdw-provider-econdb/0.1 (contact@finx.example)";

/// Series-level metadata read off the `EconDB` response, threaded onto each row.
#[derive(Default)]
struct SeriesMeta {
    title: Option<String>,
    country: Option<String>,
    frequency: Option<String>,
    unit: Option<String>,
}

/// One decoded observation: a (date, value) pair from the series data payload.
struct DecodedObservation {
    date: String,
    value: Option<f64>,
}

/// Decode a single `EconDB` value field as an `f64`. `EconDB` returns values as
/// JSON numbers or occasionally numeric strings. Empty / `"null"` strings, JSON
/// null, and missing fields are treated as absent.
fn parse_value(value: Option<&Value>) -> Option<f64> {
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

/// Read an `EconDB` string field, treating empty / `"null"` as absent.
fn parse_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

/// Read a date field that `EconDB` renders as a JSON string (or, defensively, a
/// number such as a year). Returns `None` when absent / blank.
fn parse_date(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(_)) => parse_string(value),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

/// Read the series metadata from the response root object.
///
/// `geography` (falling back to `country`) populates `MacroSeries.country` so
/// cross-country series stay queryable; `description` becomes the row title.
fn read_meta(root: &serde_json::Map<String, Value>) -> SeriesMeta {
    SeriesMeta {
        title: parse_string(root.get("description")),
        country: parse_string(root.get("geography")).or_else(|| parse_string(root.get("country"))),
        frequency: parse_string(root.get("frequency")),
        unit: parse_string(root.get("units")).or_else(|| parse_string(root.get("unit"))),
    }
}

/// Decode the `data` payload into a flat list of observations, tolerating the
/// two documented `EconDB` shapes:
///
/// * an object with parallel `dates: [...]` + `values: [...]` arrays (zipped by
///   index — a missing/short `values` array yields `None` values), or
/// * a list of `{date, value}` records.
///
/// A row is emitted for every dated observation even when its value is absent.
fn decode_observations(data: Option<&Value>) -> Vec<DecodedObservation> {
    match data {
        Some(Value::Object(map)) => decode_parallel_arrays(map),
        Some(Value::Array(items)) => decode_record_list(items),
        _ => Vec::new(),
    }
}

/// Decode the parallel-arrays shape: `{ "dates": [...], "values": [...] }`.
fn decode_parallel_arrays(map: &serde_json::Map<String, Value>) -> Vec<DecodedObservation> {
    let Some(Value::Array(dates)) = map.get("dates") else {
        return Vec::new();
    };
    let values = match map.get("values") {
        Some(Value::Array(values)) => values.as_slice(),
        _ => &[],
    };
    let mut rows = Vec::with_capacity(dates.len());
    for (index, date) in dates.iter().enumerate() {
        let Some(date) = parse_date(Some(date)) else {
            continue;
        };
        rows.push(DecodedObservation {
            date,
            value: parse_value(values.get(index)),
        });
    }
    rows
}

/// Decode the record-list shape: `[ { "date": ..., "value": ... }, ... ]`.
fn decode_record_list(items: &[Value]) -> Vec<DecodedObservation> {
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let Some(record) = item.as_object() else {
            continue;
        };
        let Some(date) = parse_date(record.get("date")) else {
            continue;
        };
        rows.push(DecodedObservation {
            date,
            value: parse_value(record.get("value")),
        });
    }
    rows
}

/// Decode the `EconDB` series response into the series identity, metadata, and a
/// flat list of observations.
///
/// `EconDB` surfaces a query error or an unknown ticker as a JSON object lacking a
/// `data` payload; that decodes to an empty (not malformed) result so an
/// out-of-range window does not error, but a non-object root does.
fn decode_series(
    raw: &Bytes,
    fallback_ticker: &str,
) -> Result<(String, SeriesMeta, Vec<DecodedObservation>)> {
    let root: Value = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("econdb parse_json: {error}")))?;

    let Value::Object(root) = &root else {
        return Err(Error::Provider(
            "econdb parse_json: expected a JSON object envelope".to_string(),
        ));
    };

    let series_id = parse_string(root.get("ticker")).unwrap_or_else(|| fallback_ticker.to_string());
    let meta = read_meta(root);
    let observations = decode_observations(root.get("data"));
    Ok((series_id, meta, observations))
}

/// Build the `EconDB` request query parameters: the `format=json` selector plus
/// the optional API token when `TDW_ECONDB_API_KEY` is set.
fn series_query_params() -> Vec<(String, String)> {
    let mut params = vec![("format".to_string(), "json".to_string())];
    if let Some(token) = optional_api_token() {
        params.push(("token".to_string(), token));
    }
    params
}

/// Read the optional `EconDB` token from the environment, trimming whitespace
/// and treating an empty value as absent.
///
/// Routes through the shared `tdw_core::http_support::read_optional_key` helper
/// so every provider reads credentials through one path (same trim/empty
/// semantics) rather than calling `std::env::var` directly.
fn optional_api_token() -> Option<String> {
    tdw_core::http_support::read_optional_key(API_KEY_ENV)
}

/// Perform an `EconDB` series GET for `ticker` with `params`, returning the
/// raw response bytes.
async fn fetch_series(
    base_url: &str,
    ticker: &str,
    params: &[(String, String)],
    ctx: &str,
) -> Result<Bytes> {
    let endpoint = format!("{}/series/{}/", base_url.trim_end_matches('/'), ticker);
    let client = tdw_core::http_support::build_client(USER_AGENT, "econdb http client build")?;
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
    /// Production `EconDB` series fetcher.
    ///
    /// Resolves the `economy/econdb/series` catalog `command` and normalizes the
    /// caller's `ticker` series observations to [`tdw_domain::MacroSeries`] rows.
    pub EcondbHttpMacroSeriesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<EcondbSeriesQuery, MacroSeries> for EcondbHttpMacroSeriesFetcher {
    const PROVIDER: &'static str = "econdb";
    const ENDPOINT: &'static str = "economy_econdb_series";

    fn transform_query(params: Value) -> Result<EcondbSeriesQuery> {
        EcondbSeriesQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &EcondbSeriesQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_series(
            self.base_url(),
            &query.ticker,
            &series_query_params(),
            "econdb series",
        )
        .await
    }

    fn transform_data(&self, query: &EcondbSeriesQuery, raw: Bytes) -> Result<Vec<MacroSeries>> {
        let endpoint = query.endpoint();
        let limit = query
            .params
            .limit
            .map_or(usize::MAX, |limit| limit as usize);
        let (series_id, meta, observations) = decode_series(&raw, &query.ticker)?;
        let title = meta
            .title
            .or_else(|| Some(endpoint.description.to_string()));
        Ok(observations
            .into_iter()
            .take(limit)
            .map(|observation| MacroSeries {
                series_id: series_id.clone(),
                title: title.clone(),
                date: observation.date,
                value: observation.value,
                country: meta.country.clone(),
                frequency: meta.frequency.clone(),
                unit: meta.unit.clone(),
                transform: None,
            })
            .collect())
    }
}
