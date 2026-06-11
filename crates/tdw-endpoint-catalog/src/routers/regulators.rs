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
use tdw_domain::{CommitmentOfTraders, FomcDocument, SeriesSearchResult, SymbolMapping};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const SEC_CIK_MAP: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "cik_map")];
const FED_FOMC_DOCUMENTS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "regulators_fed_fomc_documents",
)];
const CFTC_COT: &[ProviderCandidate] = &[ProviderCandidate::new("cftc", "regulators_cftc_cot")];
const CFTC_COT_SEARCH: &[ProviderCandidate] =
    &[ProviderCandidate::new("cftc", "regulators_cftc_cot_search")];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn symbol_mapping() -> Schema {
    schema_for!(SymbolMapping)
}

fn fomc_document() -> Schema {
    schema_for!(FomcDocument)
}

fn commitment_of_traders() -> Schema {
    schema_for!(CommitmentOfTraders)
}

fn series_search_result() -> Schema {
    schema_for!(SeriesSearchResult)
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
        CatalogEntry {
            route: "regulators/cftc/cot",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: commitment_of_traders,
            candidates: CFTC_COT,
            bronze_table: Some("raw.commitment_of_traders"),
            doc: "CFTC legacy futures-only Commitments of Traders report (Socrata, keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/cftc/cot_search",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: series_search_result,
            candidates: CFTC_COT_SEARCH,
            bronze_table: Some("raw.series_search_result"),
            doc: "Distinct CFTC COT market-and-exchange names for discovery (Socrata, keyless).",
            chartable: false,
        },
    ]
}
