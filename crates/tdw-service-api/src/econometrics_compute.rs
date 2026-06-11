//! Compute-route execution for the `econometrics/*` catalog namespace
//! (gap-matrix item **L4.3** wiring; the daemon side of the econometrics
//! analytics crate).
//!
//! An `econometrics/*` route is an
//! [`EndpointKind`](tdw_endpoint_catalog::EndpointKind) `Compute` derivation
//! with no provider candidates. Unlike the single-series technical/quantitative
//! routes, the econometric estimators read their (multi-series) inputs entirely
//! from the typed request params — a response `y` and a design `X` for OLS, a
//! list of columns for the correlation matrix and VIF, a series pair plus a lag
//! for Granger causality — so the compute functions take the raw `params`
//! `Value` directly rather than a pre-extracted value series. This module is
//! pure (params → JSON rows), carrying no policy, I/O, or async.
//!
//! Each estimator is registered once. The registry is the single source of
//! truth for *which* `econometrics/*` routes have a compute implementation; a
//! conformance test in `tdw-service-api` proves it is exactly the set of
//! `econometrics/*` Compute routes the catalog declares.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;

use tdw_analytics_econometrics::cointegration::engle_granger;
use tdw_analytics_econometrics::correlation::{correlation_matrix, vif};
use tdw_analytics_econometrics::granger::granger_causality;
use tdw_analytics_econometrics::ols::{design_from_columns, ols};
use tdw_analytics_econometrics::params::{
    CointegrationParams, ColumnsParams, GrangerParams, OlsParams,
};

/// An econometrics compute implementation: parse the route's typed params from
/// `params` (the series live inside the params) and serialize the result rows.
type ComputeFn = fn(&Value) -> Result<Vec<Value>, Error>;

/// Build the `route -> ComputeFn` registry for every `econometrics/*` estimator.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("econometrics/ols", compute_ols);
    registry.insert(
        "econometrics/correlation_matrix",
        compute_correlation_matrix,
    );
    registry.insert("econometrics/vif", compute_vif);
    registry.insert("econometrics/granger_causality", compute_granger);
    registry.insert("econometrics/cointegration", compute_cointegration);
    registry
}

/// Whether `route` is in the `econometrics/*` namespace.
#[must_use]
pub fn owns_route(route: &str) -> bool {
    route.starts_with("econometrics/")
}

/// Run the compute implementation for an `econometrics/*` `route`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the estimator (mismatched series, rank-deficient design, …).
pub fn run_compute(route: &str, params: &Value) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    compute(params)
}

/// Deserialize an estimator param struct from the request `params`, tolerating
/// the extra routing keys the dispatch envelope carries.
fn parse_params<P: for<'de> Deserialize<'de> + Default>(params: &Value) -> Result<P, Error> {
    if params.is_object() {
        serde_json::from_value(params.clone()).map_err(|error| {
            Error::InvalidQuery(format!("econometrics compute: invalid params: {error}"))
        })
    } else {
        Ok(P::default())
    }
}

fn map_econ_err(error: &tdw_analytics_econometrics::EconometricsError) -> Error {
    Error::InvalidQuery(format!("econometrics compute: {error}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
}

// --- Estimator adapters ------------------------------------------------------

fn compute_ols(params: &Value) -> Result<Vec<Value>, Error> {
    let p: OlsParams = parse_params(params)?;
    let design = design_from_columns(&p.x, p.intercept).map_err(|e| map_econ_err(&e))?;
    let summary = ols(&p.y, &design).map_err(|e| map_econ_err(&e))?;
    Ok(vec![to_value(&summary)?])
}

fn compute_correlation_matrix(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ColumnsParams = parse_params(params)?;
    let matrix = correlation_matrix(&p.columns).map_err(|e| map_econ_err(&e))?;
    // One output row per matrix row, each row a numeric array.
    matrix.iter().map(to_value).collect()
}

fn compute_vif(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ColumnsParams = parse_params(params)?;
    let factors = vif(&p.columns).map_err(|e| map_econ_err(&e))?;
    // A single row carrying the per-column VIF vector.
    Ok(vec![to_value(&factors)?])
}

fn compute_granger(params: &Value) -> Result<Vec<Value>, Error> {
    let p: GrangerParams = parse_params(params)?;
    let result = granger_causality(&p.y, &p.x, p.lag).map_err(|e| map_econ_err(&e))?;
    Ok(vec![to_value(&result)?])
}

fn compute_cointegration(params: &Value) -> Result<Vec<Value>, Error> {
    let p: CointegrationParams = parse_params(params)?;
    let result = engle_granger(&p.y, &p.x).map_err(|e| map_econ_err(&e))?;
    Ok(vec![to_value(&result)?])
}

#[cfg(test)]
mod tests {
    use super::{owns_route, run_compute};
    use serde_json::json;

    #[test]
    fn owns_route_matches_namespace() {
        assert!(owns_route("econometrics/ols"));
        assert!(!owns_route("quantitative/sharpe_ratio"));
    }

    #[test]
    fn run_compute_ols_recovers_a_line() {
        // y = 2 + 3x over x=[1,2,3,4].
        let params = json!({
            "y": [5.0, 8.0, 11.0, 14.0],
            "x": [[1.0, 2.0, 3.0, 4.0]],
            "intercept": true
        });
        let rows = run_compute("econometrics/ols", &params).expect("ols");
        assert_eq!(rows.len(), 1);
        let coeffs = rows[0]["coefficients"].as_array().expect("coefficients");
        assert!((coeffs[0]["estimate"].as_f64().expect("intercept") - 2.0).abs() < 1e-9);
        assert!((coeffs[1]["estimate"].as_f64().expect("slope") - 3.0).abs() < 1e-9);
    }

    #[test]
    fn run_compute_correlation_matrix_emits_rows() {
        let params = json!({ "columns": [[1.0, 2.0, 3.0], [3.0, 2.0, 1.0]] });
        let rows = run_compute("econometrics/correlation_matrix", &params).expect("corr");
        assert_eq!(rows.len(), 2);
        // anti-correlated columns ⇒ off-diagonal -1.
        assert!((rows[0][1].as_f64().expect("r") - -1.0).abs() < 1e-9);
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        assert!(run_compute("econometrics/nope", &json!({})).is_err());
    }

    /// The econometrics compute registry is exactly the catalog's
    /// `econometrics/*` Compute route set — no orphan either way.
    #[test]
    fn registry_matches_catalog_econometrics_routes() {
        use super::compute_registry;
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| {
                e.kind == tdw_endpoint_catalog::EndpointKind::Compute
                    && e.route.starts_with("econometrics/")
            })
            .map(|e| e.route)
            .collect();
        assert_eq!(registry, catalog);
    }
}
