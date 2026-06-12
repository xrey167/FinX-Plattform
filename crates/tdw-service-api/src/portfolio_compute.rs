//! Compute-route execution for the `portfolio/*` catalog namespace (gap-matrix
//! item **L4.4** wiring; the daemon side of the portfolio analytics crate).
//!
//! A `portfolio/*` route is an
//! [`EndpointKind`](tdw_endpoint_catalog::EndpointKind) `Compute` derivation with
//! no provider candidates. Like the `econometrics/*` routes (and unlike the
//! single-series technical/quantitative routes), the portfolio metrics read their
//! inputs entirely from the typed request params — a returns series for
//! `cumulative_returns`, an equity / cumulative-return curve for `drawdown` and
//! `max_drawdown`, a vector of position values for `allocation`, and an aligned
//! `weights`+`returns` pair for `contribution` — so the compute functions take
//! the raw `params` `Value` directly rather than a pre-extracted value series.
//! This module is pure (params → JSON rows), carrying no policy, I/O, or async.
//!
//! Each metric is registered once. The registry is the single source of truth
//! for *which* `portfolio/*` routes have a compute implementation; a conformance
//! test in `tdw-service-api` proves it is exactly the set of `portfolio/*`
//! Compute routes the catalog declares.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;

use tdw_analytics_portfolio::metrics::{
    allocation, contribution, cumulative_returns, drawdown, max_drawdown,
};
use tdw_analytics_portfolio::params::{
    AllocationParams, ContributionParams, EquityParams, ReturnsParams,
};

/// A portfolio compute implementation: parse the route's typed params from
/// `params` (the series live inside the params) and serialize the result rows.
type ComputeFn = fn(&Value) -> Result<Vec<Value>, Error>;

/// Build the `route -> ComputeFn` registry for every `portfolio/*` metric.
///
/// `BTreeMap` keeps iteration deterministic (used by the conformance test).
/// Routes match the catalog's `portfolio/*` entries.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("portfolio/cumulative_returns", compute_cumulative_returns);
    registry.insert("portfolio/drawdown", compute_drawdown);
    registry.insert("portfolio/max_drawdown", compute_max_drawdown);
    registry.insert("portfolio/allocation", compute_allocation);
    registry.insert("portfolio/contribution", compute_contribution);
    registry
}

/// Whether `route` is in the `portfolio/*` namespace.
#[must_use]
pub fn owns_route(route: &str) -> bool {
    route.starts_with("portfolio/")
}

/// Run the compute implementation for a `portfolio/*` `route`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the metric (mismatched weights/returns lengths, a zero allocation
/// total).
pub fn run_compute(route: &str, params: &Value) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    compute(params)
}

/// Deserialize a metric param struct from the request `params`, tolerating the
/// extra routing keys the dispatch envelope carries by deserializing leniently
/// when the params are an object.
fn parse_params<P: for<'de> Deserialize<'de> + Default>(params: &Value) -> Result<P, Error> {
    if params.is_object() {
        serde_json::from_value(params.clone()).map_err(|error| {
            Error::InvalidQuery(format!("portfolio compute: invalid params: {error}"))
        })
    } else {
        Ok(P::default())
    }
}

fn map_portfolio_err(error: &tdw_analytics_portfolio::PortfolioError) -> Error {
    Error::InvalidQuery(format!("portfolio compute: {error}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
}

// --- Metric adapters ---------------------------------------------------------

fn compute_cumulative_returns(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ReturnsParams = parse_params(params)?;
    Ok(vec![to_value(&cumulative_returns(&p.returns))?])
}

fn compute_drawdown(params: &Value) -> Result<Vec<Value>, Error> {
    let p: EquityParams = parse_params(params)?;
    Ok(vec![to_value(&drawdown(&p.equity))?])
}

fn compute_max_drawdown(params: &Value) -> Result<Vec<Value>, Error> {
    let p: EquityParams = parse_params(params)?;
    Ok(vec![to_value(&max_drawdown(&p.equity))?])
}

fn compute_allocation(params: &Value) -> Result<Vec<Value>, Error> {
    let p: AllocationParams = parse_params(params)?;
    let weights = allocation(&p.values).map_err(|e| map_portfolio_err(&e))?;
    Ok(vec![to_value(&weights)?])
}

fn compute_contribution(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ContributionParams = parse_params(params)?;
    let figures = contribution(&p.weights, &p.returns).map_err(|e| map_portfolio_err(&e))?;
    Ok(vec![to_value(&figures)?])
}

#[cfg(test)]
mod tests {
    use super::{owns_route, run_compute};
    use serde_json::json;

    #[test]
    fn owns_route_matches_namespace() {
        assert!(owns_route("portfolio/cumulative_returns"));
        assert!(!owns_route("quantitative/sharpe_ratio"));
        assert!(!owns_route("technical/sma"));
    }

    #[test]
    fn run_compute_cumulative_returns_compounds() {
        let rows = run_compute(
            "portfolio/cumulative_returns",
            &json!({ "returns": [0.10, -0.05, 0.10] }),
        )
        .expect("cumulative");
        assert_eq!(rows.len(), 1);
        let series = rows[0].as_array().expect("array");
        assert_eq!(series.len(), 3);
        assert!((series[2].as_f64().expect("f64") - 0.1495).abs() < 1e-9);
    }

    #[test]
    fn run_compute_max_drawdown_returns_row() {
        let rows = run_compute(
            "portfolio/max_drawdown",
            &json!({ "equity": [1.0, 1.10, 1.045, 1.1495, 1.092_025] }),
        )
        .expect("max_drawdown");
        assert_eq!(rows.len(), 1);
        assert!((rows[0]["max_drawdown"].as_f64().expect("f64") - 0.05).abs() < 1e-9);
        assert_eq!(rows[0]["peak_index"].as_u64(), Some(1));
        assert_eq!(rows[0]["trough_index"].as_u64(), Some(2));
    }

    #[test]
    fn run_compute_contribution_length_mismatch_errors() {
        let err = run_compute(
            "portfolio/contribution",
            &json!({ "weights": [0.5, 0.5], "returns": [0.1] }),
        )
        .expect_err("err");
        assert!(
            matches!(err, tdw_core::Error::InvalidQuery(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        assert!(run_compute("portfolio/nope", &json!({})).is_err());
    }

    /// The portfolio compute registry is exactly the catalog's `portfolio/*`
    /// Compute route set — no orphan route, no orphan implementation.
    #[test]
    fn registry_matches_catalog_portfolio_routes() {
        use super::compute_registry;
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| {
                e.kind == tdw_endpoint_catalog::EndpointKind::Compute
                    && e.route.starts_with("portfolio/")
            })
            .map(|e| e.route)
            .collect();
        assert_eq!(registry, catalog);
    }
}
