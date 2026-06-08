//! [`QuoteSource`] trait and [`StaticQuoteSource`] test double.

use std::collections::BTreeMap;

use async_trait::async_trait;
use tdw_domain::QuoteSnapshot;

// ---------------------------------------------------------------------------
// QuoteSource
// ---------------------------------------------------------------------------

/// Abstraction over a live quote feed.
///
/// # Production wiring
///
/// Production code should implement this trait against the daemon's
/// `GetQuoteSnapshot` dispatch or a provider fetcher (e.g.
/// `FmpHttpQuoteSnapshotFetcher`).  Neither adapter belongs in this crate —
/// they live in service-wiring code that depends on the concrete provider
/// crates and the `http` feature.  This keeps `tdw-alert-evaluator` offline
/// and free of network dependencies.
///
/// # Error model
///
/// A `String` error is intentionally lightweight: the caller logs and skips
/// the symbol rather than failing the whole run, so a rich error type adds no
/// value at this boundary.
#[async_trait]
pub trait QuoteSource: Send + Sync {
    /// Fetch the current [`QuoteSnapshot`] for `symbol`.
    ///
    /// Returns `Err(String)` on any transient or permanent failure.  The
    /// evaluation function logs the error and skips the symbol — it does not
    /// fail the run.
    async fn quote(&self, symbol: &str) -> Result<QuoteSnapshot, String>;
}

// ---------------------------------------------------------------------------
// StaticQuoteSource
// ---------------------------------------------------------------------------

/// An in-memory [`QuoteSource`] backed by a pre-loaded [`BTreeMap`].
///
/// Construct with [`StaticQuoteSource::new`] and pass a map of normalised
/// symbol → [`QuoteSnapshot`].  Symbols not present in the map return an
/// `Err` with a descriptive message.
///
/// Used in unit tests and offline scenarios where no live feed is available.
#[derive(Debug, Default, Clone)]
pub struct StaticQuoteSource {
    quotes: BTreeMap<String, QuoteSnapshot>,
}

impl StaticQuoteSource {
    /// Create a new source from a pre-built map.
    #[must_use]
    pub const fn new(quotes: BTreeMap<String, QuoteSnapshot>) -> Self {
        Self { quotes }
    }
}

#[async_trait]
impl QuoteSource for StaticQuoteSource {
    async fn quote(&self, symbol: &str) -> Result<QuoteSnapshot, String> {
        self.quotes
            .get(symbol)
            .cloned()
            .ok_or_else(|| format!("no quote available for symbol `{symbol}`"))
    }
}
