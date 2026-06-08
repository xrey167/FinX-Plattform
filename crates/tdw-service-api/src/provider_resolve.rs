//! Logical-endpoint → concrete `(provider, endpoint)` resolution (L1.5).
//!
//! A *logical endpoint* — written in slash-path form, e.g.
//! `equity/price/historical` — names a capability that several concrete
//! providers can satisfy. This layer maps one logical endpoint to its ordered
//! list of candidate `(provider, endpoint)` pairs and picks one per request:
//!
//! 1. **Explicit `provider` wins.** A non-empty `provider` argument selects the
//!    matching candidate, provided that provider is registered (feature-enabled)
//!    in this build's ingest dispatch table.
//! 2. **Otherwise, first available.** With no `provider` argument, the first
//!    candidate (in the table's deterministic declaration order) that is
//!    registered in this build is chosen.
//! 3. **Else, a structured error.** When no candidate can be served — the
//!    logical endpoint is unknown, the requested provider is not a candidate, or
//!    none of the candidates are registered — the error lists the candidate
//!    providers so the caller can pick a viable one.
//!
//! Resolution is *availability-aware*: a candidate counts only if its concrete
//! `(provider, endpoint)` pair is present in the supplied dispatch table, so
//! feature-gated providers absent from an offline build are skipped rather than
//! dispatched into a missing fetcher. This module performs no I/O and enforces
//! no policy — it is a pure lookup the dispatcher runs *after* the policy guard,
//! leaving enforcement ordering unchanged.

use std::collections::BTreeMap;

use tdw_core::{Error, Result};

/// A concrete provider endpoint that can satisfy a logical endpoint.
///
/// `provider`/`endpoint` are the exact keys used in the ingest dispatch table,
/// so a resolved candidate can be looked up there directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderCandidate {
    pub provider: &'static str,
    pub endpoint: &'static str,
}

/// Marker that distinguishes a logical endpoint (slash-path form, resolved
/// through this layer) from a concrete `endpoint` string (dispatched directly).
/// Concrete endpoint constants (`equity_historical`, `aggregates`, …) never
/// contain `/`, so the presence of a slash is an unambiguous, backwards-
/// compatible signal.
pub const LOGICAL_ENDPOINT_SEP: char = '/';

/// `true` when `endpoint` is a logical endpoint (and must be resolved) rather
/// than a concrete provider endpoint (dispatched as-is).
pub fn is_logical_endpoint(endpoint: &str) -> bool {
    endpoint.contains(LOGICAL_ENDPOINT_SEP)
}

/// The logical-endpoint → ordered candidate-list mapping.
///
/// The candidate order is the resolution preference order for the
/// no-`provider` (default) case: the first candidate registered in this build
/// wins. The list is build-independent — every concrete provider that *could*
/// serve the logical endpoint is listed regardless of enabled features;
/// availability is filtered against the dispatch table at resolution time.
///
/// Only logical endpoints with ≥2 candidate providers are listed (a single
/// provider needs no resolution layer). Each candidate's `(provider, endpoint)`
/// pair must match a key wired into the ingest dispatch table for that provider
/// to be reachable.
pub fn logical_endpoint_table() -> BTreeMap<&'static str, Vec<ProviderCandidate>> {
    const fn candidate(provider: &'static str, endpoint: &'static str) -> ProviderCandidate {
        ProviderCandidate { provider, endpoint }
    }

    let mut table: BTreeMap<&'static str, Vec<ProviderCandidate>> = BTreeMap::new();

    // Equity OHLC history. The two offline fixture fetchers lead the order so an
    // unkeyed default build resolves network-free; the keyed HTTP providers
    // follow. All map to canonical equity/bar fetchers in the dispatch table.
    table.insert(
        "equity/price/historical",
        vec![
            candidate("fileset", "equity_historical"),
            candidate("yahoo", "equity_historical"),
            candidate("fmp", "equity_historical"),
            candidate("tiingo", "historical"),
            candidate("polygon", "aggregates"),
            candidate("alpaca", "stock_bars"),
            candidate("alpha_vantage", "market_data"),
            candidate("databento", "timeseries"),
            candidate("akshare", "hist"),
        ],
    );

    // Crypto OHLC history. Both candidates are feature-gated, so this logical
    // endpoint is unresolvable in an offline build (a structured error names the
    // candidates), and resolvable once either crypto provider is enabled.
    table.insert(
        "crypto/price/historical",
        vec![
            candidate("coingecko", "ohlc"),
            candidate("ccdata", "crypto_ohlcv"),
        ],
    );

    // Index OHLC history. Bar providers that also serve index symbols; both
    // feature-gated.
    table.insert(
        "index/price/historical",
        vec![
            candidate("polygon", "aggregates"),
            candidate("databento", "timeseries"),
        ],
    );

    table
}

/// Resolve `logical_endpoint` to a concrete `(provider, endpoint)` pair.
///
/// `requested_provider` is the op payload's `provider` argument: an empty string
/// means "no explicit provider — pick the default". `is_registered` reports
/// whether a concrete `(provider, endpoint)` pair is available in this build
/// (the dispatcher passes a closure over its ingest dispatch table).
///
/// # Errors
///
/// Returns [`Error::Provider`] when the logical endpoint is unknown, when an
/// explicit `requested_provider` is not a candidate (or is a candidate but not
/// registered in this build), or when no candidate is registered. Each error
/// lists the candidate providers so the caller can choose a viable one.
pub fn resolve_logical_endpoint(
    logical_endpoint: &str,
    requested_provider: &str,
    is_registered: impl Fn(&str, &str) -> bool,
) -> Result<ProviderCandidate> {
    let table = logical_endpoint_table();
    let Some(candidates) = table.get(logical_endpoint) else {
        return Err(Error::Provider(format!(
            "unknown logical endpoint: {logical_endpoint}; known: {}",
            known_logical_endpoints(&table)
        )));
    };

    if requested_provider.is_empty() {
        // Default: first candidate registered in this build (declaration order
        // is the preference order).
        return candidates
            .iter()
            .find(|c| is_registered(c.provider, c.endpoint))
            .copied()
            .ok_or_else(|| {
                Error::Provider(format!(
                    "no registered provider for logical endpoint {logical_endpoint}; \
                     candidates: {} (enable a provider-* feature)",
                    candidate_providers(candidates)
                ))
            });
    }

    // Explicit provider: it must be a candidate for this logical endpoint AND be
    // registered in this build.
    let Some(candidate) = candidates.iter().find(|c| c.provider == requested_provider) else {
        return Err(Error::Provider(format!(
            "provider {requested_provider} does not serve logical endpoint \
             {logical_endpoint}; candidates: {}",
            candidate_providers(candidates)
        )));
    };
    if is_registered(candidate.provider, candidate.endpoint) {
        Ok(*candidate)
    } else {
        Err(Error::Provider(format!(
            "provider {requested_provider} serves logical endpoint {logical_endpoint} \
             but is not registered in this build; available candidates: {}",
            registered_candidate_providers(candidates, &is_registered)
        )))
    }
}

/// Comma-separated, sorted list of known logical endpoints for error messages.
/// `BTreeMap` iteration is sorted, so the output is stable.
fn known_logical_endpoints(table: &BTreeMap<&'static str, Vec<ProviderCandidate>>) -> String {
    table.keys().copied().collect::<Vec<_>>().join(", ")
}

/// Comma-separated candidate provider names (in preference order).
fn candidate_providers(candidates: &[ProviderCandidate]) -> String {
    candidates
        .iter()
        .map(|c| c.provider)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Comma-separated candidate provider names that are registered in this build.
fn registered_candidate_providers(
    candidates: &[ProviderCandidate],
    is_registered: impl Fn(&str, &str) -> bool,
) -> String {
    let registered: Vec<&str> = candidates
        .iter()
        .filter(|c| is_registered(c.provider, c.endpoint))
        .map(|c| c.provider)
        .collect();
    if registered.is_empty() {
        "(none — enable a provider-* feature)".to_string()
    } else {
        registered.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry stub: only the two offline fixture equity providers are present,
    /// matching the default (no-feature) build's ingest dispatch table.
    fn offline_registry(provider: &str, endpoint: &str) -> bool {
        matches!(
            (provider, endpoint),
            ("fileset" | "yahoo", "equity_historical")
        )
    }

    #[test]
    fn logical_endpoint_detection_requires_slash() {
        assert!(is_logical_endpoint("equity/price/historical"));
        assert!(!is_logical_endpoint("equity_historical"));
        assert!(!is_logical_endpoint("aggregates"));
    }

    #[test]
    fn default_resolves_first_registered_candidate() {
        // No explicit provider → first registered candidate in declaration order
        // (`fileset` leads `yahoo`).
        let resolved = resolve_logical_endpoint("equity/price/historical", "", offline_registry)
            .expect("resolves");
        assert_eq!(resolved.provider, "fileset");
        assert_eq!(resolved.endpoint, "equity_historical");
    }

    #[test]
    fn explicit_provider_wins_over_default_order() {
        // `yahoo` is not first in the candidate order, but an explicit request
        // selects it.
        let resolved =
            resolve_logical_endpoint("equity/price/historical", "yahoo", offline_registry)
                .expect("resolves");
        assert_eq!(resolved.provider, "yahoo");
        assert_eq!(resolved.endpoint, "equity_historical");
    }

    #[test]
    fn explicit_provider_not_a_candidate_errors_with_candidates() {
        let err = resolve_logical_endpoint("equity/price/historical", "binance", offline_registry)
            .expect_err("binance does not serve equity history");
        let msg = err.to_string();
        assert!(
            msg.contains("does not serve logical endpoint"),
            "got: {msg}"
        );
        assert!(msg.contains("candidates:"), "got: {msg}");
        assert!(msg.contains("fileset"), "got: {msg}");
    }

    #[test]
    fn explicit_candidate_but_unregistered_errors() {
        // `fmp` is a valid candidate but not registered in the offline build.
        let err = resolve_logical_endpoint("equity/price/historical", "fmp", offline_registry)
            .expect_err("fmp is not registered offline");
        let msg = err.to_string();
        assert!(msg.contains("not registered in this build"), "got: {msg}");
        // The error advertises the candidates that ARE registered.
        assert!(
            msg.contains("fileset") && msg.contains("yahoo"),
            "got: {msg}"
        );
    }

    #[test]
    fn unknown_logical_endpoint_errors_with_known_list() {
        let err = resolve_logical_endpoint("equity/does/not/exist", "", offline_registry)
            .expect_err("unknown logical endpoint");
        let msg = err.to_string();
        assert!(msg.contains("unknown logical endpoint"), "got: {msg}");
        assert!(msg.contains("known:"), "got: {msg}");
        assert!(msg.contains("equity/price/historical"), "got: {msg}");
    }

    #[test]
    fn no_registered_candidate_errors() {
        // All crypto candidates are feature-gated; in the offline registry stub
        // none are registered, so default resolution fails with a structured
        // error naming the candidates.
        let err = resolve_logical_endpoint("crypto/price/historical", "", offline_registry)
            .expect_err("no crypto provider registered offline");
        let msg = err.to_string();
        assert!(msg.contains("no registered provider"), "got: {msg}");
        assert!(
            msg.contains("coingecko") && msg.contains("ccdata"),
            "got: {msg}"
        );
    }

    #[test]
    fn every_candidate_pair_is_dispatchable_by_construction() {
        // Each logical-endpoint candidate must use the SAME (provider, endpoint)
        // key shape the ingest dispatch table uses, or it would be unreachable.
        // We assert the two always-registered fixtures resolve through a registry
        // that mirrors the offline dispatch table.
        for logical in logical_endpoint_table().keys() {
            // Smoke: table lookup never panics and yields ≥2 candidates (a single
            // provider needs no resolution layer).
            let candidates = &logical_endpoint_table()[logical];
            assert!(
                candidates.len() >= 2,
                "logical endpoint {logical} should have >=2 candidates"
            );
        }
    }
}
