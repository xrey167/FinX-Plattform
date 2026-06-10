//! `etf/*` catalog routes (gap-matrix item **L2.6**, the ETF-holdings crown
//! jewel).
//!
//! ETF holdings are served keylessly from SEC N-PORT (`NPORT-P`) portfolio
//! disclosures. The route declares a single `sec` candidate whose endpoint key
//! is the concrete dispatch key the runtime registers for that fetcher. A
//! per-provider conformance test in `tdw-service-api` keeps these rows and the
//! SEC provider's static endpoint table in sync.
//!
//! `etf/sectors` is intentionally not wired here: deriving per-sector weights
//! ([`tdw_domain::EtfSectorWeight`]) needs a distinct sector fetcher rather than
//! the holdings fetcher, so it is deferred rather than aliased onto the holdings
//! dispatch key (which would mis-shape its records).

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::EtfHolding;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const ETF_HOLDINGS: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "etf_holdings")];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn etf_holding() -> Schema {
    schema_for!(EtfHolding)
}

/// The `etf` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        route: "etf/holdings",
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: etf_holding,
        candidates: ETF_HOLDINGS,
        bronze_table: Some("raw.etf_holding"),
        doc: "ETF constituent holdings from SEC N-PORT (NPORT-P) disclosures (keyless).",
        chartable: false,
    }]
}
