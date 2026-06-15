#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Namespaced endpoint catalog (gap-matrix item **WS0**, the spine).
//!
//! The catalog is the single, build-independent source of truth for *which*
//! logical routes exist, what each one returns, and which concrete providers
//! can serve it. It generalizes the original three-route logical-endpoint
//! resolver (`tdw-service-api`'s `provider_resolve`) into a complete,
//! slash-namespaced table that every later layer (REST, `OpenAPI`, CLI, SDK,
//! widgets) derives from.
//!
//! A [`CatalogEntry`] names one logical route in slash form (e.g.
//! `equity/price/historical`), records the parameter and result schemas, and
//! lists its [`ProviderCandidate`]s in *declaration order* — which is the
//! fallback-preference order for the no-explicit-provider default. The list is
//! build-independent: every provider that *could* serve the route is listed
//! regardless of enabled features, with the keyless / offline-first fixtures
//! leading so an unkeyed default build resolves network-free. Availability is
//! filtered against the runtime ingest dispatch table by the consumer, exactly
//! as the original resolver did.
//!
//! This crate performs no I/O and enforces no policy: it is a pure, static,
//! deterministic table plus lookup and route-grammar helpers.

use schemars::Schema;
use serde::{Deserialize, Serialize};

mod routers;

/// Whether a route is served by a data fetcher or by a compute/derivation step.
///
/// `Fetch` routes resolve to a concrete `(provider, endpoint)` fetcher in the
/// ingest dispatch table. `Compute` routes derive their result from other
/// routes' output (e.g. a technical indicator over a price series) and carry no
/// provider candidates. WS0 ships only `Fetch` routes; `Compute` is part of the
/// shape now so later stories can add derivations without a schema migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndpointKind {
    /// Served by a concrete provider fetcher via the ingest dispatch table.
    Fetch,
    /// Derived by a compute step over other routes' output.
    Compute,
}

/// A concrete provider endpoint that can satisfy a logical route.
///
/// `provider`/`endpoint` are the exact keys used in the ingest dispatch table,
/// so a resolved candidate can be looked up there directly. This mirrors the
/// `ProviderCandidate` the original `provider_resolve` layer used, lifted into
/// the catalog so the resolver can delegate here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCandidate {
    /// Provider key, e.g. `"fileset"`, `"yahoo"`, `"polygon"`.
    pub provider: &'static str,
    /// Concrete endpoint key, e.g. `"equity_historical"`, `"aggregates"`.
    pub endpoint: &'static str,
}

impl ProviderCandidate {
    /// Construct a candidate in a `const` context (router tables are `const fn`).
    #[must_use]
    pub const fn new(provider: &'static str, endpoint: &'static str) -> Self {
        Self { provider, endpoint }
    }
}

/// One logical route in the namespaced endpoint catalog.
///
/// The `params_schema` and `model` closures return JSON schemas (the same
/// `schemars::Schema` shape the protocol/agent schema bundles emit) so callers
/// can introspect a route's inputs and outputs without compiling against the
/// concrete Rust types. `candidates` is ordered by fallback preference —
/// keyless/offline-first leads — and is empty for `Compute` routes.
#[derive(Clone)]
pub struct CatalogEntry {
    /// Slash-namespaced logical route, e.g. `"equity/price/historical"`.
    pub route: &'static str,
    /// Whether the route is fetched from a provider or computed.
    pub kind: EndpointKind,
    /// Parameter schema for the route's request (the standardized query params).
    pub params_schema: fn() -> Schema,
    /// Result data-model schema for one standardized record.
    pub model: fn() -> Schema,
    /// Ordered candidate `(provider, endpoint)` pairs; declaration order is the
    /// no-explicit-provider fallback preference. Empty for `Compute` routes.
    pub candidates: &'static [ProviderCandidate],
    /// Bronze landing table for the route's records, when it persists. `None`
    /// for read-only / no-persist routes.
    pub bronze_table: Option<&'static str>,
    /// One-line human-readable description of the route.
    pub doc: &'static str,
    /// Whether the route's result is naturally chartable (e.g. an OHLC series).
    pub chartable: bool,
}

/// Separator between route segments (slash-namespaced form).
pub const ROUTE_SEP: char = '/';

/// The complete, deterministic endpoint catalog for this build.
///
/// Aggregates every router module in a fixed order. The table is
/// build-independent (no feature gates): availability is filtered by the
/// consumer against the runtime ingest dispatch table. Entries are returned in
/// a stable, deterministic order (router order, then each router's declaration
/// order), so snapshots and generated artifacts are reproducible.
#[must_use]
pub fn catalog() -> Vec<CatalogEntry> {
    let mut entries = Vec::new();
    entries.extend(routers::equity::entries());
    entries.extend(routers::crypto::entries());
    entries.extend(routers::index::entries());
    entries.extend(routers::fixedincome::entries());
    entries.extend(routers::etf::entries());
    entries.extend(routers::derivatives::entries());
    entries.extend(routers::currency::entries());
    entries.extend(routers::economy::entries());
    entries.extend(routers::commodity::entries());
    entries.extend(routers::news::entries());
    entries.extend(routers::regulators::entries());
    entries.extend(routers::uscongress::entries());
    entries.extend(routers::technical::entries());
    entries.extend(routers::quantitative::entries());
    entries.extend(routers::econometrics::entries());
    entries.extend(routers::forecast::entries());
    entries.extend(routers::portfolio::entries());
    entries.extend(routers::imf_utils::entries());
    entries
}

/// Look up a single catalog entry by its exact slash-namespaced route.
///
/// Returns `None` when no route matches. Lookup is by exact string equality, so
/// the caller must pass a fully-qualified route (e.g. `equity/price/historical`).
#[must_use]
pub fn lookup(route: &str) -> Option<CatalogEntry> {
    catalog().into_iter().find(|entry| entry.route == route)
}

/// Validate that `route` conforms to the catalog route grammar.
///
/// A valid route is one or more lowercase segments separated by `'/'`, where
/// each segment is non-empty and made of `[a-z0-9_]`. This is the grammar all
/// catalog routes (and any caller-supplied logical endpoint) must satisfy;
/// the `catalog_routes_obey_grammar` test enforces it across the built-in set.
#[must_use]
pub fn is_valid_route(route: &str) -> bool {
    if route.is_empty() {
        return false;
    }
    let mut segments = 0usize;
    for segment in route.split(ROUTE_SEP) {
        if segment.is_empty() {
            return false;
        }
        if !segment
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return false;
        }
        segments += 1;
    }
    segments >= 1
}

/// Derive the canonical dispatch-table `endpoint` key for a single-series
/// catalog route by replacing each route separator with `'_'`.
///
/// The FRED macro/rate cluster routes map one-to-one onto a `(provider="fred",
/// endpoint=<this key>)` dispatch binding. Keeping the derivation here — beside
/// the route grammar it depends on — lets the catalog routers, the runtime
/// dispatch table, and the conformance tests agree on one spelling without any
/// crate depending on the FRED provider. Example: `economy/cpi` → `economy_cpi`,
/// `fixedincome/rate/sofr` → `fixedincome_rate_sofr`.
#[must_use]
pub fn endpoint_key_for_route(route: &str) -> String {
    route.replace(ROUTE_SEP, "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn route_grammar_accepts_lowercase_segments() {
        assert!(is_valid_route("equity/price/historical"));
        assert!(is_valid_route("crypto/price/historical"));
        assert!(is_valid_route("economy/cpi"));
        assert!(is_valid_route("news"));
        assert!(is_valid_route("equity/fundamental/balance_sheet"));
    }

    #[test]
    fn route_grammar_rejects_malformed_routes() {
        assert!(!is_valid_route(""));
        assert!(!is_valid_route("/equity"));
        assert!(!is_valid_route("equity/"));
        assert!(!is_valid_route("equity//price"));
        assert!(!is_valid_route("Equity/price"));
        assert!(!is_valid_route("equity/price/HIST"));
        assert!(!is_valid_route("equity/price/hist-orical"));
        assert!(!is_valid_route("equity price"));
    }

    #[test]
    fn catalog_routes_obey_grammar() {
        for entry in catalog() {
            assert!(
                is_valid_route(entry.route),
                "route {:?} violates the catalog grammar",
                entry.route
            );
        }
    }

    #[test]
    fn catalog_routes_are_unique() {
        let mut seen = BTreeSet::new();
        for entry in catalog() {
            assert!(
                seen.insert(entry.route),
                "duplicate catalog route: {:?}",
                entry.route
            );
        }
    }

    #[test]
    fn catalog_order_is_deterministic() {
        let first: Vec<&'static str> = catalog().iter().map(|e| e.route).collect();
        let second: Vec<&'static str> = catalog().iter().map(|e| e.route).collect();
        assert_eq!(first, second, "catalog() must be deterministic");
    }

    #[test]
    fn fetch_routes_have_candidates_compute_routes_do_not() {
        for entry in catalog() {
            match entry.kind {
                EndpointKind::Fetch => assert!(
                    !entry.candidates.is_empty(),
                    "Fetch route {:?} must have at least one candidate",
                    entry.route
                ),
                EndpointKind::Compute => assert!(
                    entry.candidates.is_empty(),
                    "Compute route {:?} must have no provider candidates",
                    entry.route
                ),
            }
        }
    }

    #[test]
    fn lookup_finds_seeded_routes_and_misses_unknown() {
        assert!(lookup("equity/price/historical").is_some());
        assert!(lookup("crypto/price/historical").is_some());
        assert!(lookup("index/price/historical").is_some());
        assert!(lookup("does/not/exist").is_none());
    }

    #[test]
    fn chartable_flag_set_on_known_charting_routes() {
        // G014 sanity: the OHLCV equity price route and the fixed-income
        // single-series routes are chartable; every technical/* Compute route is
        // chartable (set at construction in the technical router). The
        // quantitative/* and econometrics/* Compute routes (G015) summarize a
        // whole series into a single figure / coefficient table, so they are
        // intentionally NOT chartable.
        assert!(
            lookup("equity/price/historical")
                .expect("equity route")
                .chartable
        );
        assert!(
            lookup("fixedincome/government/yield_curve")
                .expect("yield curve route")
                .chartable
        );
        for entry in catalog() {
            if entry.kind == EndpointKind::Compute && entry.route.starts_with("technical/") {
                assert!(
                    entry.chartable,
                    "technical Compute route {:?} should be chartable",
                    entry.route
                );
            }
        }
        // The non-technical analytics Compute routes are not chartable.
        for namespace in ["quantitative/", "econometrics/", "portfolio/"] {
            for entry in catalog() {
                if entry.route.starts_with(namespace) {
                    assert!(
                        !entry.chartable,
                        "Compute route {:?} should not be chartable",
                        entry.route
                    );
                }
            }
        }
    }

    #[test]
    fn equity_candidate_order_matches_legacy_resolver() {
        // The equity route's candidate order is load-bearing: the offline
        // fixtures (`fileset`, then `yahoo`) must lead so an unkeyed default
        // build resolves network-free, exactly as the legacy resolver did.
        let equity = lookup("equity/price/historical").expect("equity route present");
        let providers: Vec<&'static str> = equity.candidates.iter().map(|c| c.provider).collect();
        assert_eq!(
            providers,
            vec![
                "fileset",
                "yahoo",
                "fmp",
                "tiingo",
                "polygon",
                "alpaca",
                "alpha_vantage",
                "databento",
                "akshare",
            ]
        );
    }
}
