//! `econometrics/*` catalog routes (gap-matrix item **L4.3**).
//!
//! Every route here is an [`EndpointKind::Compute`] derivation: a regression or
//! econometric test computed over caller-supplied series rather than fetched
//! from a provider. Per the catalog invariant, Compute routes declare **no**
//! provider candidates. The params schema is the estimator's typed parameter
//! struct and the model schema its typed output, both sourced from the
//! `tdw-analytics-econometrics` crate so the catalog and the runtime compute
//! implementation share one schema definition.
//!
//! # Input convention
//!
//! Unlike the single-series technical/quantitative routes, the econometric
//! routes need richer multi-series inputs (a response `y` and a design `X` for
//! OLS, a list of columns for the correlation matrix and VIF, a pair of series
//! plus a lag for Granger causality). Those inputs are carried **inline in the
//! params** (the typed param structs), so these routes are inline-only — they do
//! not support the nested `source` price-fetch form, which yields a single
//! series. Chartability is `false`: the outputs are coefficient tables and test
//! statistics, not per-bar series.

use schemars::{Schema, schema_for};
use tdw_analytics_econometrics::cointegration::CointegrationResult;
use tdw_analytics_econometrics::diagnostics::ResidualAutocorrelationResult;
use tdw_analytics_econometrics::granger::GrangerResult;
use tdw_analytics_econometrics::ols::{AutocorrelationResult, OlsCoefficients, OlsSummary};
use tdw_analytics_econometrics::panel::PanelSummary;
use tdw_analytics_econometrics::params::{
    CointegrationParams, ColumnsParams, GrangerParams, OlsParams, PanelParams,
    ResidualAutocorrelationParams,
};

use crate::{CatalogEntry, EndpointKind};

// --- Param-schema closures. --------------------------------------------------

fn ols_params() -> Schema {
    schema_for!(OlsParams)
}
fn columns_params() -> Schema {
    schema_for!(ColumnsParams)
}
fn granger_params() -> Schema {
    schema_for!(GrangerParams)
}
fn cointegration_params() -> Schema {
    schema_for!(CointegrationParams)
}
fn residual_autocorrelation_params() -> Schema {
    schema_for!(ResidualAutocorrelationParams)
}
fn panel_params() -> Schema {
    schema_for!(PanelParams)
}

// --- Output-model schema closures. -------------------------------------------

fn ols_model() -> Schema {
    schema_for!(OlsSummary)
}
fn granger_model() -> Schema {
    schema_for!(GrangerResult)
}
fn cointegration_model() -> Schema {
    schema_for!(CointegrationResult)
}
fn ols_coefficients_model() -> Schema {
    schema_for!(OlsCoefficients)
}
fn autocorrelation_model() -> Schema {
    schema_for!(AutocorrelationResult)
}
fn residual_autocorrelation_model() -> Schema {
    schema_for!(ResidualAutocorrelationResult)
}
fn panel_model() -> Schema {
    schema_for!(PanelSummary)
}

/// Model schema for a route whose row is a numeric vector — the correlation
/// matrix emits one such row per input column, the VIF route one scalar per
/// column. A JSON array of numbers describes both.
fn numeric_row_model() -> Schema {
    schema_for!(Vec<f64>)
}

/// Construct one non-chartable `econometrics/*` Compute entry with no
/// candidates.
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

/// Declaration-order spec table for the `econometrics/*` routes.
type RouteSpec = (&'static str, fn() -> Schema, fn() -> Schema, &'static str);

const ROUTES: &[RouteSpec] = &[
    (
        "econometrics/ols",
        ols_params,
        ols_model,
        "Ordinary least squares: coefficients, std errors, t-stats, R2, F, DW.",
    ),
    (
        "econometrics/correlation_matrix",
        columns_params,
        numeric_row_model,
        "Pearson correlation matrix of a set of equal-length columns.",
    ),
    (
        "econometrics/vif",
        columns_params,
        numeric_row_model,
        "Variance inflation factors via auxiliary-regression R-squared.",
    ),
    (
        "econometrics/granger_causality",
        granger_params,
        granger_model,
        "Granger-causality F-test of x-lags improving the prediction of y.",
    ),
    (
        "econometrics/cointegration",
        cointegration_params,
        cointegration_model,
        "Engle-Granger step one plus a residual stationarity score.",
    ),
    (
        "econometrics/ols_regression",
        ols_params,
        ols_coefficients_model,
        "Ordinary least squares: the estimated coefficient table only.",
    ),
    (
        "econometrics/ols_regression_summary",
        ols_params,
        ols_model,
        "Ordinary least squares: the full summary (coefficients plus R2, F, DW).",
    ),
    (
        "econometrics/autocorrelation",
        ols_params,
        autocorrelation_model,
        "Durbin-Watson residual-autocorrelation statistic of an OLS fit.",
    ),
    (
        "econometrics/residual_autocorrelation",
        residual_autocorrelation_params,
        residual_autocorrelation_model,
        "Breusch-Godfrey LM test for residual serial correlation.",
    ),
    (
        "econometrics/panel_pooled",
        panel_params,
        panel_model,
        "Pooled OLS panel estimator (ignores the entity structure).",
    ),
    (
        "econometrics/panel_fixed",
        panel_params,
        panel_model,
        "One-way fixed-effects (within) panel estimator.",
    ),
    (
        "econometrics/panel_between",
        panel_params,
        panel_model,
        "Between panel estimator (OLS on the entity means).",
    ),
    (
        "econometrics/panel_first_difference",
        panel_params,
        panel_model,
        "First-difference panel estimator (OLS on within-entity differences).",
    ),
    (
        "econometrics/panel_random_effects",
        panel_params,
        panel_model,
        "Swamy-Arora random-effects (feasible-GLS) panel estimator.",
    ),
    (
        "econometrics/panel_fmac",
        panel_params,
        panel_model,
        "Fama-MacBeth two-pass panel estimator (per-period cross-sections averaged).",
    ),
];

/// The `econometrics` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    ROUTES
        .iter()
        .map(|&(route, params_schema, model, doc)| compute_entry(route, params_schema, model, doc))
        .collect()
}
