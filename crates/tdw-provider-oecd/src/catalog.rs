//! Clean-room catalog mapping `economy/*` OECD command paths to the public OECD
//! SDMX-JSON dataset (and default dimension filter) that backs them
//! (`OpenBB`-parity item **P4W4**).
//!
//! Each [`OecdEndpoint`] standardizes one `OpenBB` Platform `economy/*` command
//! onto an OECD SDMX dataset code plus a default dimension `filter` that selects
//! a representative cross-country slice. The dataset codes and key dimensions are
//! public facts from the OECD SDMX-JSON documentation at
//! <https://data.oecd.org/api/sdmx-json-documentation/>:
//! `EO` (Economic Outlook / forecasts), `MEI` (Main Economic Indicators) with the
//! `IRLT`/`IR3TIB` interest-rate subjects, `MEI_CLI` (Composite Leading
//! Indicators), `HOUSE_PRICES`, and the `SPASTT01` share-price subject. The API
//! is keyless.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build. The caller may
//! override the default `filter` (e.g. to a single country) on a per-request
//! basis; the catalog default keeps each route immediately queryable.

/// One standardized OECD SDMX-JSON endpoint.
///
/// `command` is the `economy/*` command path it standardizes. `dataset` is the
/// OECD SDMX dataset code (appended to [`crate::BASE_URL`]); `default_filter` is
/// the dimension key applied when the caller supplies none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OecdEndpoint {
    /// `economy/*` command path this endpoint standardizes.
    pub command: &'static str,
    /// OECD SDMX dataset code, e.g. `"MEI_CLI"`.
    pub dataset: &'static str,
    /// Default SDMX dimension filter (caller-overridable).
    pub default_filter: &'static str,
    /// Human-readable title carried onto emitted [`tdw_domain::MacroSeries`] rows.
    pub title: &'static str,
    /// Frequency label, e.g. `"quarterly"`, `"monthly"`, `"annual"`.
    pub frequency: &'static str,
    /// Unit label, e.g. `"index"`, `"percent"`.
    pub unit: &'static str,
}

/// The standardized OECD SDMX-JSON endpoint catalog (keyless macro cluster).
pub const ENDPOINTS: &[OecdEndpoint] = &[
    OecdEndpoint {
        command: "economy/gdp/forecast",
        dataset: "EO",
        // Economic Outlook: USA real GDP volume growth, annual.
        default_filter: "USA.GDPV_ANNPCT.A",
        title: "OECD Economic Outlook: Real GDP forecast",
        frequency: "annual",
        unit: "percent",
    },
    OecdEndpoint {
        command: "economy/interest_rates",
        dataset: "MEI",
        // Main Economic Indicators: USA long-term interest rates, monthly.
        default_filter: "USA.IRLT.ST.M",
        title: "OECD Main Economic Indicators: Long-term interest rates",
        frequency: "monthly",
        unit: "percent",
    },
    OecdEndpoint {
        command: "economy/composite_leading_indicator",
        dataset: "MEI_CLI",
        // Amplitude-adjusted composite leading indicator, USA, monthly.
        default_filter: "LOLITOAA.USA.M",
        title: "OECD Composite Leading Indicator (amplitude-adjusted)",
        frequency: "monthly",
        unit: "index",
    },
    OecdEndpoint {
        command: "economy/house_price_index",
        dataset: "HOUSE_PRICES",
        // Nominal house price index, USA, quarterly.
        default_filter: "USA.NOMINAL.Q",
        title: "OECD House Price Index (nominal)",
        frequency: "quarterly",
        unit: "index",
    },
    OecdEndpoint {
        command: "economy/share_price_index",
        dataset: "MEI",
        // Share price index (SPASTT01), USA, monthly index level.
        default_filter: "USA.SPASTT01.IXOB.M",
        title: "OECD Main Economic Indicators: Share price index",
        frequency: "monthly",
        unit: "index",
    },
];

/// Resolve a catalog entry by its `economy/*` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static OecdEndpoint> {
    ENDPOINTS.iter().find(|entry| entry.command == command)
}

#[cfg(test)]
mod tests {
    use super::{ENDPOINTS, resolve};
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
    fn datasets_and_filters_are_non_empty() {
        for entry in ENDPOINTS {
            assert!(
                !entry.dataset.is_empty(),
                "empty dataset for {}",
                entry.command
            );
            assert!(
                !entry.default_filter.is_empty(),
                "empty default filter for {}",
                entry.command
            );
        }
    }

    #[test]
    fn resolve_finds_known_and_misses_unknown() {
        let cli = resolve("economy/composite_leading_indicator").expect("cli resolve");
        assert_eq!(cli.dataset, "MEI_CLI");
        let gdp = resolve("economy/gdp/forecast").expect("gdp forecast resolve");
        assert_eq!(gdp.dataset, "EO");
        assert!(resolve("economy/bogus").is_none());
    }
}
