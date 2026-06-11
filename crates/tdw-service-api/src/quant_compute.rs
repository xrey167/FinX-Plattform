//! Compute-route execution for the `quantitative/*` catalog namespace
//! (gap-matrix item **L4.2** wiring; the daemon side of the quant analytics
//! crate).
//!
//! A `quantitative/*` route is an [`EndpointKind`](tdw_endpoint_catalog::EndpointKind)
//! `Compute` derivation with no provider candidates. Instead of fetching, it
//! runs a pure metric from `tdw-analytics-quant` over a **returns** series the
//! dispatcher resolves (from an inline `data` array or a nested `source` price
//! fetch) and hands here already reduced to returns. This module is pure
//! (returns + params → a single JSON figure), so it carries no policy, I/O, or
//! async.
//!
//! Each metric is registered once. The registry is the single source of truth
//! for *which* `quantitative/*` routes have a compute implementation; a
//! conformance test in `tdw-service-api` proves it is exactly the set of
//! `quantitative/*` Compute routes the catalog declares, and the tool-registry
//! wiring registers one `RegisteredTool` per registry entry.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;

use tdw_analytics_quant::metrics::{
    capm, expected_shortfall, jarque_bera, kurtosis, max_drawdown, omega_ratio, sharpe_ratio, skew,
    sortino_ratio, value_at_risk, volatility,
};
use tdw_analytics_quant::params::{
    CapmParams, OmegaParams, SharpeParams, SortinoParams, VarParams, VolatilityParams,
};

/// A quantitative compute implementation: parse the route's typed params from
/// `params`, run the metric over the returns `values`, and serialize the single
/// output figure (a scalar or a typed row) to JSON.
type ComputeFn = fn(&[f64], &Value) -> Result<Value, Error>;

/// Build the `route -> ComputeFn` registry for every `quantitative/*` metric.
///
/// `BTreeMap` keeps iteration deterministic (used by the conformance test and
/// the tool-registry wiring). Routes match the catalog's `quantitative/*`
/// entries.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("quantitative/sharpe_ratio", compute_sharpe);
    registry.insert("quantitative/sortino_ratio", compute_sortino);
    registry.insert("quantitative/omega_ratio", compute_omega);
    registry.insert("quantitative/max_drawdown", compute_max_drawdown);
    registry.insert("quantitative/calmar_ratio", compute_calmar);
    registry.insert("quantitative/volatility", compute_volatility);
    registry.insert("quantitative/skewness", compute_skewness);
    registry.insert("quantitative/kurtosis", compute_kurtosis);
    registry.insert("quantitative/value_at_risk", compute_value_at_risk);
    registry.insert(
        "quantitative/expected_shortfall",
        compute_expected_shortfall,
    );
    registry.insert("quantitative/capm", compute_capm);
    registry.insert("quantitative/jarque_bera", compute_jarque_bera);
    registry
}

/// Whether `route` is in the `quantitative/*` namespace.
#[must_use]
pub fn owns_route(route: &str) -> bool {
    route.starts_with("quantitative/")
}

/// Run the compute implementation for a `quantitative/*` `route` over the
/// returns `values`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the metric (e.g. a non-positive annualization factor or a
/// mismatched benchmark length).
pub fn run_compute(route: &str, values: &[f64], params: &Value) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    let figure = compute(values, params)?;
    Ok(vec![figure])
}

/// A flexible inbound value record. Accepts a bare JSON number, a `{date?,
/// value}` object (the `MacroSeries`/`RateObservation` single-series shape), or
/// an OHLCV record (from which the `close` is taken), so a hand-rolled inline
/// `data` array and a nested fetch result both parse without the caller
/// reshaping.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum InboundValue {
    /// A bare number — a raw return (or price, if the caller passes prices).
    Scalar(f64),
    /// An object carrying the load-bearing numeric field under one of the
    /// common keys.
    Object(ValueObject),
}

#[derive(Debug, Deserialize)]
struct ValueObject {
    #[serde(default)]
    value: Option<f64>,
    #[serde(default)]
    close: Option<f64>,
}

impl InboundValue {
    fn as_number(&self) -> Option<f64> {
        match self {
            Self::Scalar(v) => Some(*v),
            Self::Object(object) => object.value.or(object.close),
        }
    }
}

/// Parse a JSON array into a value series of `f64`.
///
/// Each element may be a bare number, a `{value}` / `{close}` object, or an
/// OHLCV row (its `close` is used). This is the *raw value* series — the
/// dispatcher decides whether to treat it as returns directly or reduce a price
/// series to returns first.
///
/// # Errors
///
/// Returns [`Error::InvalidQuery`] when `records` is not an array or any element
/// carries no readable numeric value.
pub fn parse_values(records: &Value) -> Result<Vec<f64>, Error> {
    let array = records.as_array().ok_or_else(|| {
        Error::InvalidQuery(
            "quantitative compute: `data` must be an array of numbers or {value} records"
                .to_string(),
        )
    })?;
    let mut values = Vec::with_capacity(array.len());
    for (index, record) in array.iter().enumerate() {
        let inbound: InboundValue = serde_json::from_value(record.clone()).map_err(|error| {
            Error::InvalidQuery(format!(
                "quantitative compute: row {index} is not a number or value record: {error}"
            ))
        })?;
        let number = inbound.as_number().ok_or_else(|| {
            Error::InvalidQuery(format!(
                "quantitative compute: row {index} carries no numeric value/close field"
            ))
        })?;
        values.push(number);
    }
    Ok(values)
}

/// Deserialize a metric param struct from the request `params`, tolerating the
/// extra routing keys (`provider`, `data`, `source`) the dispatch envelope
/// carries by deserializing leniently when the params are an object.
fn parse_params<P: for<'de> Deserialize<'de> + Default>(params: &Value) -> Result<P, Error> {
    if params.is_object() {
        serde_json::from_value(params.clone()).map_err(|error| {
            Error::InvalidQuery(format!("quantitative compute: invalid params: {error}"))
        })
    } else {
        Ok(P::default())
    }
}

fn map_quant_err(error: &tdw_analytics_quant::QuantError) -> Error {
    Error::InvalidQuery(format!("quantitative compute: {error}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
}

// --- Metric adapters ---------------------------------------------------------

fn compute_sharpe(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: SharpeParams = parse_params(params)?;
    let figure = sharpe_ratio(values, p).map_err(|e| map_quant_err(&e))?;
    to_value(&figure)
}

fn compute_sortino(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: SortinoParams = parse_params(params)?;
    let figure = sortino_ratio(values, p).map_err(|e| map_quant_err(&e))?;
    to_value(&figure)
}

fn compute_omega(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: OmegaParams = parse_params(params)?;
    to_value(&omega_ratio(values, p))
}

fn compute_max_drawdown(values: &[f64], _params: &Value) -> Result<Value, Error> {
    to_value(&max_drawdown(values))
}

fn compute_calmar(values: &[f64], _params: &Value) -> Result<Value, Error> {
    // The Calmar ratio is reported alongside the drawdown it derives from; the
    // route returns the same drawdown row (carrying `calmar_ratio`).
    to_value(&max_drawdown(values))
}

fn compute_volatility(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: VolatilityParams = parse_params(params)?;
    let figure = volatility(values, p).map_err(|e| map_quant_err(&e))?;
    to_value(&figure)
}

fn compute_skewness(values: &[f64], _params: &Value) -> Result<Value, Error> {
    to_value(&skew(values))
}

fn compute_kurtosis(values: &[f64], _params: &Value) -> Result<Value, Error> {
    to_value(&kurtosis(values))
}

fn compute_value_at_risk(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: VarParams = parse_params(params)?;
    to_value(&value_at_risk(values, p))
}

fn compute_expected_shortfall(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: VarParams = parse_params(params)?;
    to_value(&expected_shortfall(values, p))
}

fn compute_jarque_bera(values: &[f64], _params: &Value) -> Result<Value, Error> {
    to_value(&jarque_bera(values))
}

/// CAPM needs the benchmark series inline in `params.benchmark`; it is a
/// paired-series route and so does not support the nested `source` form.
fn compute_capm(values: &[f64], params: &Value) -> Result<Value, Error> {
    let p: CapmParams = parse_params(params)?;
    let benchmark = params.get("benchmark").ok_or_else(|| {
        Error::InvalidQuery(
            "quantitative compute: capm requires an inline `benchmark` returns array".to_string(),
        )
    })?;
    let benchmark_values = parse_values(benchmark)?;
    let row = capm(values, &benchmark_values, p).map_err(|e| map_quant_err(&e))?;
    to_value(&row)
}

#[cfg(test)]
mod tests {
    use super::{owns_route, parse_values, run_compute};
    use serde_json::json;

    #[test]
    fn owns_route_matches_namespace() {
        assert!(owns_route("quantitative/sharpe_ratio"));
        assert!(!owns_route("technical/sma"));
        assert!(!owns_route("econometrics/ols"));
    }

    #[test]
    fn parse_values_accepts_numbers_objects_and_ohlcv() {
        let numbers = parse_values(&json!([0.1, -0.05, 0.1])).expect("numbers");
        assert_eq!(numbers, vec![0.1, -0.05, 0.1]);
        let objects = parse_values(&json!([{ "value": 0.2 }, { "close": 0.3 }])).expect("objects");
        assert_eq!(objects, vec![0.2, 0.3]);
    }

    #[test]
    fn run_compute_sharpe_returns_single_figure() {
        let values = [0.10, -0.05, 0.10, -0.05];
        let rows = run_compute("quantitative/sharpe_ratio", &values, &json!({})).expect("sharpe");
        assert_eq!(rows.len(), 1);
        assert!((rows[0].as_f64().expect("f64") - 0.288_675_134_594_8).abs() < 1e-9);
    }

    #[test]
    fn run_compute_capm_requires_benchmark() {
        let values = [0.1, 0.2, 0.3, 0.4];
        let err = run_compute("quantitative/capm", &values, &json!({})).expect_err("err");
        assert!(
            matches!(err, tdw_core::Error::InvalidQuery(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        assert!(run_compute("quantitative/nope", &[0.1], &json!({})).is_err());
    }

    /// The quant compute registry is exactly the catalog's `quantitative/*`
    /// Compute route set — no orphan route, no orphan implementation.
    #[test]
    fn registry_matches_catalog_quantitative_routes() {
        use super::compute_registry;
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| {
                e.kind == tdw_endpoint_catalog::EndpointKind::Compute
                    && e.route.starts_with("quantitative/")
            })
            .map(|e| e.route)
            .collect();
        assert_eq!(registry, catalog);
    }
}
