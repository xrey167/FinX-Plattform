//! Compute-route execution for the `derivatives/pricing/*` catalog namespace
//! (ecosystem item **G001** wiring; the daemon side of the option-pricing
//! crate).
//!
//! A `derivatives/pricing/*` route is an
//! [`EndpointKind`](tdw_endpoint_catalog::EndpointKind) `Compute` derivation with
//! no provider candidates. Like the `econometrics/*` and `portfolio/*` routes
//! (and unlike the single-series technical/quantitative routes), the pricing
//! models read their inputs entirely from the typed request params — the
//! contract terms (spot, strike, rate, dividend yield, volatility, time to
//! expiry) plus model knobs — so the compute functions take the raw `params`
//! `Value` directly rather than a pre-extracted value series. This module is pure
//! (params -> JSON rows), carrying no policy, I/O, or async; the Monte-Carlo
//! route is deterministic given its `seed` param.
//!
//! Each model is registered once. The registry is the single source of truth for
//! *which* `derivatives/pricing/*` routes have a compute implementation; a
//! conformance test in `tdw-service-api` proves it is exactly the set of
//! `derivatives/pricing/*` Compute routes the catalog declares.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use tdw_core::Error;

use tdw_quant_options::params::{
    BinomialParams, BlackScholesParams, ImpliedVolParams, MonteCarloParams,
};
use tdw_quant_options::{binomial, black_scholes, implied_vol, monte_carlo};

/// An option-pricing compute implementation: parse the route's typed params from
/// `params` (the contract terms live inside the params) and serialize the result
/// rows.
type ComputeFn = fn(&Value) -> Result<Vec<Value>, Error>;

/// Build the `route -> ComputeFn` registry for every `derivatives/pricing/*`
/// model.
///
/// `BTreeMap` keeps iteration deterministic (used by the conformance test).
/// Routes match the catalog's `derivatives/pricing/*` entries.
#[must_use]
pub fn compute_registry() -> BTreeMap<&'static str, ComputeFn> {
    let mut registry: BTreeMap<&'static str, ComputeFn> = BTreeMap::new();
    registry.insert("derivatives/pricing/black_scholes", compute_black_scholes);
    registry.insert("derivatives/pricing/greeks", compute_greeks);
    registry.insert(
        "derivatives/pricing/implied_volatility",
        compute_implied_volatility,
    );
    registry.insert("derivatives/pricing/binomial", compute_binomial);
    registry.insert("derivatives/pricing/monte_carlo", compute_monte_carlo);
    registry
}

/// Whether `route` is in the `derivatives/pricing/*` namespace.
#[must_use]
pub fn owns_route(route: &str) -> bool {
    route.starts_with("derivatives/pricing/")
}

/// Run the compute implementation for a `derivatives/pricing/*` `route`.
///
/// # Errors
///
/// Returns [`Error::Provider`] when `route` has no registered compute
/// implementation, and [`Error::InvalidQuery`] when the supplied params are
/// invalid for the model (a non-positive contract input, a zero step/path count,
/// or an implied-volatility price outside the attainable range).
pub fn run_compute(route: &str, params: &Value) -> Result<Vec<Value>, Error> {
    let registry = compute_registry();
    let Some(compute) = registry.get(route) else {
        return Err(Error::Provider(format!(
            "no compute implementation registered for catalog route {route}"
        )));
    };
    compute(params)
}

/// Deserialize a model param struct from the request `params`. The pricing
/// params have no all-optional default, so (unlike the analytics routes) a
/// missing required field is a structured `InvalidQuery` error rather than a
/// silent default.
fn parse_params<P: for<'de> Deserialize<'de>>(params: &Value) -> Result<P, Error> {
    serde_json::from_value(params.clone())
        .map_err(|error| Error::InvalidQuery(format!("option pricing: invalid params: {error}")))
}

fn map_option_err(error: &tdw_quant_options::OptionError) -> Error {
    Error::InvalidQuery(format!("option pricing: {error}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::Provider(format!("compute serialize: {e}")))
}

// --- Model adapters ----------------------------------------------------------

fn compute_black_scholes(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BlackScholesParams = parse_params(params)?;
    let price = black_scholes::price(p).map_err(|e| map_option_err(&e))?;
    Ok(vec![to_value(&price)?])
}

fn compute_greeks(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BlackScholesParams = parse_params(params)?;
    let greeks = black_scholes::greeks(p).map_err(|e| map_option_err(&e))?;
    Ok(vec![to_value(&greeks)?])
}

fn compute_implied_volatility(params: &Value) -> Result<Vec<Value>, Error> {
    let p: ImpliedVolParams = parse_params(params)?;
    let vol = implied_vol::implied_volatility(p).map_err(|e| map_option_err(&e))?;
    Ok(vec![to_value(&vol)?])
}

fn compute_binomial(params: &Value) -> Result<Vec<Value>, Error> {
    let p: BinomialParams = parse_params(params)?;
    let price = binomial::price(p).map_err(|e| map_option_err(&e))?;
    Ok(vec![to_value(&price)?])
}

fn compute_monte_carlo(params: &Value) -> Result<Vec<Value>, Error> {
    let p: MonteCarloParams = parse_params(params)?;
    let estimate = monte_carlo::price(p).map_err(|e| map_option_err(&e))?;
    Ok(vec![to_value(&estimate)?])
}

#[cfg(test)]
mod tests {
    use super::{owns_route, run_compute};
    use serde_json::json;

    fn base() -> serde_json::Value {
        json!({
            "spot": 100.0,
            "strike": 100.0,
            "rate": 0.05,
            "volatility": 0.2,
            "time_to_expiry": 1.0
        })
    }

    #[test]
    fn owns_route_matches_namespace() {
        assert!(owns_route("derivatives/pricing/black_scholes"));
        assert!(!owns_route("derivatives/options/chains"));
        assert!(!owns_route("quantitative/sharpe_ratio"));
    }

    #[test]
    fn run_compute_black_scholes_matches_textbook() {
        let rows = run_compute("derivatives/pricing/black_scholes", &base()).expect("bs");
        assert_eq!(rows.len(), 1);
        let price = rows[0].as_f64().expect("f64");
        assert!((price - 10.4506).abs() < 1e-3, "price {price}");
    }

    #[test]
    fn run_compute_greeks_returns_row() {
        let rows = run_compute("derivatives/pricing/greeks", &base()).expect("greeks");
        assert_eq!(rows.len(), 1);
        let delta = rows[0]["delta"].as_f64().expect("delta");
        assert!(delta > 0.0 && delta < 1.0, "call delta {delta}");
    }

    #[test]
    fn run_compute_implied_volatility_round_trips() {
        // Price at sigma=0.2, then invert: should recover ~0.2.
        let mut iv = base();
        iv["market_price"] = json!(10.450_583_572);
        iv.as_object_mut().expect("obj").remove("volatility");
        let rows = run_compute("derivatives/pricing/implied_volatility", &iv).expect("iv");
        let vol = rows[0].as_f64().expect("f64");
        assert!((vol - 0.2).abs() < 1e-4, "iv {vol}");
    }

    #[test]
    fn run_compute_monte_carlo_is_seeded_and_near_bs() {
        let mut mc = base();
        mc["paths"] = json!(50_000);
        mc["seed"] = json!(7);
        let a = run_compute("derivatives/pricing/monte_carlo", &mc).expect("mc a");
        let b = run_compute("derivatives/pricing/monte_carlo", &mc).expect("mc b");
        assert_eq!(a, b, "same seed must reproduce the same estimate");
        let price = a[0]["price"].as_f64().expect("price");
        assert!((price - 10.4506).abs() < 0.2, "mc {price}");
    }

    #[test]
    fn run_compute_binomial_converges_to_bs() {
        let mut bin = base();
        bin["steps"] = json!(500);
        let rows = run_compute("derivatives/pricing/binomial", &bin).expect("bin");
        let price = rows[0].as_f64().expect("f64");
        assert!((price - 10.4506).abs() < 0.05, "binomial {price}");
    }

    #[test]
    fn run_compute_invalid_params_errors() {
        // Missing a required field (`volatility`) is a structured error.
        let bad = json!({ "spot": 100.0, "strike": 100.0, "rate": 0.05, "time_to_expiry": 1.0 });
        let err = run_compute("derivatives/pricing/black_scholes", &bad).expect_err("err");
        assert!(
            matches!(err, tdw_core::Error::InvalidQuery(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn run_compute_unknown_route_errors() {
        assert!(run_compute("derivatives/pricing/nope", &base()).is_err());
    }

    /// The option-pricing compute registry is exactly the catalog's
    /// `derivatives/pricing/*` Compute route set — no orphan route, no orphan
    /// implementation.
    #[test]
    fn registry_matches_catalog_pricing_routes() {
        use super::compute_registry;
        use std::collections::BTreeSet;
        let registry: BTreeSet<&str> = compute_registry().keys().copied().collect();
        let catalog: BTreeSet<&str> = tdw_endpoint_catalog::catalog()
            .iter()
            .filter(|e| {
                e.kind == tdw_endpoint_catalog::EndpointKind::Compute
                    && e.route.starts_with("derivatives/pricing/")
            })
            .map(|e| e.route)
            .collect();
        assert_eq!(registry, catalog);
    }
}
