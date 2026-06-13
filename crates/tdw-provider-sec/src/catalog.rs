//! Clean-room catalog mapping `OpenBB` SEC command paths to the public SEC
//! EDGAR / data.sec.gov endpoints that back them (gap-matrix item **L2.6**).
//!
//! Each [`SecEndpoint`] standardizes one `OpenBB` Platform command (per the
//! `docs/roadmap/openbb-surface-domains.md` regulators / equity / etf tables)
//! onto a concrete SEC data API plus the metadata needed to populate the
//! relevant [`tdw_domain`] row. The API paths are public facts documented at
//! <https://www.sec.gov/search-filings/edgar-application-programming-interfaces>
//! (e.g. `company_tickers.json` for the ticker↔CIK map, `submissions/CIK*.json`
//! for filing indices, `data.sec.gov/api/xbrl/companyfacts` for XBRL facts).
//!
//! This module is intentionally dependency-free (no `http` feature, no
//! `tdw-core`/`tdw-domain`) so the catalog and its coverage tests compile and
//! run in the default offline workspace build. The HTTP fetchers in
//! `http_fetcher.rs` resolve a caller-supplied command key against this catalog.

/// Which standardized [`tdw_domain`](https://docs.rs) model a SEC endpoint emits.
///
/// The discriminant lets the dispatch layer pick the right fetcher for a
/// command without the catalog crate depending on the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecModel {
    /// Standardizes to [`tdw_domain::SymbolMapping`] (`company_tickers.json`).
    SymbolMapping,
    /// Standardizes to [`tdw_domain::OwnershipRecord`] (Form 13F submissions).
    Form13F,
    /// Standardizes to [`tdw_domain::OwnershipRecord`] (fails-to-deliver).
    FailsToDeliver,
    /// Standardizes to [`tdw_domain::EtfHolding`] (N-PORT portfolio).
    EtfHoldings,
    /// Standardizes to [`tdw_domain::CompanyFacts`] (XBRL company facts).
    CompanyFacts,
    /// Standardizes to the filings-index row (latest periodic financial reports).
    LatestFinancialReports,
}

/// One standardized SEC-backed endpoint.
///
/// `command` is the `OpenBB` Platform command path it standardizes (e.g.
/// `"regulators/sec/cik_map"`, `"etf/holdings"`). The remaining fields carry the
/// static normalization metadata the fetchers consult.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecEndpoint {
    /// `OpenBB` command path this endpoint standardizes.
    pub command: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: SecModel,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized SEC-backed endpoint catalog (the keyless government wave).
///
/// Ordered by cluster: the regulators ticker map, ownership (13F),
/// shorts (fails-to-deliver), then ETF holdings (N-PORT).
pub const ENDPOINTS: &[SecEndpoint] = &[
    SecEndpoint {
        command: "regulators/sec/cik_map",
        model: SecModel::SymbolMapping,
        description: "Map ticker symbols to SEC CIKs via company_tickers.json.",
    },
    SecEndpoint {
        command: "equity/ownership/form_13f",
        model: SecModel::Form13F,
        description: "Form 13F-HR institutional-holding filing index from submissions.",
    },
    SecEndpoint {
        command: "equity/shorts/fails_to_deliver",
        model: SecModel::FailsToDeliver,
        description: "SEC fails-to-deliver records for a symbol/CIK.",
    },
    SecEndpoint {
        command: "etf/holdings",
        model: SecModel::EtfHoldings,
        description: "ETF constituent holdings from NPORT-P portfolio disclosures.",
    },
    SecEndpoint {
        command: "equity/compare/company_facts",
        model: SecModel::CompanyFacts,
        description: "XBRL company facts (reported concept values) from companyfacts.",
    },
    SecEndpoint {
        command: "equity/discovery/latest_financial_reports",
        model: SecModel::LatestFinancialReports,
        description: "Latest periodic financial reports (10-K/10-Q) from submissions.",
    },
];

/// Resolve a catalog entry by its `OpenBB` `command` path.
///
/// Returns the matching [`SecEndpoint`] or `None` when the command is not in the
/// standardized catalog.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static SecEndpoint> {
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
    fn resolve_finds_known_commands_and_misses_unknown() {
        let cik = resolve("regulators/sec/cik_map").expect("cik_map must resolve");
        assert_eq!(cik.model, SecModel::SymbolMapping);

        let etf = resolve("etf/holdings").expect("etf holdings must resolve");
        assert_eq!(etf.model, SecModel::EtfHoldings);

        assert!(resolve("equity/price/quote").is_none());
    }

    #[test]
    fn every_model_class_is_populated() {
        for model in [
            SecModel::SymbolMapping,
            SecModel::Form13F,
            SecModel::FailsToDeliver,
            SecModel::EtfHoldings,
            SecModel::CompanyFacts,
            SecModel::LatestFinancialReports,
        ] {
            assert!(
                ENDPOINTS.iter().any(|e| e.model == model),
                "no endpoint for model {model:?}"
            );
        }
    }
}
