//! Clean-room catalog mapping `OpenBB` Federal-Reserve command paths to the
//! public Fed data portals that back them (gap-matrix item **L3.1**).
//!
//! Each [`FedEndpoint`] standardizes one `OpenBB` Platform command (per the
//! `docs/roadmap/openbb-surface-domains.md` economy / fixedincome tables) onto a
//! concrete Federal Reserve data resource. The resource paths are public facts
//! from the Fed's documentation:
//! * H.6 Money Stock Measures — <https://www.federalreserve.gov/releases/h6/>
//! * Primary-dealer statistics (FRBNY markets data) —
//!   <https://www.newyorkfed.org/markets/counterparties/primary-dealers-statistics>
//! * FOMC meeting calendar / documents —
//!   <https://www.federalreserve.gov/monetarypolicy/fomccalendars.htm>
//!
//! Keyless. This module is dependency-free (no `http` feature) so the catalog
//! and its tests compile in the default offline workspace build.

/// Which standardized [`tdw_domain`](https://docs.rs) model a Fed endpoint emits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FedModel {
    /// Standardizes to [`tdw_domain::MacroSeries`] (observation series).
    Macro,
    /// Standardizes to [`tdw_domain::FomcDocument`] (document index).
    FomcDocuments,
}

/// One standardized Federal-Reserve-backed endpoint.
///
/// `command` is the `OpenBB` Platform command path it standardizes. `series_id`
/// labels the emitted [`tdw_domain::MacroSeries`] rows (the full H.6 table id or
/// the primary-dealer statistic id); it is empty for the document-index endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FedEndpoint {
    /// `OpenBB` command path this endpoint standardizes.
    pub command: &'static str,
    /// Series / table identifier carried onto emitted rows (empty when N/A).
    pub series_id: &'static str,
    /// Human-readable title for `MacroSeries::title`.
    pub title: &'static str,
    /// Frequency label, e.g. `"weekly"`, `"monthly"`.
    pub frequency: &'static str,
    /// Unit label, e.g. `"usd"`, `"count"`.
    pub unit: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: FedModel,
}

/// The standardized Federal-Reserve endpoint catalog (keyless government wave).
pub const ENDPOINTS: &[FedEndpoint] = &[
    FedEndpoint {
        command: "economy/money_measures",
        series_id: "H6_M2_SA",
        title: "H.6 Money Stock Measures (M2, seasonally adjusted)",
        frequency: "monthly",
        unit: "usd",
        model: FedModel::Macro,
    },
    FedEndpoint {
        command: "fixedincome/government/dealer_stats",
        series_id: "PD_NET_POSITIONS",
        title: "Primary Dealer Net Positioning (FRBNY)",
        frequency: "weekly",
        unit: "usd",
        model: FedModel::Macro,
    },
    FedEndpoint {
        command: "regulators/fed/fomc_documents",
        series_id: "",
        title: "FOMC Meeting Documents Index",
        frequency: "",
        unit: "",
        model: FedModel::FomcDocuments,
    },
    // -- economy: SOMA / primary-dealer breadth (OpenBB-parity P4W4) ---------
    FedEndpoint {
        command: "economy/central_bank_holdings",
        series_id: "SOMA_HOLDINGS",
        title: "Federal Reserve SOMA Securities Holdings",
        frequency: "weekly",
        unit: "usd",
        model: FedModel::Macro,
    },
    FedEndpoint {
        command: "economy/primary_dealer_positioning",
        series_id: "PD_NET_POSITIONS",
        title: "Primary Dealer Net Positioning (FRBNY)",
        frequency: "weekly",
        unit: "usd",
        model: FedModel::Macro,
    },
    FedEndpoint {
        command: "economy/primary_dealer_fails",
        series_id: "PD_FAILS",
        title: "Primary Dealer Settlement Fails (FRBNY)",
        frequency: "weekly",
        unit: "usd",
        model: FedModel::Macro,
    },
    FedEndpoint {
        command: "economy/fomc_documents",
        series_id: "",
        title: "FOMC Meeting Documents Index",
        frequency: "",
        unit: "",
        model: FedModel::FomcDocuments,
    },
];

/// Resolve a catalog entry by its `OpenBB` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static FedEndpoint> {
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
        let money = resolve("economy/money_measures").expect("money_measures resolve");
        assert_eq!(money.model, FedModel::Macro);
        let fomc = resolve("regulators/fed/fomc_documents").expect("fomc resolve");
        assert_eq!(fomc.model, FedModel::FomcDocuments);
        assert!(resolve("economy/cpi").is_none());
    }

    #[test]
    fn macro_endpoints_carry_series_id() {
        for entry in ENDPOINTS.iter().filter(|e| e.model == FedModel::Macro) {
            assert!(
                !entry.series_id.is_empty(),
                "macro endpoint {} must carry a series id",
                entry.command
            );
        }
    }
}
