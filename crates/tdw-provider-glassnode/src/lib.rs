#![forbid(unsafe_code)]

//! Glassnode on-chain analytics provider.
//!
//! Core types, validation, and a mock fetcher live here (always compiled).
//! The real HTTP backend is in [`http_fetcher`], gated by the `http` feature.

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::GlassnodeHttpFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "glassnode";
pub const BASE_URL: &str = "https://api.glassnode.com/v1";
pub const API_KEY_PARAM: &str = "api_key";
pub const API_KEY_ENV: &str = "TDW_GLASSNODE_API_KEY";

pub type Result<T> = std::result::Result<T, GlassnodeProviderError>;

/// Selects which Glassnode on-chain metric to fetch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GlassnodeMetric {
    /// MVRV Z-Score — `/metrics/market/mvrv_z_score`.
    MvrvZScore,
    /// Long-term holder supply — `/metrics/supply/lth_sum`.
    LthSupply,
    /// Net unrealised profit/loss — `/metrics/indicators/nupl`.
    Nupl,
}

impl GlassnodeMetric {
    /// Returns the API path segment for this metric.
    #[must_use]
    pub fn api_path(&self) -> &'static str {
        match self {
            Self::MvrvZScore => "/metrics/market/mvrv_z_score",
            Self::LthSupply => "/metrics/supply/lth_sum",
            Self::Nupl => "/metrics/indicators/nupl",
        }
    }
}

/// Query parameters for a Glassnode metric request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GlassnodeMetricQuery {
    /// Ticker symbol, e.g. `"BTC"`.
    pub asset: String,
    /// Which metric to fetch.
    pub metric: GlassnodeMetric,
    /// Resolution interval, e.g. `"24h"`.
    pub interval: String,
}

impl GlassnodeMetricQuery {
    /// Construct a validated query.
    ///
    /// # Errors
    ///
    /// Returns [`GlassnodeProviderError::EmptyAsset`] when `asset` is blank,
    /// or [`GlassnodeProviderError::EmptyInterval`] when `interval` is blank.
    pub fn new(
        asset: impl Into<String>,
        metric: GlassnodeMetric,
        interval: impl Into<String>,
    ) -> Result<Self> {
        let asset = asset.into();
        let asset = asset.trim().to_ascii_uppercase();
        if asset.is_empty() {
            return Err(GlassnodeProviderError::EmptyAsset);
        }
        let interval = interval.into();
        let interval = interval.trim().to_string();
        if interval.is_empty() {
            return Err(GlassnodeProviderError::EmptyInterval);
        }
        Ok(Self {
            asset,
            metric,
            interval,
        })
    }
}

/// A single data point returned by the Glassnode API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GlassnodeDataPoint {
    /// Unix timestamp (seconds).
    pub timestamp: i64,
    /// Metric value.
    pub value: f64,
    /// Asset the metric was requested for.
    pub asset: String,
    /// Metric identifier.
    pub metric: GlassnodeMetric,
}

/// Errors produced by this provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GlassnodeProviderError {
    #[error("glassnode asset must not be empty")]
    EmptyAsset,
    #[error("glassnode interval must not be empty")]
    EmptyInterval,
    #[error("glassnode api key env TDW_GLASSNODE_API_KEY must be set")]
    MissingApiKey,
    #[error("glassnode provider error: {0}")]
    Provider(String),
}

/// Returns a stub data-point for offline / unit-test use.
///
/// # Errors
///
/// Propagates any validation error from [`GlassnodeMetricQuery::new`].
pub fn mock_fetch(query: &GlassnodeMetricQuery) -> Result<Vec<GlassnodeDataPoint>> {
    Ok(vec![GlassnodeDataPoint {
        timestamp: 1_704_153_600,
        value: 1.85,
        asset: query.asset.clone(),
        metric: query.metric.clone(),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_normalises_asset_to_uppercase() {
        let q = GlassnodeMetricQuery::new("btc", GlassnodeMetric::MvrvZScore, "24h")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        assert_eq!(q.asset, "BTC");
        assert_eq!(q.interval, "24h");
    }

    #[test]
    fn query_rejects_empty_asset() {
        let err = GlassnodeMetricQuery::new("  ", GlassnodeMetric::Nupl, "24h")
            .expect_err("empty asset must be rejected");
        assert_eq!(err, GlassnodeProviderError::EmptyAsset);
    }

    #[test]
    fn query_rejects_empty_interval() {
        let err = GlassnodeMetricQuery::new("BTC", GlassnodeMetric::LthSupply, "")
            .expect_err("empty interval must be rejected");
        assert_eq!(err, GlassnodeProviderError::EmptyInterval);
    }

    #[test]
    fn metric_api_paths_are_correct() {
        assert_eq!(
            GlassnodeMetric::MvrvZScore.api_path(),
            "/metrics/market/mvrv_z_score"
        );
        assert_eq!(
            GlassnodeMetric::LthSupply.api_path(),
            "/metrics/supply/lth_sum"
        );
        assert_eq!(GlassnodeMetric::Nupl.api_path(), "/metrics/indicators/nupl");
    }

    #[test]
    fn mock_fetch_returns_stub_data_point() {
        let q = GlassnodeMetricQuery::new("BTC", GlassnodeMetric::MvrvZScore, "24h")
            .unwrap_or_else(|e| panic!("query must build: {e}"));
        let rows = mock_fetch(&q).unwrap_or_else(|e| panic!("mock_fetch must succeed: {e}"));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].timestamp, 1_704_153_600);
        assert_eq!(rows[0].asset, "BTC");
    }
}
