//! `commodity/*` catalog routes.
//!
//! The EIA report cluster (gap-matrix item L2.x): the Weekly Petroleum Status
//! Report and the Short-Term Energy Outlook. Both routes share the EIA `report`
//! fetcher (the report discriminator is injected per dispatch binding), but each
//! route gets a distinct dispatch endpoint key — the route with each `'/'`
//! replaced by `'_'`, matching `endpoint_key_for_route` — so the per-route
//! `report` value the dispatcher injects never collides. A conformance test in
//! tdw-service-api keeps these rows and the dispatch table in sync. Adding a
//! route is an append to [`entries`], never a new wiring point in
//! [`crate::catalog`].

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::CommodityReportRow;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// EIA-backed candidate for the petroleum status report.
const COMMODITY_PETROLEUM_STATUS_REPORT: &[ProviderCandidate] = &[ProviderCandidate::new(
    "eia",
    "commodity_petroleum_status_report",
)];
/// EIA-backed candidate for the short-term energy outlook.
const COMMODITY_SHORT_TERM_ENERGY_OUTLOOK: &[ProviderCandidate] = &[ProviderCandidate::new(
    "eia",
    "commodity_short_term_energy_outlook",
)];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn commodity_report_row() -> Schema {
    schema_for!(CommodityReportRow)
}

/// The `commodity` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "commodity/petroleum_status_report",
            kind: EndpointKind::Fetch,
            params_schema,
            model: commodity_report_row,
            candidates: COMMODITY_PETROLEUM_STATUS_REPORT,
            bronze_table: Some("raw.commodity_report_row"),
            doc: "EIA Weekly Petroleum Status Report (WPSR) series rows.",
            chartable: true,
        },
        CatalogEntry {
            route: "commodity/short_term_energy_outlook",
            kind: EndpointKind::Fetch,
            params_schema,
            model: commodity_report_row,
            candidates: COMMODITY_SHORT_TERM_ENERGY_OUTLOOK,
            bronze_table: Some("raw.commodity_report_row"),
            doc: "EIA Short-Term Energy Outlook (STEO) series rows.",
            chartable: true,
        },
    ]
}
