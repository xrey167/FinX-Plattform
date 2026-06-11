//! Clean-room catalog mapping `OpenBB` CFTC command paths to the public CFTC
//! Socrata (SODA) dataset that backs them (OpenBB-parity item **P2W5**).
//!
//! Each [`CftcEndpoint`] standardizes one `OpenBB` Platform command (per the
//! `regulators/cftc/*` surface) onto the CFTC legacy futures-only Commitments of
//! Traders dataset. The dataset resource id is a public fact from the CFTC's own
//! Socrata API docs at <https://publicreporting.cftc.gov/> (resource
//! `6dca-aqww`). The API is keyless; an optional `X-App-Token` raises the rate
//! limit.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build.

/// Which standardized [`tdw_domain`](https://docs.rs) model a CFTC endpoint
/// emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CftcModel {
    /// Standardizes to [`tdw_domain::CommitmentOfTraders`].
    Cot,
    /// Standardizes to [`tdw_domain::SeriesSearchResult`] (market discovery).
    CotSearch,
}

/// One standardized CFTC Socrata endpoint.
///
/// `command` is the `OpenBB` Platform command path it standardizes. `dataset` is
/// the Socrata resource path (appended to [`crate::BASE_URL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CftcEndpoint {
    /// `OpenBB` command path this endpoint standardizes.
    pub command: &'static str,
    /// Socrata resource path, e.g. `"resource/6dca-aqww.json"`.
    pub dataset: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: CftcModel,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The Socrata resource path shared by both CFTC COT routes.
pub const COT_DATASET: &str = "resource/6dca-aqww.json";

/// The standardized CFTC endpoint catalog.
pub const ENDPOINTS: &[CftcEndpoint] = &[
    CftcEndpoint {
        command: "regulators/cftc/cot",
        dataset: COT_DATASET,
        model: CftcModel::Cot,
        description: "CFTC legacy futures-only Commitments of Traders report \
                      (Socrata 6dca-aqww).",
    },
    CftcEndpoint {
        command: "regulators/cftc/cot_search",
        dataset: COT_DATASET,
        model: CftcModel::CotSearch,
        description: "Distinct CFTC COT market-and-exchange names for discovery \
                      (Socrata 6dca-aqww).",
    },
];

/// Resolve a catalog entry by its `OpenBB` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static CftcEndpoint> {
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
        let cot = resolve("regulators/cftc/cot").expect("cot resolve");
        assert_eq!(cot.model, CftcModel::Cot);
        let search = resolve("regulators/cftc/cot_search").expect("cot_search resolve");
        assert_eq!(search.model, CftcModel::CotSearch);
        assert!(resolve("regulators/cftc/bogus").is_none());
    }
}
