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
use tdw_domain::{EquityHistoricalData, FuturesCurvePoint, OptionContract};

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

/// The `derivatives` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
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
    ]
}
