//! `quantitative/*` catalog routes (gap-matrix item **L4.2**).
//!
//! Every route here is an [`EndpointKind::Compute`] derivation: a returns-based
//! quantitative metric computed over a value series rather than fetched from a
//! provider. Per the catalog invariant, Compute routes declare **no** provider
//! candidates. The params schema is the metric's typed parameter struct and the
//! model schema is its typed output (a scalar `f64` or a typed row struct), both
//! sourced from the `tdw-analytics-quant` crate so the catalog and the runtime
//! compute implementation share one schema definition.
//!
//! # Input convention and `source` support
//!
//! Each metric consumes a **returns** series (see `tdw-analytics-quant`'s
//! crate-level docs). The dispatcher extracts that series from either an inline
//! `data` array (numbers, `{date?, value}` objects, or OHLCV rows reduced to
//! close-to-close returns) or a nested `source` fetch of a price-series route
//! (e.g. `equity/price/historical`), whose close column is reduced to returns.
//! The two paired-series routes — `capm` (asset vs `benchmark`) — take their
//! second series inline via the params and are therefore inline-`data` only;
//! every single-series route additionally supports the nested `source` form.

use schemars::{Schema, schema_for};
use tdw_analytics_quant::metrics::{CapmRow, DrawdownRow, JarqueBeraRow};
use tdw_analytics_quant::params::{
    CapmParams, NoParams, OmegaParams, SharpeParams, SortinoParams, VarParams, VolatilityParams,
};

use crate::{CatalogEntry, EndpointKind};

/// One scalar (single `f64` figure) output-row schema: a JSON `number`. Scalar
/// metrics (Sharpe, Sortino, volatility, …) summarize the series into a bare
/// float, so their model schema is the primitive `f64` schema.
fn scalar_model() -> Schema {
    schema_for!(f64)
}

// --- Param-schema closures (one per distinct param struct). ------------------

fn sharpe_params() -> Schema {
    schema_for!(SharpeParams)
}
fn sortino_params() -> Schema {
    schema_for!(SortinoParams)
}
fn omega_params() -> Schema {
    schema_for!(OmegaParams)
}
fn volatility_params() -> Schema {
    schema_for!(VolatilityParams)
}
fn var_params() -> Schema {
    schema_for!(VarParams)
}
fn capm_params() -> Schema {
    schema_for!(CapmParams)
}
fn no_params() -> Schema {
    schema_for!(NoParams)
}

// --- Multi-field output-row schema closures. ---------------------------------

fn capm_model() -> Schema {
    schema_for!(CapmRow)
}
fn drawdown_model() -> Schema {
    schema_for!(DrawdownRow)
}
fn jarque_bera_model() -> Schema {
    schema_for!(JarqueBeraRow)
}

/// Construct one non-chartable `quantitative/*` Compute entry with no
/// candidates. Quantitative metrics summarize a whole series into a single
/// figure, so there is nothing per-bar to chart — `chartable` is `false`.
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

/// Declaration-order spec table for the `quantitative/*` routes: `(route,
/// params_schema, model, doc)`. Kept as a `const` slice so `entries()` stays a
/// short map and the per-route boilerplate lives in one place.
type RouteSpec = (&'static str, fn() -> Schema, fn() -> Schema, &'static str);

const ROUTES: &[RouteSpec] = &[
    (
        "quantitative/sharpe_ratio",
        sharpe_params,
        scalar_model,
        "Sharpe ratio of a returns series (rf, annualization periods).",
    ),
    (
        "quantitative/sortino_ratio",
        sortino_params,
        scalar_model,
        "Sortino ratio: excess return over downside deviation below a target.",
    ),
    (
        "quantitative/omega_ratio",
        omega_params,
        scalar_model,
        "Omega ratio: gains-above-threshold over losses-below-threshold mass.",
    ),
    (
        "quantitative/max_drawdown",
        no_params,
        drawdown_model,
        "Maximum peak-to-trough drawdown (and the derived Calmar ratio).",
    ),
    (
        "quantitative/calmar_ratio",
        no_params,
        drawdown_model,
        "Calmar ratio: mean return over the absolute maximum drawdown.",
    ),
    (
        "quantitative/volatility",
        volatility_params,
        scalar_model,
        "Annualized volatility (standard deviation of returns scaled by periods).",
    ),
    (
        "quantitative/skewness",
        no_params,
        scalar_model,
        "Adjusted Fisher-Pearson sample skewness of the returns.",
    ),
    (
        "quantitative/kurtosis",
        no_params,
        scalar_model,
        "Bias-corrected sample excess kurtosis of the returns.",
    ),
    (
        "quantitative/value_at_risk",
        var_params,
        scalar_model,
        "Historical Value-at-Risk at the given confidence level.",
    ),
    (
        "quantitative/expected_shortfall",
        var_params,
        scalar_model,
        "Expected shortfall (conditional VaR): mean loss beyond the VaR cutoff.",
    ),
    (
        "quantitative/capm",
        capm_params,
        capm_model,
        "CAPM alpha and beta of the asset returns versus a benchmark series.",
    ),
    (
        "quantitative/jarque_bera",
        no_params,
        jarque_bera_model,
        "Jarque-Bera normality statistic and its chi-squared p-value.",
    ),
];

/// The `quantitative` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    ROUTES
        .iter()
        .map(|&(route, params_schema, model, doc)| compute_entry(route, params_schema, model, doc))
        .collect()
}
