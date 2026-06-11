//! `portfolio/*` catalog routes (gap-matrix item **L4.4**).
//!
//! Every route here is an [`EndpointKind::Compute`] derivation: a portfolio
//! metric computed over caller-supplied returns / equity / weights series rather
//! than fetched from a provider. Per the catalog invariant, Compute routes
//! declare **no** provider candidates. The params schema is the metric's typed
//! parameter struct and the model schema its typed output (a numeric vector or a
//! typed row), both sourced from the `tdw-analytics-portfolio` crate so the
//! catalog and the runtime compute implementation share one schema definition.
//!
//! # Input convention
//!
//! Like the `econometrics/*` routes, the portfolio metrics carry their inputs
//! **inline in the params** (a returns series, an equity / cumulative-return
//! curve, a vector of position values, or an aligned weights+returns pair), so
//! these routes are inline-only — they do not support the nested `source`
//! price-fetch form. Chartability is `false`: a normalized weight vector or a
//! drawdown summary is not a per-bar OHLCV series.

use schemars::{Schema, schema_for};
use tdw_analytics_portfolio::metrics::MaxDrawdownRow;
use tdw_analytics_portfolio::params::{
    AllocationParams, ContributionParams, EquityParams, ReturnsParams,
};

use crate::{CatalogEntry, EndpointKind};

// --- Param-schema closures. --------------------------------------------------

fn returns_params() -> Schema {
    schema_for!(ReturnsParams)
}
fn equity_params() -> Schema {
    schema_for!(EquityParams)
}
fn allocation_params() -> Schema {
    schema_for!(AllocationParams)
}
fn contribution_params() -> Schema {
    schema_for!(ContributionParams)
}

// --- Output-model schema closures. -------------------------------------------

/// Model schema for a route whose row is a numeric vector — `cumulative_returns`
/// and `drawdown` emit one value per input step, `allocation` one weight per
/// position, `contribution` one figure per asset. A JSON array of numbers
/// describes all four.
fn numeric_row_model() -> Schema {
    schema_for!(Vec<f64>)
}

fn max_drawdown_model() -> Schema {
    schema_for!(MaxDrawdownRow)
}

/// Construct one non-chartable `portfolio/*` Compute entry with no candidates.
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

/// Declaration-order spec table for the `portfolio/*` routes.
type RouteSpec = (&'static str, fn() -> Schema, fn() -> Schema, &'static str);

const ROUTES: &[RouteSpec] = &[
    (
        "portfolio/cumulative_returns",
        returns_params,
        numeric_row_model,
        "Running compounded cumulative-return series from periodic returns.",
    ),
    (
        "portfolio/drawdown",
        equity_params,
        numeric_row_model,
        "Per-step peak-to-trough drawdown series of an equity curve.",
    ),
    (
        "portfolio/max_drawdown",
        equity_params,
        max_drawdown_model,
        "Worst drawdown magnitude over an equity curve, with peak/trough indices.",
    ),
    (
        "portfolio/allocation",
        allocation_params,
        numeric_row_model,
        "Normalize position values to portfolio weights summing to one.",
    ),
    (
        "portfolio/contribution",
        contribution_params,
        numeric_row_model,
        "Per-asset return contribution: weight times return for each asset.",
    ),
];

/// The `portfolio` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    ROUTES
        .iter()
        .map(|&(route, params_schema, model, doc)| compute_entry(route, params_schema, model, doc))
        .collect()
}
