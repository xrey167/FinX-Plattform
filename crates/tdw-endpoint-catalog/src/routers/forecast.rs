//! `forecast/*` catalog routes (ecosystem item **G003**).
//!
//! Every route here is an [`EndpointKind::Compute`] derivation: a classical
//! statistical forecast, backtest metric, decomposition, or anomaly scan computed
//! over a caller-supplied time series rather than fetched from a provider. Per the
//! catalog invariant, Compute routes declare **no** provider candidates. The
//! params schema is the model's typed parameter struct and the model schema its
//! typed output, both sourced from the `tdw-analytics-forecast` crate so the
//! catalog and the runtime compute implementation share one schema definition.
//!
//! # Input convention
//!
//! Like the econometric routes (and unlike the single-series technical/quant
//! routes), the forecasting routes carry their inputs inline in the typed params
//! (the historical series `y`, the horizon, the seasonal period, the smoothing
//! knobs), so these routes are inline-only — they do not support the nested
//! `source` price-fetch form. Chartability is `false`: the outputs are forecast
//! point lists, metric scalars, decompositions, and anomaly flags, not per-bar
//! series.

use schemars::{Schema, schema_for};
use tdw_analytics_forecast::params::{
    AnomalyParams, AnomalyResult, ArimaParams, AutoArimaParams, BacktestParams, BaselineParams,
    Decomposition, EtsParams, ExpoParams, ForecastResult, LinRegrParams, MetricsParams, MstlParams,
    PrecisionMetrics, ThetaParams,
};

use crate::{CatalogEntry, EndpointKind};

// --- Param-schema closures. --------------------------------------------------

fn baseline_params() -> Schema {
    schema_for!(BaselineParams)
}
fn expo_params() -> Schema {
    schema_for!(ExpoParams)
}
fn ets_params() -> Schema {
    schema_for!(EtsParams)
}
fn theta_params() -> Schema {
    schema_for!(ThetaParams)
}
fn mstl_params() -> Schema {
    schema_for!(MstlParams)
}
fn linregr_params() -> Schema {
    schema_for!(LinRegrParams)
}
fn arima_params() -> Schema {
    schema_for!(ArimaParams)
}
fn autoarima_params() -> Schema {
    schema_for!(AutoArimaParams)
}
fn backtest_params() -> Schema {
    schema_for!(BacktestParams)
}
fn metrics_params() -> Schema {
    schema_for!(MetricsParams)
}
fn anomaly_params() -> Schema {
    schema_for!(AnomalyParams)
}

// --- Output-model schema closures. -------------------------------------------

fn forecast_model() -> Schema {
    schema_for!(ForecastResult)
}
fn decomposition_model() -> Schema {
    schema_for!(Decomposition)
}
fn metrics_model() -> Schema {
    schema_for!(PrecisionMetrics)
}
fn anomaly_model() -> Schema {
    schema_for!(AnomalyResult)
}

/// Construct one non-chartable `forecast/*` Compute entry with no candidates.
const fn compute_entry(
    route: &'static str,
    params_schema: fn() -> Schema,
    model: fn() -> Schema,
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Compute,
        params_schema,
        model,
        candidates: &[],
        bronze_table: None,
        doc,
        chartable: false,
    }
}

/// Declaration-order spec table for the `forecast/*` routes.
type RouteSpec = (&'static str, fn() -> Schema, fn() -> Schema, &'static str);

const ROUTES: &[RouteSpec] = &[
    (
        "forecast/seasonalnaive",
        baseline_params,
        forecast_model,
        "Seasonal-naive baseline: tile the last full season forward over the horizon.",
    ),
    (
        "forecast/rwd",
        baseline_params,
        forecast_model,
        "Random-walk-with-drift baseline: extrapolate the mean first difference.",
    ),
    (
        "forecast/expo",
        expo_params,
        forecast_model,
        "Holt-Winters exponential smoothing (level + trend + optional additive seasonal).",
    ),
    (
        "forecast/ets",
        ets_params,
        forecast_model,
        "ETS additive model selection over the level/trend/seasonal variants by one-step SSE.",
    ),
    (
        "forecast/theta",
        theta_params,
        forecast_model,
        "The Theta method (theta = 2): a strong classical baseline carrying a regression drift.",
    ),
    (
        "forecast/mstl",
        mstl_params,
        decomposition_model,
        "Additive moving-average seasonal-trend decomposition (trend + seasonal + residual).",
    ),
    (
        "forecast/linregr",
        linregr_params,
        forecast_model,
        "Linear-regression forecast with autoregressive lag features and optional covariates.",
    ),
    (
        "forecast/arima",
        arima_params,
        forecast_model,
        "ARIMA(p, d, q) forecast estimated by Hannan-Rissanen two-stage least squares.",
    ),
    (
        "forecast/autoarima",
        autoarima_params,
        forecast_model,
        "AutoARIMA: stepwise Hyndman-Khandakar order selection over (p, d, q) by information criterion.",
    ),
    (
        "forecast/backtest",
        backtest_params,
        metrics_model,
        "Expanding-window backtest of a forecast model scored with RMSE/MAE/MAPE/SMAPE.",
    ),
    (
        "forecast/metrics",
        metrics_params,
        metrics_model,
        "Precision metrics RMSE/MAE/MAPE/SMAPE over an aligned actual/forecast pair.",
    ),
    (
        "forecast/anomaly",
        anomaly_params,
        anomaly_model,
        "Quantile-band anomaly detection: flag points outside the empirical quantile band.",
    ),
];

/// The `forecast` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    ROUTES
        .iter()
        .map(|&(route, params_schema, model, doc)| compute_entry(route, params_schema, model, doc))
        .collect()
}
