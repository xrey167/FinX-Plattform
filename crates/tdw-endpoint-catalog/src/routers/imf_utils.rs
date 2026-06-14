//! `imf_utils/*` catalog routes (OpenBB-parity **P4W9**).
//!
//! Auxiliary IMF SDMX discovery helpers used to drive the `economy/imf/*`
//! `CompactData` queries: list the available dataflows / tables, read a
//! dataflow's key dimensions, and build a presentation table from them. Every
//! route standardizes one `imf_utils` command onto the
//! [`tdw_domain::ImfDiscoveryRecord`] model. The single candidate per route is
//! `(provider="imf", endpoint=<route with '/'→'_'>)` — the exact key the runtime
//! dispatch table registers under `provider-imf`; the binding injects the route's
//! `command` so one discovery fetcher serves every route. A conformance test in
//! `tdw-service-api` keeps these rows and the dispatch table in sync.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::ImfDiscoveryRecord;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const IMF_UTILS_LIST_DATAFLOWS: &[ProviderCandidate] =
    &[ProviderCandidate::new("imf", "imf_utils_list_dataflows")];
const IMF_UTILS_LIST_TABLES: &[ProviderCandidate] =
    &[ProviderCandidate::new("imf", "imf_utils_list_tables")];
const IMF_UTILS_GET_DATAFLOW_DIMENSIONS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "imf",
    "imf_utils_get_dataflow_dimensions",
)];
const IMF_UTILS_PRESENTATION_TABLE: &[ProviderCandidate] = &[ProviderCandidate::new(
    "imf",
    "imf_utils_presentation_table",
)];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn imf_discovery_record() -> Schema {
    schema_for!(ImfDiscoveryRecord)
}

/// One `imf_utils` discovery catalog entry (`imf_utils/*` →
/// [`ImfDiscoveryRecord`]).
fn discovery_entry(
    route: &'static str,
    candidates: &'static [ProviderCandidate],
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: imf_discovery_record,
        candidates,
        bronze_table: Some("raw.imf_discovery_record"),
        doc,
        chartable: false,
    }
}

/// The `imf_utils` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        discovery_entry(
            "imf_utils/list_dataflows",
            IMF_UTILS_LIST_DATAFLOWS,
            "List available IMF SDMX dataflows (SDMX-JSON discovery, keyless).",
        ),
        discovery_entry(
            "imf_utils/list_tables",
            IMF_UTILS_LIST_TABLES,
            "List IMF SDMX dataflow tables, optionally filtered by id prefix (keyless).",
        ),
        discovery_entry(
            "imf_utils/get_dataflow_dimensions",
            IMF_UTILS_GET_DATAFLOW_DIMENSIONS,
            "Get the SDMX key dimensions of an IMF dataflow (keyless).",
        ),
        discovery_entry(
            "imf_utils/presentation_table",
            IMF_UTILS_PRESENTATION_TABLE,
            "Build an IMF dataflow presentation table from its SDMX dimensions (keyless).",
        ),
    ]
}
