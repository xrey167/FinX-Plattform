//! `derivatives/*` catalog routes.
//!
//! The keyless Yahoo derivatives cluster (gap-matrix item L2.4): options chains,
//! futures historical bars (via continuation symbols), and the futures forward
//! curve. Each route's single candidate endpoint key matches the Yahoo fetcher's
//! `ENDPOINT` const — also the runtime dispatch-table key — and a conformance
//! test in tdw-service-api keeps these rows and that table in sync. Adding a
//! route is an append to [`entries`], never a new wiring point in
//! [`crate::catalog`].

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{EquityHistoricalData, FuturesCurvePoint, FuturesInstrument, OptionContract};
use tdw_quant_options::params::{
    BinomialParams, BlackScholesParams, Greeks, ImpliedVolParams, MonteCarloParams, MonteCarloPrice,
};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

// Multi-provider route: Yahoo (keyless, offline-first fixture) leads so an
// unkeyed default build resolves network-free, exactly as the legacy resolver's
// keyless-first convention dictates; CBOE follows as a second keyless source
// (CBOE's public CDN also needs no key, but Yahoo already ships the recorded
// fixture, so it stays first by reliability/coverage). Both endpoint keys match
// each fetcher's `ENDPOINT` const.
const DERIVATIVES_OPTIONS_CHAINS: &[ProviderCandidate] = &[
    ProviderCandidate::new("yahoo", "options_chains"),
    ProviderCandidate::new("cboe", "options_chains"),
];
const DERIVATIVES_FUTURES_HISTORICAL: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "futures_historical")];
const DERIVATIVES_FUTURES_CURVE: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "futures_curve")];
// Keyless Deribit futures-instrument routes (openbb-parity P4W7). Both reuse
// Deribit's public `/public/get_instruments` endpoint; the endpoint keys match
// each catalog-facing fetcher's `ENDPOINT` const and runtime dispatch key.
const DERIVATIVES_FUTURES_INSTRUMENTS: &[ProviderCandidate] =
    &[ProviderCandidate::new("deribit", "futures_instruments")];
const DERIVATIVES_FUTURES_INFO: &[ProviderCandidate] =
    &[ProviderCandidate::new("deribit", "futures_info")];
// Intrinio keyed options cluster (openbb-parity total wave G002): unusual
// activity, market snapshots, and the IV-surface chain inputs. Each route's sole
// candidate endpoint key matches the Intrinio fetcher's `ENDPOINT` const — also
// the runtime dispatch-table key. All keyed (Intrinio only; live calls require
// the PAID INTRINIO_API_KEY).
const DERIVATIVES_OPTIONS_UNUSUAL: &[ProviderCandidate] = &[ProviderCandidate::new(
    "intrinio",
    "derivatives_options_unusual",
)];
const DERIVATIVES_OPTIONS_SNAPSHOTS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "intrinio",
    "derivatives_options_snapshots",
)];
const DERIVATIVES_OPTIONS_SURFACE: &[ProviderCandidate] = &[ProviderCandidate::new(
    "intrinio",
    "derivatives_options_surface",
)];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn option_contract() -> Schema {
    schema_for!(OptionContract)
}

fn equity_historical() -> Schema {
    schema_for!(EquityHistoricalData)
}

fn futures_curve_point() -> Schema {
    schema_for!(FuturesCurvePoint)
}

fn futures_instrument() -> Schema {
    schema_for!(FuturesInstrument)
}

// --- Computed option-pricing routes (eco G001). -----------------------------
// The `derivatives/pricing/*` routes are `EndpointKind::Compute` derivations
// over caller-supplied contract inputs (spot/strike/rate/vol/time/...), with no
// provider candidates. Their params/model schemas come from the pure-Rust
// `tdw-quant-options` crate so the catalog and the runtime compute share one
// schema definition (mirroring the `quantitative/*` analytics routes).

fn black_scholes_params() -> Schema {
    schema_for!(BlackScholesParams)
}
fn implied_vol_params() -> Schema {
    schema_for!(ImpliedVolParams)
}
fn binomial_params() -> Schema {
    schema_for!(BinomialParams)
}
fn monte_carlo_params() -> Schema {
    schema_for!(MonteCarloParams)
}
fn scalar_model() -> Schema {
    schema_for!(f64)
}
fn greeks_model() -> Schema {
    schema_for!(Greeks)
}
fn monte_carlo_model() -> Schema {
    schema_for!(MonteCarloPrice)
}

/// Construct one `derivatives/pricing/*` Compute entry with no provider
/// candidates. Pricing routes summarize the contract into a single figure / row,
/// so they are not chartable.
const fn compute_pricing_entry(
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

/// The `derivatives` namespace's catalog entries, in declaration order: the
/// provider-backed Fetch routes followed by the computed `pricing/*` routes.
pub fn entries() -> Vec<CatalogEntry> {
    let mut fetch = vec![
        CatalogEntry {
            route: "derivatives/options/chains",
            kind: EndpointKind::Fetch,
            params_schema,
            model: option_contract,
            candidates: DERIVATIVES_OPTIONS_CHAINS,
            bronze_table: Some("raw.option_contract"),
            doc: "Delayed options chain (calls and puts across expiries), Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "derivatives/futures/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: equity_historical,
            candidates: DERIVATIVES_FUTURES_HISTORICAL,
            bronze_table: Some("raw.equity_historical"),
            doc: "Historical OHLCV bars for a futures continuation symbol, Yahoo-backed.",
            chartable: true,
        },
        CatalogEntry {
            route: "derivatives/futures/curve",
            kind: EndpointKind::Fetch,
            params_schema,
            model: futures_curve_point,
            candidates: DERIVATIVES_FUTURES_CURVE,
            bronze_table: Some("raw.futures_curve_point"),
            doc: "Futures forward curve (per-expiry contract last prices), Yahoo-backed.",
            chartable: true,
        },
        CatalogEntry {
            route: "derivatives/futures/instruments",
            kind: EndpointKind::Fetch,
            params_schema,
            model: futures_instrument,
            candidates: DERIVATIVES_FUTURES_INSTRUMENTS,
            bronze_table: Some("raw.futures_instrument"),
            doc: "List tradable futures instruments for a currency, Deribit-backed (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "derivatives/futures/info",
            kind: EndpointKind::Fetch,
            params_schema,
            model: futures_instrument,
            candidates: DERIVATIVES_FUTURES_INFO,
            bronze_table: Some("raw.futures_instrument"),
            doc: "Futures instrument metadata for one instrument, Deribit-backed (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "derivatives/options/unusual",
            kind: EndpointKind::Fetch,
            params_schema,
            model: option_contract,
            candidates: DERIVATIVES_OPTIONS_UNUSUAL,
            bronze_table: Some("raw.option_contract"),
            doc: "Unusual options activity (block / sweep trades) for a symbol, \
                  Intrinio-backed (keyed).",
            chartable: false,
        },
        CatalogEntry {
            route: "derivatives/options/snapshots",
            kind: EndpointKind::Fetch,
            params_schema,
            model: option_contract,
            candidates: DERIVATIVES_OPTIONS_SNAPSHOTS,
            bronze_table: Some("raw.option_contract"),
            doc: "Options market snapshots across the chain (quote / greeks), \
                  Intrinio-backed (keyed).",
            chartable: false,
        },
        CatalogEntry {
            route: "derivatives/options/surface",
            kind: EndpointKind::Fetch,
            params_schema,
            model: option_contract,
            candidates: DERIVATIVES_OPTIONS_SURFACE,
            bronze_table: Some("raw.option_contract"),
            doc: "Implied-volatility surface inputs over the options chain (per-contract \
                  IV / greeks); the surface solver is a documented follow-up. Intrinio-backed \
                  (keyed).",
            chartable: false,
        },
    ];
    fetch.extend(pricing_entries());
    fetch
}

/// The computed `derivatives/pricing/*` Compute entries (eco G001), in
/// declaration order. Split out of [`entries`] so each list stays short.
fn pricing_entries() -> Vec<CatalogEntry> {
    vec![
        compute_pricing_entry(
            "derivatives/pricing/black_scholes",
            black_scholes_params,
            scalar_model,
            "Black-Scholes-Merton European call/put price (with dividend yield).",
        ),
        compute_pricing_entry(
            "derivatives/pricing/greeks",
            black_scholes_params,
            greeks_model,
            "Analytic Black-Scholes greeks: delta, gamma, theta, vega, rho.",
        ),
        compute_pricing_entry(
            "derivatives/pricing/implied_volatility",
            implied_vol_params,
            scalar_model,
            "Implied volatility inverted from a market price (Newton-Raphson + bisection).",
        ),
        compute_pricing_entry(
            "derivatives/pricing/binomial",
            binomial_params,
            scalar_model,
            "Cox-Ross-Rubinstein binomial-tree price (European or American exercise).",
        ),
        compute_pricing_entry(
            "derivatives/pricing/monte_carlo",
            monte_carlo_params,
            monte_carlo_model,
            "Seeded Monte-Carlo GBM European price with antithetic variates and standard error.",
        ),
    ]
}
