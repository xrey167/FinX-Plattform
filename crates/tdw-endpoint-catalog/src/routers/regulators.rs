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
use tdw_domain::{
    CommitmentOfTraders, FilingFile, FilingHeader, FomcDocument, LitigationRelease, SecFilingHtml,
    SecInstitution, SeriesSearchResult, SicCode, SymbolMapping,
};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const SEC_CIK_MAP: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "cik_map")];
// Keyless SEC regulator utilities (openbb-parity P4W8). Each endpoint key
// matches the SEC fetcher's `ENDPOINT` const and runtime dispatch key.
const SEC_SYMBOL_MAP: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "symbol_map")];
const SEC_INSTITUTIONS_SEARCH: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "institutions_search")];
const SEC_SIC_SEARCH: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "sic_search")];
const SEC_FILING_HEADERS: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "filing_headers")];
const SEC_SCHEMA_FILES: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "schema_files")];
const SEC_RSS_LITIGATION: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "rss_litigation")];
// OpenBB-parity total G003c: keyless filing-HTML retrieval by EDGAR archive URL.
const SEC_HTM_FILE: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "htm_file")];
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

fn sec_institution() -> Schema {
    schema_for!(SecInstitution)
}

fn sic_code() -> Schema {
    schema_for!(SicCode)
}

fn filing_header() -> Schema {
    schema_for!(FilingHeader)
}

fn filing_file() -> Schema {
    schema_for!(FilingFile)
}

fn litigation_release() -> Schema {
    schema_for!(LitigationRelease)
}

fn sec_filing_html() -> Schema {
    schema_for!(SecFilingHtml)
}

/// The `regulators` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    let mut entries = core_entries();
    entries.extend(sec_utility_entries());
    entries
}

/// The pre-existing `regulators` routes (SEC `cik_map`, Fed FOMC, CFTC cluster).
fn core_entries() -> Vec<CatalogEntry> {
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

/// The keyless SEC regulator-utility routes (openbb-parity P4W8).
fn sec_utility_entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "regulators/sec/symbol_map",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: symbol_mapping,
            candidates: SEC_SYMBOL_MAP,
            bronze_table: Some("raw.symbol_mapping"),
            doc: "Map SEC CIKs to ticker symbols via company_tickers.json (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/institutions_search",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: sec_institution,
            candidates: SEC_INSTITUTIONS_SEARCH,
            bronze_table: Some("raw.sec_institution"),
            doc: "Search SEC-regulated institutions by name in company_tickers.json (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/sic_search",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: sic_code,
            candidates: SEC_SIC_SEARCH,
            bronze_table: Some("raw.sic_code"),
            doc: "Search the SEC Standard Industrial Classification (SIC) code list (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/filing_headers",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: filing_header,
            candidates: SEC_FILING_HEADERS,
            bronze_table: Some("raw.filing_header"),
            doc: "Filing header metadata for an accession from the EDGAR index (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/schema_files",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: filing_file,
            candidates: SEC_SCHEMA_FILES,
            bronze_table: Some("raw.filing_file"),
            doc: "List the schema/data files in a filing from the EDGAR index (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/rss_litigation",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: litigation_release,
            candidates: SEC_RSS_LITIGATION,
            bronze_table: Some("raw.litigation_release"),
            doc: "SEC litigation releases from the public litigation RSS feed (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "regulators/sec/htm_file",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: sec_filing_html,
            candidates: SEC_HTM_FILE,
            bronze_table: Some("raw.sec_filing_html"),
            doc: "Retrieve an HTML file from a filing by its EDGAR archive URL (keyless).",
            chartable: false,
        },
    ]
}
