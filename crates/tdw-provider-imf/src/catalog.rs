//! Clean-room catalog mapping `economy/imf/*` command paths to the public IMF
//! Data Services SDMX-JSON database that backs them (OpenBB-parity item
//! **P3W3**).
//!
//! Each [`ImfEndpoint`] standardizes one `economy/imf/*` command onto an IMF
//! SDMX `CompactData` database id. The database ids are public facts from the
//! IMF Data Services SDMX-JSON documentation at <https://dataservices.imf.org/>:
//! `IFS` (International Financial Statistics), `DOT` (Direction of Trade), and
//! `BOP` (Balance of Payments). The API is keyless.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build. The concrete
//! SDMX `Key` (the dot-separated `Frequency.RefArea.Indicator` dimensions) is a
//! per-request caller parameter, not a catalog fact, because one database backs
//! arbitrarily many series.

/// One standardized IMF SDMX-JSON database endpoint.
///
/// `command` is the `economy/imf/*` command path it standardizes. `database` is
/// the IMF SDMX `CompactData` database id (appended to [`crate::BASE_URL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImfEndpoint {
    /// `economy/imf/*` command path this endpoint standardizes.
    pub command: &'static str,
    /// IMF SDMX `CompactData` database id, e.g. `"IFS"`.
    pub database: &'static str,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// International Financial Statistics database id.
pub const IFS_DATABASE: &str = "IFS";
/// Direction of Trade Statistics database id.
pub const DOT_DATABASE: &str = "DOT";
/// Balance of Payments database id.
pub const BOP_DATABASE: &str = "BOP";

/// The standardized IMF SDMX-JSON endpoint catalog.
pub const ENDPOINTS: &[ImfEndpoint] = &[
    ImfEndpoint {
        command: "economy/imf/international_financial_statistics",
        database: IFS_DATABASE,
        description: "IMF International Financial Statistics time series \
                      (SDMX-JSON CompactData IFS).",
    },
    ImfEndpoint {
        command: "economy/imf/direction_of_trade",
        database: DOT_DATABASE,
        description: "IMF Direction of Trade Statistics time series \
                      (SDMX-JSON CompactData DOT).",
    },
    ImfEndpoint {
        command: "economy/imf/balance_of_payments",
        database: BOP_DATABASE,
        description: "IMF Balance of Payments time series \
                      (SDMX-JSON CompactData BOP).",
    },
];

/// Resolve a catalog entry by its `economy/imf/*` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static ImfEndpoint> {
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
        let ifs = resolve("economy/imf/international_financial_statistics").expect("ifs resolve");
        assert_eq!(ifs.database, IFS_DATABASE);
        let dot = resolve("economy/imf/direction_of_trade").expect("dot resolve");
        assert_eq!(dot.database, DOT_DATABASE);
        let bop = resolve("economy/imf/balance_of_payments").expect("bop resolve");
        assert_eq!(bop.database, BOP_DATABASE);
        assert!(resolve("economy/imf/bogus").is_none());
    }
}
