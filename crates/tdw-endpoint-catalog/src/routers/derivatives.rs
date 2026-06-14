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
    ]
}
