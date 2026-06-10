//! `technical/*` catalog routes (gap-matrix item **L4.1**).
//!
//! Every route here is an [`EndpointKind::Compute`] derivation: a technical
//! indicator computed over an OHLCV series rather than fetched from a provider.
//! Per the catalog invariant, Compute routes declare **no** provider candidates
//! (the `xtask catalog-check` and the `fetch_routes_have_candidates_compute_…`
//! unit test enforce this). The params schema is the indicator's typed parameter
//! struct and the model schema is its typed output-row struct, both sourced from
//! the `tdw-analytics-technical` crate so the catalog and the runtime compute
//! implementation share one schema definition.

use schemars::{Schema, schema_for};
use tdw_analytics_technical::indicators::{
    AdxRow, AroonRow, ChannelRow, FisherRow, MacdRow, StochasticRow,
};
use tdw_analytics_technical::params::{
    AdxParams, AroonParams, BollingerParams, CciParams, DonchianParams, FisherParams, HmaParams,
    KeltnerParams, LengthParams, MacdParams, RocParams, StochasticParams,
};

use crate::{CatalogEntry, EndpointKind};

/// One scalar (single `Option<f64>` column) output-row schema: a JSON `number`.
/// Scalar indicators (SMA, RSI, ATR, …) emit a bare nullable float per bar, so
/// their model schema is the primitive `f64` schema.
fn scalar_model() -> Schema {
    schema_for!(f64)
}

// --- Param-schema closures (one per distinct param struct). ------------------

fn length_params() -> Schema {
    schema_for!(LengthParams)
}
fn macd_params() -> Schema {
    schema_for!(MacdParams)
}
fn stochastic_params() -> Schema {
    schema_for!(StochasticParams)
}
fn cci_params() -> Schema {
    schema_for!(CciParams)
}
fn adx_params() -> Schema {
    schema_for!(AdxParams)
}
fn aroon_params() -> Schema {
    schema_for!(AroonParams)
}
fn bollinger_params() -> Schema {
    schema_for!(BollingerParams)
}
fn keltner_params() -> Schema {
    schema_for!(KeltnerParams)
}
fn donchian_params() -> Schema {
    schema_for!(DonchianParams)
}
fn fisher_params() -> Schema {
    schema_for!(FisherParams)
}
fn roc_params() -> Schema {
    schema_for!(RocParams)
}
fn hma_params() -> Schema {
    schema_for!(HmaParams)
}

// --- Multi-line output-row schema closures. ----------------------------------

fn macd_model() -> Schema {
    schema_for!(MacdRow)
}
fn stochastic_model() -> Schema {
    schema_for!(StochasticRow)
}
fn adx_model() -> Schema {
    schema_for!(AdxRow)
}
fn aroon_model() -> Schema {
    schema_for!(AroonRow)
}
fn channel_model() -> Schema {
    schema_for!(ChannelRow)
}
fn fisher_model() -> Schema {
    schema_for!(FisherRow)
}

/// Construct one chartable `technical/*` Compute entry with no candidates.
///
/// Keeps `entries()` under the catalog crate's `too_many_lines = 100` ceiling by
/// hiding the repeated `CatalogEntry { kind: Compute, candidates: &[], … }`
/// boilerplate behind a constructor (the same `flat_entry` pattern the equity
/// router uses for its Fetch rows).
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
        chartable: true,
    }
}

/// Declaration-order spec table for the `technical/*` routes: `(route,
/// params_schema, model, doc)`. Kept as a `const` slice so `entries()` stays a
/// short map (under the catalog crate's `too_many_lines = 100` ceiling) and the
/// per-route boilerplate lives in one place.
type RouteSpec = (&'static str, fn() -> Schema, fn() -> Schema, &'static str);

const ROUTES: &[RouteSpec] = &[
    (
        "technical/sma",
        length_params,
        scalar_model,
        "Simple moving average over the close series.",
    ),
    (
        "technical/ema",
        length_params,
        scalar_model,
        "Exponential moving average (SMA-seeded) over the close series.",
    ),
    (
        "technical/wma",
        length_params,
        scalar_model,
        "Linearly weighted moving average over the close series.",
    ),
    (
        "technical/hma",
        hma_params,
        scalar_model,
        "Hull moving average over the close series.",
    ),
    (
        "technical/macd",
        macd_params,
        macd_model,
        "MACD line, signal, and histogram (12/26/9 defaults).",
    ),
    (
        "technical/rsi",
        length_params,
        scalar_model,
        "Wilder Relative Strength Index over the close series.",
    ),
    (
        "technical/stochastic",
        stochastic_params,
        stochastic_model,
        "Stochastic oscillator %K and %D.",
    ),
    (
        "technical/cci",
        cci_params,
        scalar_model,
        "Commodity Channel Index over the typical price.",
    ),
    (
        "technical/adx",
        adx_params,
        adx_model,
        "Wilder ADX with +DI and -DI directional lines.",
    ),
    (
        "technical/aroon",
        aroon_params,
        aroon_model,
        "Aroon up, down, and oscillator.",
    ),
    (
        "technical/bbands",
        bollinger_params,
        channel_model,
        "Bollinger Bands (upper/middle/lower).",
    ),
    (
        "technical/keltner",
        keltner_params,
        channel_model,
        "Keltner Channels (EMA midline ± ATR multiple).",
    ),
    (
        "technical/donchian",
        donchian_params,
        channel_model,
        "Donchian Channels (rolling high/low extremes).",
    ),
    (
        "technical/atr",
        length_params,
        scalar_model,
        "Wilder Average True Range over the OHLC series.",
    ),
    (
        "technical/obv",
        length_params,
        scalar_model,
        "On-Balance Volume running total.",
    ),
    (
        "technical/ad",
        length_params,
        scalar_model,
        "Chaikin Accumulation/Distribution line.",
    ),
    (
        "technical/vwap",
        length_params,
        scalar_model,
        "Session-anchored Volume-Weighted Average Price.",
    ),
    (
        "technical/fisher",
        fisher_params,
        fisher_model,
        "Ehlers Fisher transform with trigger line.",
    ),
    (
        "technical/roc",
        roc_params,
        scalar_model,
        "Rate of change (percent) over the close series.",
    ),
    (
        "technical/momentum",
        roc_params,
        scalar_model,
        "Momentum (price difference) over the close series.",
    ),
];

/// The `technical` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    ROUTES
        .iter()
        .map(|&(route, params_schema, model, doc)| compute_entry(route, params_schema, model, doc))
        .collect()
}
