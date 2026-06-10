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
use tdw_domain::MacroSeries;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// ECB-backed candidate for `currency/reference_rates`.
const CURRENCY_REFERENCE_RATES: &[ProviderCandidate] =
    &[ProviderCandidate::new("ecb", "reference_rates")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn macro_series() -> Schema {
    schema_for!(MacroSeries)
}

/// The `currency` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        route: "currency/reference_rates",
        kind: EndpointKind::Fetch,
        params_schema,
        model: macro_series,
        candidates: CURRENCY_REFERENCE_RATES,
        bronze_table: Some("raw.macro_series"),
        doc: "Daily euro foreign-exchange reference rates (all pairs), ECB-backed.",
        chartable: true,
    }]
}
