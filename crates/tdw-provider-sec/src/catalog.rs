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
    /// Standardizes to the filings-index row (N-PORT portfolio disclosures).
    NportDisclosure,
    /// Standardizes to [`tdw_domain::SymbolMapping`] (CIK → ticker map).
    SymbolMap,
    /// Standardizes to [`tdw_domain::SecInstitution`] (institution name search).
    InstitutionsSearch,
    /// Standardizes to [`tdw_domain::SicCode`] (SIC industry-code search).
    SicSearch,
    /// Standardizes to [`tdw_domain::FilingHeader`] (filing-index header block).
    FilingHeaders,
    /// Standardizes to [`tdw_domain::FilingFile`] (filing-index file list).
    SchemaFiles,
    /// Standardizes to [`tdw_domain::LitigationRelease`] (litigation RSS feed).
    RssLitigation,
    /// Standardizes to [`tdw_domain::SecFilingHtml`] (a fetched filing HTML file).
    HtmFile,
    /// Standardizes to [`tdw_domain::SecFilingHtml`] (the MD&A section document).
    ManagementDiscussionAnalysis,
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
    SecEndpoint {
        command: "etf/nport_disclosure",
        model: SecModel::NportDisclosure,
        description: "Fund N-PORT portfolio-disclosure filing index from submissions.",
    },
    // Keyless SEC regulator utilities (openbb-parity P4W8).
    SecEndpoint {
        command: "regulators/sec/symbol_map",
        model: SecModel::SymbolMap,
        description: "Map SEC CIKs to ticker symbols via company_tickers.json.",
    },
    SecEndpoint {
        command: "regulators/sec/institutions_search",
        model: SecModel::InstitutionsSearch,
        description: "Search SEC-regulated institutions by name in company_tickers.json.",
    },
    SecEndpoint {
        command: "regulators/sec/sic_search",
        model: SecModel::SicSearch,
        description: "Search the SEC Standard Industrial Classification (SIC) code list.",
    },
    SecEndpoint {
        command: "regulators/sec/filing_headers",
        model: SecModel::FilingHeaders,
        description: "Filing header metadata for an accession from the EDGAR index.json.",
    },
    SecEndpoint {
        command: "regulators/sec/schema_files",
        model: SecModel::SchemaFiles,
        description: "List the schema/data files in a filing from the EDGAR index.json.",
    },
    SecEndpoint {
        command: "regulators/sec/rss_litigation",
        model: SecModel::RssLitigation,
        description: "SEC litigation releases from the public litigation RSS feed.",
    },
    // OpenBB-parity total G003c: keyless filing-HTML retrieval + MD&A section.
    SecEndpoint {
        command: "regulators/sec/htm_file",
        model: SecModel::HtmFile,
        description: "Retrieve an HTML file from a filing by its EDGAR archive URL.",
    },
    SecEndpoint {
        command: "equity/fundamental/management_discussion_analysis",
        model: SecModel::ManagementDiscussionAnalysis,
        description: "Management Discussion & Analysis section document from the latest 10-K.",
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
            SecModel::NportDisclosure,
            SecModel::SymbolMap,
            SecModel::InstitutionsSearch,
            SecModel::SicSearch,
            SecModel::FilingHeaders,
            SecModel::SchemaFiles,
            SecModel::RssLitigation,
            SecModel::HtmFile,
            SecModel::ManagementDiscussionAnalysis,
        ] {
            assert!(
                ENDPOINTS.iter().any(|e| e.model == model),
                "no endpoint for model {model:?}"
            );
        }
    }
}
