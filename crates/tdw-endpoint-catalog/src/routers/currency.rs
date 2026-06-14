//! `currency/*` catalog routes.
//!
//! The ECB euro foreign-exchange reference-rates snapshot (gap-matrix item
//! L2.x). The single candidate endpoint key matches the ECB fetcher's
//! `ENDPOINT` const — also the runtime dispatch-table key — and a conformance
//! test in tdw-service-api keeps this row and that table in sync. Adding a
//! route is an append to [`entries`], never a new wiring point in
//! [`crate::catalog`].

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{CurrencySnapshot, Instrument, MacroSeries, MarketDataBar};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// ECB-backed candidate for `currency/reference_rates`.
const CURRENCY_REFERENCE_RATES: &[ProviderCandidate] =
    &[ProviderCandidate::new("ecb", "reference_rates")];

/// FMP-backed candidate for `currency/search`.
///
/// Reuses the existing FMP `/search` fetcher (the `fmp/search` dispatch binding
/// already registered for `equity/search`): FMP's symbol search spans equities,
/// ETFs, FX, and crypto pairs, so the same keyword search serves FX-pair
/// discovery. No new fetcher or dispatch binding is added — this is a catalog
/// projection of the existing `(fmp, search)` endpoint onto a second route, the
/// established reuse pattern (mirrors the shared `polygon/aggregates` candidate).
const CURRENCY_SEARCH: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "search")];

/// Candidate order for `currency/price/historical` (gap-matrix item L2.x
/// polygon). The Polygon aggregate-bars fetcher serves FX pairs via the `C:`
/// ticker prefix (e.g. `C:EURUSD`); the caller supplies the prefixed ticker, so
/// the existing `aggregates` fetcher and its dispatch binding are reused
/// verbatim. Single keyed candidate (Polygon), so the route is unresolvable in
/// an offline build and resolvable once `provider-polygon` is enabled.
const CURRENCY_PRICE_HISTORICAL: &[ProviderCandidate] =
    &[ProviderCandidate::new("polygon", "aggregates")];

/// FMP-backed candidate for `currency/snapshots`.
///
/// FMP's `/fx` forex-snapshot endpoint returns the full set of FX-pair quotes
/// (bid/ask/open); an optional `symbol` narrows to one pair client-side. Single
/// keyed candidate, so the route resolves once `provider-fmp` is enabled.
const CURRENCY_SNAPSHOTS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "currency_snapshots")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn currency_snapshot() -> Schema {
    schema_for!(CurrencySnapshot)
}

fn macro_series() -> Schema {
    schema_for!(MacroSeries)
}

fn market_data_bar() -> Schema {
    schema_for!(MarketDataBar)
}

fn instrument() -> Schema {
    schema_for!(Instrument)
}

/// The `currency` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "currency/reference_rates",
            kind: EndpointKind::Fetch,
            params_schema,
            model: macro_series,
            candidates: CURRENCY_REFERENCE_RATES,
            bronze_table: Some("raw.macro_series"),
            doc: "Daily euro foreign-exchange reference rates (all pairs), ECB-backed.",
            chartable: true,
        },
        CatalogEntry {
            route: "currency/price/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: market_data_bar,
            candidates: CURRENCY_PRICE_HISTORICAL,
            bronze_table: Some("raw.market_data_bar"),
            doc: "Historical OHLCV bars for an FX pair (Polygon `C:` ticker prefix).",
            chartable: true,
        },
        CatalogEntry {
            route: "currency/search",
            kind: EndpointKind::Fetch,
            params_schema,
            model: instrument,
            candidates: CURRENCY_SEARCH,
            bronze_table: Some("raw.instrument"),
            doc: "Search available currency (FX) pairs by name or fragment, FMP-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "currency/snapshots",
            kind: EndpointKind::Fetch,
            params_schema,
            model: currency_snapshot,
            candidates: CURRENCY_SNAPSHOTS,
            bronze_table: Some("raw.currency_snapshot"),
            doc: "Forex-pair snapshot quotes (price/bid/ask) across all pairs, FMP-backed.",
            chartable: false,
        },
    ]
}
