//! Clean-room catalog mapping `OpenBB`-style `uscongress/*` command paths to the
//! public `congress.gov` v3 API routes that back them (OpenBB-parity **P4W12**).
//!
//! Each [`CongressEndpoint`] standardizes one `uscongress` command onto a
//! `congress.gov` `/bill/...` route. The route templates are public facts from
//! the `congress.gov` API docs at <https://api.congress.gov/>; the API requires
//! a free api.data.gov key passed as the `api_key` query parameter.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build.

/// Which standardized [`tdw_domain`](https://docs.rs) model a `congress.gov`
/// endpoint emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CongressModel {
    /// Standardizes to [`tdw_domain::CongressBill`].
    Bill,
    /// Standardizes to [`tdw_domain::BillTextUrl`].
    TextUrl,
}

/// One standardized `congress.gov` endpoint.
///
/// `command` is the `uscongress` command path it standardizes. `route_template`
/// is the `congress.gov` path appended to [`crate::BASE_URL`], with
/// `{congress}`, `{billType}`, and `{billNumber}` placeholders the fetcher
/// substitutes from the query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CongressEndpoint {
    /// `uscongress` command path this endpoint standardizes.
    pub command: &'static str,
    /// `congress.gov` route template, relative to [`crate::BASE_URL`].
    pub route_template: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: CongressModel,
    /// Whether the route requires a `bill_number` path parameter.
    pub requires_bill_number: bool,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized `congress.gov` endpoint catalog.
pub const ENDPOINTS: &[CongressEndpoint] = &[
    CongressEndpoint {
        command: "uscongress/bills",
        route_template: "/bill/{congress}/{billType}",
        model: CongressModel::Bill,
        requires_bill_number: false,
        description: "List bills for a congress and bill type from the congress.gov v3 API.",
    },
    CongressEndpoint {
        command: "uscongress/bill_info",
        route_template: "/bill/{congress}/{billType}/{billNumber}",
        model: CongressModel::Bill,
        requires_bill_number: true,
        description: "Detailed metadata for a single congress.gov bill (sponsor, policy area).",
    },
    CongressEndpoint {
        command: "uscongress/bill_text_urls",
        route_template: "/bill/{congress}/{billType}/{billNumber}/text",
        model: CongressModel::TextUrl,
        requires_bill_number: true,
        description: "Downloadable text-version URLs for a congress.gov bill (Formatted/PDF/XML).",
    },
];

/// Resolve a catalog entry by its `uscongress` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static CongressEndpoint> {
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
        let bills = resolve("uscongress/bills").expect("bills resolve");
        assert_eq!(bills.model, CongressModel::Bill);
        assert!(!bills.requires_bill_number);
        let info = resolve("uscongress/bill_info").expect("bill_info resolve");
        assert!(info.requires_bill_number);
        let text = resolve("uscongress/bill_text_urls").expect("bill_text_urls resolve");
        assert_eq!(text.model, CongressModel::TextUrl);
        assert!(resolve("uscongress/bogus").is_none());
    }
}
