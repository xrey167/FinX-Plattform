//! Real CFTC Socrata HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public CFTC Socrata API
//! (`https://publicreporting.cftc.gov`). The API is keyless; an optional
//! `X-App-Token` (read from `TDW_CFTC_APP_TOKEN`) raises the rate limit and is
//! omitted when the env var is unset. Live calls are gated by `TDW_CFTC_LIVE=1`
//! in the integration tests.
//!
//! The Socrata SODA endpoint returns a bare JSON array of row objects whose
//! values are all JSON strings; the fetchers parse the numeric columns and
//! normalize each row to a standardized [`tdw_domain`] model.

#![cfg(feature = "http")]

use std::collections::BTreeSet;

use tdw_core::http_support::prelude::*;
use tdw_domain::{CommitmentOfTraders, SeriesSearchResult};

use crate::{APP_TOKEN_ENV, BASE_URL, CftcCotQuery};

const USER_AGENT: &str = "tdw-provider-cftc/0.1 (contact@finx.example)";
/// Socrata column carrying the market-and-exchange name.
const MARKET_COLUMN: &str = "market_and_exchange_names";
/// Socrata column carrying the report date (`report_date_as_yyyy_mm_dd`).
const REPORT_DATE_COLUMN: &str = "report_date_as_yyyy_mm_dd";

/// Decode a single Socrata field as an `f64`. Socrata returns every value as a
/// JSON string, but JSON numbers are accepted too. Empty / `"null"` strings are
/// treated as absent.
fn parse_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
                None
            } else {
                trimmed.replace(',', "").parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// Read a Socrata string field, treating empty / `"null"` as absent.
fn parse_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

/// Normalize a Socrata `report_date_as_yyyy_mm_dd` value (an ISO-8601 timestamp
/// such as `"2024-06-04T00:00:00.000"`) to the bare `YYYY-MM-DD` calendar date.
fn parse_report_date(value: Option<&Value>) -> Option<String> {
    let raw = parse_string(value)?;
    let date = raw.split(['T', ' ']).next().unwrap_or(raw.as_str());
    if date.is_empty() {
        None
    } else {
        Some(date.to_string())
    }
}

/// Build the Socrata SODA request query parameters from the query: the
/// `$where` market filter, the `$order` (most-recent first), and `$limit`.
fn soda_query_params(query: &CftcCotQuery) -> Vec<(String, String)> {
    let mut params = Vec::new();
    if let Some(market) = &query.market {
        // Single-quote escaping per SoQL: double any embedded single quote.
        let escaped = market.replace('\'', "''");
        params.push(("$where".to_string(), format!("{MARKET_COLUMN}='{escaped}'")));
    }
    params.push(("$order".to_string(), format!("{REPORT_DATE_COLUMN} DESC")));
    let limit = query.params.limit.unwrap_or(0);
    if limit > 0 {
        params.push(("$limit".to_string(), limit.to_string()));
    }
    params
}

/// Build the Socrata SODA request parameters for the market-discovery query:
/// select the distinct market column, ordered, optionally capped.
fn soda_search_params(query: &CftcCotQuery) -> Vec<(String, String)> {
    let mut params = vec![
        ("$select".to_string(), format!("distinct {MARKET_COLUMN}")),
        ("$order".to_string(), format!("{MARKET_COLUMN} ASC")),
    ];
    let limit = query.params.limit.unwrap_or(0);
    if limit > 0 {
        params.push(("$limit".to_string(), limit.to_string()));
    }
    params
}

/// Perform a Socrata GET for `dataset` with `params`, attaching the optional
/// `X-App-Token` header when `TDW_CFTC_APP_TOKEN` is set, and return the raw
/// response bytes.
async fn fetch_socrata(
    base_url: &str,
    dataset: &str,
    params: &[(String, String)],
    ctx: &str,
) -> Result<Bytes> {
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        dataset.trim_start_matches('/')
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "cftc http client build")?;
    let mut request = client.get(&endpoint).query(params);
    if let Some(token) = optional_app_token() {
        request = request.header("X-App-Token", token);
    }
    let response = request
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

/// Read the optional Socrata `X-App-Token` from the environment, trimming
/// whitespace and treating an empty value as absent.
fn optional_app_token() -> Option<String> {
    std::env::var(APP_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Decode a Socrata response body (a bare JSON array of row objects) into a
/// vector of untyped maps so the all-string columns can be coerced per field.
fn decode_rows(raw: &Bytes, ctx: &str) -> Result<Vec<serde_json::Map<String, Value>>> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?;
    match value {
        Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|item| match item {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .collect()),
        // Socrata surfaces query errors as a JSON object with an `error` flag.
        Value::Object(map) => {
            let message = map
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unexpected object response");
            Err(Error::Provider(format!("{ctx} api error: {message}")))
        }
        _ => Err(Error::Provider(format!(
            "{ctx} parse_json: expected a JSON array of rows"
        ))),
    }
}

// ── COT series fetcher ────────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production CFTC Socrata Commitments-of-Traders fetcher.
    ///
    /// Standardizes `regulators/cftc/cot` to [`tdw_domain::CommitmentOfTraders`]
    /// rows from the CFTC legacy futures-only COT Socrata dataset.
    pub CftcHttpCotFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CftcCotQuery, CommitmentOfTraders> for CftcHttpCotFetcher {
    const PROVIDER: &'static str = "cftc";
    const ENDPOINT: &'static str = "regulators_cftc_cot";

    fn transform_query(params: Value) -> Result<CftcCotQuery> {
        CftcCotQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &CftcCotQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_socrata(
            self.base_url(),
            &query.dataset,
            &soda_query_params(query),
            "cftc cot",
        )
        .await
    }

    fn transform_data(
        &self,
        _query: &CftcCotQuery,
        raw: Bytes,
    ) -> Result<Vec<CommitmentOfTraders>> {
        let records = decode_rows(&raw, "cftc cot")?;
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
            let Some(market) = parse_string(record.get(MARKET_COLUMN)) else {
                continue;
            };
            let Some(report_date) = parse_report_date(record.get(REPORT_DATE_COLUMN)) else {
                continue;
            };
            rows.push(CommitmentOfTraders {
                market,
                report_date,
                open_interest: parse_f64(record.get("open_interest_all")),
                noncommercial_long: parse_f64(record.get("noncomm_positions_long_all")),
                noncommercial_short: parse_f64(record.get("noncomm_positions_short_all")),
                commercial_long: parse_f64(record.get("comm_positions_long_all")),
                commercial_short: parse_f64(record.get("comm_positions_short_all")),
                total_reportable_long: parse_f64(record.get("tot_rept_positions_long_all")),
                total_reportable_short: parse_f64(record.get("tot_rept_positions_short_all")),
                nonreportable_long: parse_f64(record.get("nonrept_positions_long_all")),
                nonreportable_short: parse_f64(record.get("nonrept_positions_short_all")),
            });
        }
        Ok(rows)
    }
}

// ── COT market-discovery fetcher ──────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production CFTC Socrata COT market-discovery fetcher.
    ///
    /// Standardizes `regulators/cftc/cot_search` to [`tdw_domain::SeriesSearchResult`]
    /// rows, one per distinct `market_and_exchange_names` value, so callers can
    /// discover the markets to feed into the `regulators/cftc/cot` series.
    pub CftcHttpCotSearchFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CftcCotQuery, SeriesSearchResult> for CftcHttpCotSearchFetcher {
    const PROVIDER: &'static str = "cftc";
    const ENDPOINT: &'static str = "regulators_cftc_cot_search";

    fn transform_query(params: Value) -> Result<CftcCotQuery> {
        CftcCotQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &CftcCotQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_socrata(
            self.base_url(),
            &query.dataset,
            &soda_search_params(query),
            "cftc cot_search",
        )
        .await
    }

    fn transform_data(&self, _query: &CftcCotQuery, raw: Bytes) -> Result<Vec<SeriesSearchResult>> {
        let records = decode_rows(&raw, "cftc cot_search")?;
        // De-duplicate defensively: `$select distinct` already collapses
        // duplicates, but a non-distinct fallback response must not yield repeats.
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for record in records {
            let Some(market) = parse_string(record.get(MARKET_COLUMN)) else {
                continue;
            };
            if seen.insert(market.clone()) {
                rows.push(SeriesSearchResult {
                    series_id: market,
                    title: None,
                    frequency: Some("Weekly".to_string()),
                    units: None,
                    popularity: None,
                });
            }
        }
        Ok(rows)
    }
}
