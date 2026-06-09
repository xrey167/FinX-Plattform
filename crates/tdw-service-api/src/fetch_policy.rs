//! Pure cache-tier fetch-policy decision layer for market-data fetches.
//!
//! This module answers a single question, independently of any I/O: *given a
//! data kind's freshness [`CacheTier`] and when it was last fetched, should we
//! fetch a fresh value or serve the cached one?* It holds no state, performs no
//! network or clock access (the caller supplies `now_ms`), and is wired into no
//! handler yet — it is a self-contained policy layer made available for future
//! callers, mirroring how the news/watchlist compose layers were added as pure
//! logic ahead of full wiring.

use serde::{Deserialize, Serialize};

use crate::policy::ServiceEndpoint;

/// Freshness tier for a fetched data kind. Each tier maps to a cache TTL:
/// how long a previously-fetched value may be served before a refetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheTier {
    /// Always refetch; never serve cached (e.g. live quote snapshots).
    Realtime,
    /// Short TTL for intraday series.
    Intraday,
    /// Long TTL; refreshes about once per trading day (e.g. EOD bars).
    EndOfDay,
    /// Effectively static reference data (e.g. company profile, symbology).
    Reference,
}

impl CacheTier {
    /// Cache time-to-live in milliseconds for this tier: how long a previously
    /// fetched value may be served before a refetch is required.
    ///
    /// These are reasonable defaults, not a tuned policy:
    /// - [`CacheTier::Realtime`] => `0` (never serve cached; always refetch).
    /// - [`CacheTier::Intraday`] => `60_000` (60 seconds).
    /// - [`CacheTier::EndOfDay`] => `21_600_000` (6 hours).
    /// - [`CacheTier::Reference`] => `604_800_000` (7 days).
    #[must_use]
    pub const fn ttl_ms(self) -> i64 {
        match self {
            Self::Realtime => 0,
            Self::Intraday => 60_000,
            Self::EndOfDay => 6 * 60 * 60 * 1000,
            Self::Reference => 7 * 24 * 60 * 60 * 1000,
        }
    }

    /// Map a [`ServiceEndpoint`] to its cache freshness tier.
    ///
    /// Only `EquityHistorical` is a cacheable data fetch (end-of-day bars), so
    /// it maps to [`CacheTier::EndOfDay`]. Every non-data endpoint
    /// (`RunQuery`, `IngestBatch`, `ToolCall`, `UdfRun`, `AlertManage`,
    /// `UserRegister`) is not a cached fetch and maps to [`CacheTier::Realtime`]
    /// so it is never served from cache by this layer.
    #[must_use]
    pub const fn for_endpoint(endpoint: ServiceEndpoint) -> Self {
        match endpoint {
            ServiceEndpoint::EquityHistorical => Self::EndOfDay,
            ServiceEndpoint::RunQuery
            | ServiceEndpoint::IngestBatch
            | ServiceEndpoint::ToolCall
            | ServiceEndpoint::UdfRun
            | ServiceEndpoint::AlertManage
            | ServiceEndpoint::UserRegister => Self::Realtime,
        }
    }
}

/// The outcome of a cache-freshness decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchDecision {
    /// Fetch a fresh value; do not serve the cached one.
    Fetch,
    /// Serve the cached value, which is `age_ms` milliseconds old.
    ServeCached {
        /// Age of the cached value in milliseconds (`now_ms - last_fetched_at_ms`).
        age_ms: i64,
    },
}

/// Decide whether to fetch fresh data or serve a cached value.
///
/// Logic:
/// - No prior fetch (`last_fetched_at_ms` is `None`) => [`FetchDecision::Fetch`].
/// - Negative age (`now_ms < last_fetched_at_ms`, i.e. clock skew) =>
///   [`FetchDecision::Fetch`] (safe default).
/// - [`CacheTier::Realtime`] (TTL `0`) => always [`FetchDecision::Fetch`].
/// - Age at or beyond the tier TTL => [`FetchDecision::Fetch`].
/// - Otherwise => [`FetchDecision::ServeCached`] carrying the computed age.
#[must_use]
pub const fn decide(
    tier: CacheTier,
    now_ms: i64,
    last_fetched_at_ms: Option<i64>,
) -> FetchDecision {
    let Some(last) = last_fetched_at_ms else {
        return FetchDecision::Fetch;
    };
    let age = now_ms - last;
    if age < 0 || tier.ttl_ms() == 0 || age >= tier.ttl_ms() {
        FetchDecision::Fetch
    } else {
        FetchDecision::ServeCached { age_ms: age }
    }
}

/// Convenience predicate: `true` when [`decide`] would serve the cached value.
#[must_use]
pub const fn is_fresh(tier: CacheTier, now_ms: i64, last_fetched_at_ms: Option<i64>) -> bool {
    matches!(
        decide(tier, now_ms, last_fetched_at_ms),
        FetchDecision::ServeCached { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_ms_matches_documented_defaults() {
        assert_eq!(CacheTier::Realtime.ttl_ms(), 0);
        assert_eq!(CacheTier::Intraday.ttl_ms(), 60_000);
        assert_eq!(CacheTier::EndOfDay.ttl_ms(), 21_600_000);
        assert_eq!(CacheTier::Reference.ttl_ms(), 604_800_000);
    }

    #[test]
    fn no_prior_fetch_always_fetches() {
        assert_eq!(
            decide(CacheTier::Reference, 1_000, None),
            FetchDecision::Fetch
        );
        assert_eq!(
            decide(CacheTier::EndOfDay, 1_000, None),
            FetchDecision::Fetch
        );
        assert!(!is_fresh(CacheTier::Reference, 1_000, None));
    }

    #[test]
    fn realtime_always_fetches_even_with_recent_value() {
        // Fetched 1ms ago, yet Realtime never serves cached.
        assert_eq!(
            decide(CacheTier::Realtime, 1_000, Some(999)),
            FetchDecision::Fetch
        );
        assert!(!is_fresh(CacheTier::Realtime, 1_000, Some(999)));
    }

    #[test]
    fn serves_cached_within_ttl_with_correct_age() {
        // Intraday TTL is 60s; fetched 30s ago.
        let now = 100_000;
        let last = now - 30_000;
        assert_eq!(
            decide(CacheTier::Intraday, now, Some(last)),
            FetchDecision::ServeCached { age_ms: 30_000 }
        );
        assert!(is_fresh(CacheTier::Intraday, now, Some(last)));
    }

    #[test]
    fn fetches_exactly_at_ttl_boundary() {
        // age == ttl => Fetch (boundary is inclusive on the fetch side).
        let now = 100_000;
        let last = now - CacheTier::Intraday.ttl_ms();
        assert_eq!(
            decide(CacheTier::Intraday, now, Some(last)),
            FetchDecision::Fetch
        );
    }

    #[test]
    fn fetches_past_ttl_boundary() {
        let now = 100_000;
        let last = now - CacheTier::Intraday.ttl_ms() - 1;
        assert_eq!(
            decide(CacheTier::Intraday, now, Some(last)),
            FetchDecision::Fetch
        );
    }

    #[test]
    fn just_under_ttl_serves_cached() {
        let now = 100_000;
        let age = CacheTier::Intraday.ttl_ms() - 1;
        let last = now - age;
        assert_eq!(
            decide(CacheTier::Intraday, now, Some(last)),
            FetchDecision::ServeCached { age_ms: age }
        );
    }

    #[test]
    fn negative_age_clock_skew_fetches() {
        // Last fetch is in the "future" relative to now.
        assert_eq!(
            decide(CacheTier::Reference, 1_000, Some(2_000)),
            FetchDecision::Fetch
        );
        assert!(!is_fresh(CacheTier::Reference, 1_000, Some(2_000)));
    }

    #[test]
    fn end_of_day_long_ttl_serves_cached() {
        // Fetched 5 hours ago, EOD TTL is 6 hours.
        let now = 1_000_000_000;
        let last = now - 5 * 60 * 60 * 1000;
        assert_eq!(
            decide(CacheTier::EndOfDay, now, Some(last)),
            FetchDecision::ServeCached {
                age_ms: 5 * 60 * 60 * 1000
            }
        );
        assert!(is_fresh(CacheTier::EndOfDay, now, Some(last)));
    }

    #[test]
    fn reference_long_ttl_serves_cached() {
        // Fetched 6 days ago, Reference TTL is 7 days.
        let now = 1_000_000_000_000;
        let last = now - 6 * 24 * 60 * 60 * 1000;
        assert_eq!(
            decide(CacheTier::Reference, now, Some(last)),
            FetchDecision::ServeCached {
                age_ms: 6 * 24 * 60 * 60 * 1000
            }
        );
        assert!(is_fresh(CacheTier::Reference, now, Some(last)));
    }

    #[test]
    fn for_endpoint_maps_equity_historical_to_end_of_day() {
        assert_eq!(
            CacheTier::for_endpoint(ServiceEndpoint::EquityHistorical),
            CacheTier::EndOfDay
        );
    }

    #[test]
    fn for_endpoint_maps_non_data_endpoints_to_realtime() {
        for endpoint in [
            ServiceEndpoint::RunQuery,
            ServiceEndpoint::IngestBatch,
            ServiceEndpoint::ToolCall,
            ServiceEndpoint::UdfRun,
            ServiceEndpoint::AlertManage,
        ] {
            assert_eq!(CacheTier::for_endpoint(endpoint), CacheTier::Realtime);
        }
    }
}
