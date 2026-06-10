//! `regulators/*` catalog routes (gap-matrix items **L2.6**, **L3.1**).
//!
//! SEC and Federal Reserve regulatory utilities. Each route declares a single
//! keyless candidate `(provider, endpoint_key)` where the endpoint key is the
//! concrete dispatch key the runtime registers for that provider's fetcher (not
//! the `'/'→'_'` route form, since these providers use short endpoint keys).
//! Per-provider conformance tests in `tdw-service-api` keep these rows and the
//! providers' static endpoint tables in sync without this crate depending on the
//! providers.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{FomcDocument, SymbolMapping};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const SEC_CIK_MAP: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "cik_map")];
const FED_FOMC_DOCUMENTS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "regulators_fed_fomc_documents",
)];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn symbol_mapping() -> Schema {
    schema_for!(SymbolMapping)
}

fn fomc_document() -> Schema {
    schema_for!(FomcDocument)
}

/// The `regulators` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "regulators/sec/cik_map",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: symbol_mapping,
            candidates: SEC_CIK_MAP,
            bronze_table: Some("raw.symbol_mapping"),
            doc: "Map ticker symbols to SEC CIKs via SEC company_tickers.json (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/fed/fomc_documents",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: fomc_document,
            candidates: FED_FOMC_DOCUMENTS,
            bronze_table: Some("raw.fomc_document"),
            doc: "FOMC meeting documents index from the Federal Reserve (keyless).",
            chartable: false,
        },
    ]
}
