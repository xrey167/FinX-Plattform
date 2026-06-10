//! Clean-room catalog mapping `OpenBB` US-Treasury command paths to the public
//! Treasury `FiscalData` API datasets that back them (gap-matrix item **L3.2**).
//!
//! Each [`GovUsEndpoint`] standardizes one `OpenBB` Platform command (per the
//! `docs/roadmap/openbb-surface-domains.md` `fixedincome/government` table) onto
//! a concrete `FiscalData` dataset path. The dataset paths are public facts from
//! the `FiscalData` API docs at <https://fiscaldata.treasury.gov/api-documentation/>
//! (e.g. `v1/accounting/od/auctions_query` for auction results). The API is
//! keyless.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build.

/// Which standardized [`tdw_domain`](https://docs.rs) model a Treasury endpoint
/// emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GovUsModel {
    /// Standardizes to [`tdw_domain::TreasuryAuction`].
    Auction,
    /// Standardizes to [`tdw_domain::TreasuryPrice`].
    Price,
}

/// One standardized US-Treasury `FiscalData` endpoint.
///
/// `command` is the `OpenBB` Platform command path it standardizes. `dataset` is
/// the `FiscalData` API path (appended to [`crate::BASE_URL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GovUsEndpoint {
    /// `OpenBB` command path this endpoint standardizes.
    pub command: &'static str,
    /// `FiscalData` dataset path, e.g. `"v1/accounting/od/auctions_query"`.
    pub dataset: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: GovUsModel,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized US-Treasury endpoint catalog.
pub const ENDPOINTS: &[GovUsEndpoint] = &[
    GovUsEndpoint {
        command: "fixedincome/government/treasury_auctions",
        dataset: "v1/accounting/od/auctions_query",
        model: GovUsModel::Auction,
        description: "US Treasury security auction results (FiscalData auctions_query).",
    },
    GovUsEndpoint {
        command: "fixedincome/government/treasury_prices",
        dataset: "v1/accounting/od/securities_sales",
        model: GovUsModel::Price,
        description: "US Treasury security reference prices (FiscalData securities data).",
    },
];

/// Resolve a catalog entry by its `OpenBB` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static GovUsEndpoint> {
    ENDPOINTS.iter().find(|entry| entry.command == command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn command_paths_are_unique() {
        let mut seen = BTreeSet::new();
        for entry in ENDPOINTS {
            assert!(
                seen.insert(entry.command),
                "duplicate command path: {}",
                entry.command
            );
        }
    }

    #[test]
    fn resolve_finds_known_and_misses_unknown() {
        let auctions =
            resolve("fixedincome/government/treasury_auctions").expect("auctions resolve");
        assert_eq!(auctions.model, GovUsModel::Auction);
        assert!(resolve("economy/cpi").is_none());
    }
}
