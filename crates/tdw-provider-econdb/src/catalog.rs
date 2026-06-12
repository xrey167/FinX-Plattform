//! Clean-room catalog for the `EconDB` economic time-series provider
//! (`OpenBB`-parity item **P3W4**).
//!
//! `EconDB` exposes one standardized route — `economy/econdb/series` — that
//! fetches a single `EconDB` series by its `ticker`. The route↔endpoint mapping
//! is a public fact from the `EconDB` public API documentation at
//! <https://www.econdb.com/api/>: series observations are served from
//! `series/{TICKER}/?format=json`. The API is keyless for public series; an
//! optional free token raises limits.
//!
//! This module is dependency-free (no `http` feature) so the catalog and its
//! coverage tests compile in the default offline workspace build. The concrete
//! `EconDB` `ticker` (e.g. `RGDPUS`) is a per-request caller parameter, not a
//! catalog fact, because one route serves arbitrarily many tickers.

/// One standardized `EconDB` route.
///
/// `command` is the `economy/econdb/*` command path it standardizes.
/// `description` is a human-readable label carried into each emitted row's title
/// when the series response omits its own description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EcondbEndpoint {
    /// `economy/econdb/*` command path this endpoint standardizes.
    pub command: &'static str,
    /// Human-readable description of the endpoint.
    pub description: &'static str,
}

/// The standardized `EconDB` endpoint catalog (one route: a series-by-ticker
/// fetch).
pub const ENDPOINTS: &[EcondbEndpoint] = &[EcondbEndpoint {
    command: "economy/econdb/series",
    description: "EconDB economic time series by ticker \
                  (standardized cross-country macro series).",
}];

/// Resolve a catalog entry by its `economy/econdb/*` `command` path.
#[must_use]
pub fn resolve(command: &str) -> Option<&'static EcondbEndpoint> {
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
    fn resolve_finds_known_and_misses_unknown() {
        let series = resolve("economy/econdb/series").expect("series resolve");
        assert_eq!(series.command, "economy/econdb/series");
        assert!(resolve("economy/econdb/bogus").is_none());
    }
}
