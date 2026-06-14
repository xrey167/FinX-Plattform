//! `uscongress/*` catalog routes (OpenBB-parity **P4W12**).
//!
//! US Congress legislative data from the public `congress.gov` v3 API. Each
//! route declares a single keyed candidate `(provider, endpoint_key)` where the
//! endpoint key is the concrete dispatch key the runtime registers for the
//! `congress-gov` provider's fetcher. A per-provider conformance test in
//! `tdw-service-api` keeps these rows and the provider's static endpoint table in
//! sync without this crate depending on the provider.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{BillTextUrl, CongressBill};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const USCONGRESS_BILLS: &[ProviderCandidate] =
    &[ProviderCandidate::new("congress-gov", "uscongress_bills")];
const USCONGRESS_BILL_INFO: &[ProviderCandidate] = &[ProviderCandidate::new(
    "congress-gov",
    "uscongress_bill_info",
)];
const USCONGRESS_BILL_TEXT_URLS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "congress-gov",
    "uscongress_bill_text_urls",
)];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn congress_bill() -> Schema {
    schema_for!(CongressBill)
}

fn bill_text_url() -> Schema {
    schema_for!(BillTextUrl)
}

/// The `uscongress` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "uscongress/bills",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: congress_bill,
            candidates: USCONGRESS_BILLS,
            bronze_table: Some("raw.congress_bill"),
            doc: "List US Congress bills for a congress and bill type (congress.gov, free key).",
            chartable: false,
        },
        CatalogEntry {
            route: "uscongress/bill_info",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: congress_bill,
            candidates: USCONGRESS_BILL_INFO,
            bronze_table: Some("raw.congress_bill"),
            doc: "Detailed metadata for a single US Congress bill (congress.gov, free key).",
            chartable: false,
        },
        CatalogEntry {
            route: "uscongress/bill_text_urls",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: bill_text_url,
            candidates: USCONGRESS_BILL_TEXT_URLS,
            bronze_table: Some("raw.bill_text_url"),
            doc: "Downloadable text-version URLs for a US Congress bill (congress.gov, free key).",
            chartable: false,
        },
    ]
}
