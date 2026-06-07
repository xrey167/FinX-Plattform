//! Real Polygon aggregates backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Polygon's stock aggregates
//! endpoint directly via `reqwest`. Live calls require
//! `POLYGON_API_KEY`; the live integration test is additionally gated
//! by `TDW_POLYGON_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, PolygonAggregatesQuery, aggregates_request};

const API_KEY_ENV: &str = "POLYGON_API_KEY";
const USER_AGENT: &str = "tdw-provider-polygon/0.1";

#[derive(Deserialize)]
struct PolygonEnvelope {
    #[serde(default)]
    ticker: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    results: Vec<PolygonAggregate>,
}

#[derive(Deserialize)]
struct PolygonAggregate {
    #[serde(rename = "o")]
    open: f64,
    #[serde(rename = "h")]
    high: f64,
    #[serde(rename = "l")]
    low: f64,
    #[serde(rename = "c")]
    close: f64,
    #[serde(rename = "v")]
    volume: f64,
    #[serde(rename = "t")]
    timestamp_ms: i64,
}

/// Provider specification for the Polygon aggregate-bars fetcher.
pub struct PolygonAggregatesSpec;

impl ProviderSpec for PolygonAggregatesSpec {
    const PROVIDER: &'static str = "polygon";
    const ENDPOINT: &'static str = "aggregates";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "polygon client";
    const SEND_ERR: &'static str = "polygon extract_data";
    const RETURNED_ERR: &'static str = "polygon extract_data returned";
    const READ_BODY_ERR: &'static str = "polygon read body";

    type Query = PolygonAggregatesQuery;
    type Data = MarketDataBar;

    fn transform_query(params: Value) -> Result<PolygonAggregatesQuery> {
        let ticker = params
            .get("ticker")
            .or_else(|| params.get("symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("polygon ticker must be a string".to_string()))?;
        let from = params
            .get("from")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("polygon from must be YYYY-MM-DD".to_string()))?;
        let to = params
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("polygon to must be YYYY-MM-DD".to_string()))?;
        let adjusted = params
            .get("adjusted")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|error| Error::InvalidQuery(format!("polygon limit too large: {error}")))?
            .unwrap_or(5_000);

        PolygonAggregatesQuery::new(ticker, from, to)
            .and_then(|query| query.with_limit(limit))
            .map(|query| query.with_adjusted(adjusted))
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &PolygonAggregatesQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        aggregates_request(&query.ticker, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "polygon")?;
        let endpoint = format!(
            "{}/v2/aggs/ticker/{}/range/1/day/{}/{}",
            base_url.trim_end_matches('/'),
            query.ticker,
            query.from,
            query.to
        );
        let query_params = [
            ("adjusted", query.adjusted.to_string()),
            ("sort", "asc".to_string()),
            ("limit", query.limit.to_string()),
            ("apiKey", api_key),
        ];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &PolygonAggregatesQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: PolygonEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("polygon parse_json: {error}")))?;
        if envelope
            .status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("ERROR"))
        {
            let message = envelope.error.or(envelope.message).unwrap_or_default();
            return Err(Error::Provider(format!("polygon api error: {message}")));
        }
        let symbol = envelope.ticker.unwrap_or_else(|| query.ticker.clone());
        let mut rows = Vec::with_capacity(envelope.results.len());
        for aggregate in envelope.results {
            rows.push(MarketDataBar {
                symbol: symbol.clone(),
                venue: "polygon".to_string(),
                granularity: TimeGranularity::Day,
                ts: unix_millis_to_iso_timestamp(aggregate.timestamp_ms),
                open: aggregate.open,
                high: aggregate.high,
                low: aggregate.low,
                close: aggregate.close,
                volume: aggregate.volume,
                source: "polygon".to_string(),
            });
        }
        Ok(rows)
    }
}

/// Production Polygon aggregate-bars fetcher.
pub type PolygonHttpAggregatesFetcher = HttpFetcher<PolygonAggregatesSpec>;

fn unix_millis_to_iso_timestamp(timestamp_millis: i64) -> String {
    let seconds = timestamp_millis.div_euclid(1_000);
    let days_since_epoch = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_millis_to_iso_timestamp_matches_well_known_dates() {
        assert_eq!(unix_millis_to_iso_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            unix_millis_to_iso_timestamp(1_704_153_600_000),
            "2024-01-02T00:00:00Z"
        );
        assert_eq!(
            unix_millis_to_iso_timestamp(-86_400_000),
            "1969-12-31T00:00:00Z"
        );
    }
}
