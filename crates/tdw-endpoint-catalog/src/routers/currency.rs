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
use tdw_domain::{MacroSeries, MarketDataBar};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// ECB-backed candidate for `currency/reference_rates`.
const CURRENCY_REFERENCE_RATES: &[ProviderCandidate] =
    &[ProviderCandidate::new("ecb", "reference_rates")];

/// Candidate order for `currency/price/historical` (gap-matrix item L2.x
/// polygon). The Polygon aggregate-bars fetcher serves FX pairs via the `C:`
/// ticker prefix (e.g. `C:EURUSD`); the caller supplies the prefixed ticker, so
/// the existing `aggregates` fetcher and its dispatch binding are reused
/// verbatim. Single keyed candidate (Polygon), so the route is unresolvable in
/// an offline build and resolvable once `provider-polygon` is enabled.
const CURRENCY_PRICE_HISTORICAL: &[ProviderCandidate] =
    &[ProviderCandidate::new("polygon", "aggregates")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn macro_series() -> Schema {
    schema_for!(MacroSeries)
}

fn market_data_bar() -> Schema {
    schema_for!(MarketDataBar)
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
    ]
}
