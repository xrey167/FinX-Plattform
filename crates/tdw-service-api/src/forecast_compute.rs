//! Compute-route execution for the `forecast/*` catalog namespace (ecosystem
//! item **G003** wiring; the daemon side of the classical-forecasting analytics
//! crate).
//!
//! A `forecast/*` route is an
//! [`EndpointKind`](tdw_endpoint_catalog::EndpointKind) `Compute` derivation with
//! no provider candidates. Like the `econometrics/*` and `derivatives/pricing/*`
//! routes (and unlike the single-series technical/quantitative routes), the
//! forecasters read their inputs entirely from the typed request params — the
//! historical series `y`, the horizon, the seasonal period, the smoothing knobs —
//! so the compute functions take the raw `params` `Value` directly rather than a
//! pre-extracted value series. This module is pure (params -> JSON rows), carrying
//! no policy, I/O, or async; every model is deterministic from its inputs.
//!
//! Each model is registered once. The registry is the single source of truth for
//! *which* `forecast/*` routes have a compute implementation; a conformance test
//! in `tdw-service-api` proves it is exactly the set of `forecast/*` Compute
//! routes the catalog declares.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;

use tdw_analytics_forecast::params::{
    AnomalyParams, BacktestParams, BaselineParams, EtsParams, ExpoParams, LinRegrParams,
    MetricsParams, MstlParams, ThetaParams,
};
use tdw_analytics_forecast::{anomaly, backtest, baseline, expo, linregr, metrics, mstl, theta};

/// A forecasting compute implementation: parse the route's typed params from
/// `params` (the series live inside the params) and serialize the result rows.
type ComputeFn = fn(&Value) -> Result<Vec<Value>, Error>;

/// Build the `route -> ComputeFn` registry for every `forecast/*` model.
///
/// `BTreeMap` keeps iteration deterministic (used by the conformance test).
/// Routes match the catalog's `forecast/*` entries.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("forecast/seasonalnaive", compute_seasonalnaive);
    registry.insert("forecast/rwd", compute_rwd);
    registry.insert("forecast/expo", compute_expo);
    registry.insert("forecast/ets", compute_ets);
    registry.insert("forecast/theta", compute_theta);
    registry.insert("forecast/mstl", compute_mstl);
    registry.insert("forecast/linregr", compute_linregr);
    registry.insert("forecast/backtest", compute_backtest);
    registry.insert("forecast/metrics", compute_metrics);
    registry.insert("forecast/anomaly", compute_anomaly);
    registry
}

/// Whether `route` is in the `forecast/*` namespace.
#[must_use]
pub fn owns_route(route: &str) -> bool {
    route.starts_with("forecast/")
}

/// Run the compute implementation for a `forecast/*` `route`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the model (an empty/too-short series, a zero horizon, an
/// out-of-range coefficient, …).
pub fn run_compute(route: &str, params: &Value) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    compute(params)
}

/// Deserialize a model param struct from the request `params`, tolerating the
/// extra routing keys the dispatch envelope carries. Every forecast param struct
/// is all-optional (has a `Default`), so a non-object body yields the default.
fn parse_params<P: for<'de> Deserialize<'de> + Default>(params: &Value) -> Result<P, Error> {
    if params.is_object() {
        serde_json::from_value(params.clone()).map_err(|error| {
            Error::InvalidQuery(format!("forecast compute: invalid params: {error}"))
        })
    } else {
        Ok(P::default())
    }
}

fn map_forecast_err(error: &tdw_analytics_forecast::ForecastError) -> Error {
    Error::InvalidQuery(format!("forecast compute: {error}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
}

// --- Model adapters ----------------------------------------------------------

fn compute_seasonalnaive(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BaselineParams = parse_params(params)?;
    let result = baseline::seasonal_naive_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_rwd(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BaselineParams = parse_params(params)?;
    let result = baseline::rwd_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_expo(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ExpoParams = parse_params(params)?;
    let result = expo::expo_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_ets(params: &Value) -> Result<Vec<Value>, Error> {
    let p: EtsParams = parse_params(params)?;
    let result = expo::ets_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_theta(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ThetaParams = parse_params(params)?;
    let result = theta::theta_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_mstl(params: &Value) -> Result<Vec<Value>, Error> {
    let p: MstlParams = parse_params(params)?;
    let result = mstl::mstl_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_linregr(params: &Value) -> Result<Vec<Value>, Error> {
    let p: LinRegrParams = parse_params(params)?;
    let result = linregr::linregr_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_backtest(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BacktestParams = parse_params(params)?;
    let result = backtest::backtest_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_metrics(params: &Value) -> Result<Vec<Value>, Error> {
    let p: MetricsParams = parse_params(params)?;
    let result = metrics::metrics_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_anomaly(params: &Value) -> Result<Vec<Value>, Error> {
    let p: AnomalyParams = parse_params(params)?;
    let result = anomaly::anomaly_from_params(&p).map_err(|e| map_forecast_err(&e))?;
    Ok(vec![to_value(&result)?])
}

#[cfg(test)]
mod tests {
    use super::{owns_route, run_compute};
    use serde_json::json;

    #[test]
    fn owns_route_matches_namespace() {
        assert!(owns_route("forecast/seasonalnaive"));
        assert!(!owns_route("econometrics/ols"));
        assert!(!owns_route("quantitative/sharpe_ratio"));
    }

    #[test]
    fn run_compute_seasonalnaive_reproduces_period() {
        let params = json!({
            "y": [1.0, 2.0, 3.0, 4.0, 10.0, 20.0, 30.0, 40.0],
            "horizon": 4,
            "period": 4
        });
        let rows = run_compute("forecast/seasonalnaive", &params).expect("seasonalnaive");
        assert_eq!(rows.len(), 1);
        let points = rows[0]["points"].as_array().expect("points");
        assert!((points[0]["value"].as_f64().expect("v") - 10.0).abs() < 1e-9);
        assert!((points[3]["value"].as_f64().expect("v") - 40.0).abs() < 1e-9);
    }

    #[test]
    fn run_compute_rwd_drift_equals_mean_first_difference() {
        let params = json!({ "y": [2.0, 4.0, 6.0, 8.0, 10.0], "horizon": 3 });
        let rows = run_compute("forecast/rwd", &params).expect("rwd");
        let points = rows[0]["points"].as_array().expect("points");
        // drift 2 ⇒ 12, 14, 16.
        assert!((points[0]["value"].as_f64().expect("v") - 12.0).abs() < 1e-9);
        assert!((points[2]["value"].as_f64().expect("v") - 16.0).abs() < 1e-9);
    }

    #[test]
    fn run_compute_metrics_hand_calculation() {
        let params = json!({
            "actual": [100.0, 200.0, 300.0],
            "forecast": [110.0, 190.0, 330.0]
        });
        let rows = run_compute("forecast/metrics", &params).expect("metrics");
        let rmse = rows[0]["rmse"].as_f64().expect("rmse");
        assert!(
            (rmse - (1100.0_f64 / 3.0).sqrt()).abs() < 1e-9,
            "rmse {rmse}"
        );
        let mae = rows[0]["mae"].as_f64().expect("mae");
        assert!((mae - 50.0 / 3.0).abs() < 1e-9, "mae {mae}");
    }

    #[test]
    fn run_compute_anomaly_flags_a_spike() {
        let params = json!({
            "y": [10.0, 11.0, 9.0, 10.0, 11.0, 9.0, 10.0, 11.0, 9.0, 10.0, 100.0],
            "lower_quantile": 0.05,
            "upper_quantile": 0.95
        });
        let rows = run_compute("forecast/anomaly", &params).expect("anomaly");
        let anomalies = rows[0]["anomalies"].as_array().expect("anomalies");
        assert!(
            anomalies.iter().any(|a| a["index"].as_u64() == Some(10)),
            "spike at index 10 must be flagged"
        );
    }

    #[test]
    fn run_compute_invalid_params_errors() {
        // A seasonal-naive request with period 1 is rejected as a structured
        // error (no seasonal structure).
        let bad = json!({ "y": [1.0, 2.0, 3.0], "horizon": 1, "period": 1 });
        let err = run_compute("forecast/seasonalnaive", &bad).expect_err("err");
        assert!(
            matches!(err, tdw_core::Error::InvalidQuery(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        assert!(run_compute("forecast/nope", &json!({})).is_err());
    }

    /// The forecast compute registry is exactly the catalog's `forecast/*`
    /// Compute route set — no orphan route, no orphan implementation.
    #[test]
    fn registry_matches_catalog_forecast_routes() {
        use super::compute_registry;
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| {
                e.kind == tdw_endpoint_catalog::EndpointKind::Compute
                    && e.route.starts_with("forecast/")
            })
            .map(|e| e.route)
            .collect();
        assert_eq!(registry, catalog);
    }
}
