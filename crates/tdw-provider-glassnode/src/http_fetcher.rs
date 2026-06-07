#![cfg(feature = "http")]
//! Real Glassnode HTTP backend implementing [`tdw_core::Fetcher`].
//!
//! Gated by the `http` feature. Reads `TDW_GLASSNODE_API_KEY` from the
//! environment and appends it as the `api_key` query parameter. Live
//! integration tests are additionally gated by `TDW_GLASSNODE_LIVE=1`.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{API_KEY_ENV, BASE_URL, GlassnodeDataPoint, GlassnodeMetric, GlassnodeMetricQuery};

const USER_AGENT: &str = "tdw-provider-glassnode/0.1";

/// Raw API response element — `[{"t": <unix>, "v": <f64>}]`.
#[derive(Deserialize)]
struct RawPoint {
    t: i64,
    v: f64,
}

/// Provider specification for the Glassnode metric fetcher.
pub struct GlassnodeSpec;

impl ProviderSpec for GlassnodeSpec {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "metric";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "glassnode client";
    const SEND_ERR: &'static str = "glassnode extract_data";
    const RETURNED_ERR: &'static str = "glassnode extract_data returned";
    const READ_BODY_ERR: &'static str = "glassnode read body";

    type Query = GlassnodeMetricQuery;
    type Data = GlassnodeDataPoint;

    fn transform_query(params: Value) -> Result<GlassnodeMetricQuery> {
        let asset = params
            .get("asset")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("glassnode asset must be a string".to_string()))?;
        let interval = params
            .get("interval")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("glassnode interval must be a string".to_string())
            })?;
        let metric: GlassnodeMetric = params
            .get("metric")
            .ok_or_else(|| Error::InvalidQuery("glassnode metric is required".to_string()))
            .and_then(|value| {
                serde_json::from_value(value.clone())
                    .map_err(|error| Error::InvalidQuery(format!("glassnode metric: {error}")))
            })?;
        GlassnodeMetricQuery::new(asset, metric, interval)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &GlassnodeMetricQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "glassnode")?;

        let url = format!(
            "{}{}",
            base_url.trim_end_matches('/'),
            query.metric.api_path()
        );

        Ok(client.get(&url).query(&[
            ("a", query.asset.as_str()),
            ("i", query.interval.as_str()),
            ("api_key", api_key.as_str()),
        ]))
    }

    fn transform_data(query: &GlassnodeMetricQuery, raw: Bytes) -> Result<Vec<GlassnodeDataPoint>> {
        let points: Vec<RawPoint> = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("glassnode json parse: {error}")))?;

        let rows = points
            .into_iter()
            .map(|point| GlassnodeDataPoint {
                timestamp: point.t,
                value: point.v,
                asset: query.asset.clone(),
                metric: query.metric.clone(),
            })
            .collect();

        Ok(rows)
    }
}

/// Production Glassnode metric fetcher.
pub type GlassnodeHttpFetcher = HttpFetcher<GlassnodeSpec>;
