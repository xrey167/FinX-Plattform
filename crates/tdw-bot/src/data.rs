//! The data-access seam: the abstract trait the core router fetches through.
//!
//! The core never talks to a daemon directly — it calls [`DataSource`], an
//! abstract trait. The production transport wires a real REST/catalog-backed
//! implementation; the unit tests inject a [`FakeDataSource`] so the whole core
//! (parser, router, formatting) is exercised offline with deterministic data.

use tdw_domain::MarketDataBar;

/// A single recent-headline record returned for `/news`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewsItem {
    /// Headline text.
    pub title: String,
    /// Publication or source name.
    pub source: String,
    /// Publication date/time label (as the provider supplied it).
    pub published: String,
}

/// A latest-price snapshot returned for `/quote`.
#[derive(Clone, Debug, PartialEq)]
pub struct Quote {
    /// The resolved ticker symbol.
    pub symbol: String,
    /// Latest traded/last price.
    pub price: f64,
    /// Absolute change over the comparison window, when known.
    pub change: Option<f64>,
    /// Percent change over the comparison window, when known.
    pub change_percent: Option<f64>,
    /// Currency code (e.g. `"USD"`), when known.
    pub currency: Option<String>,
}

/// A data-access failure surfaced to the router (which renders it as a text
/// reply). Kept as a readable message rather than a typed error tree because the
/// bot only ever shows it to a human.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataError {
    /// Human-readable failure reason.
    pub message: String,
}

impl DataError {
    /// Build a data error from a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DataError {}

/// The abstract data seam the core router fetches through.
///
/// Implementors map a ticker to the warehouse's catalog/REST routes
/// (`equity/price/quote`, `news/company`, `equity/price/historical`). The core
/// depends only on this trait, so it is fully testable with a fake and never
/// hard-requires a live daemon. Methods are synchronous: a production adapter
/// runs the router on a blocking task, keeping the async transport plumbing in
/// the adapter (no business logic) and the core runtime-free.
pub trait DataSource {
    /// Fetch a latest-price snapshot for `ticker`.
    ///
    /// # Errors
    ///
    /// Returns a [`DataError`] when the underlying fetch fails or the ticker is
    /// unknown.
    fn quote(&self, ticker: &str) -> Result<Quote, DataError>;

    /// Fetch recent headlines for `ticker` (most recent first).
    ///
    /// # Errors
    ///
    /// Returns a [`DataError`] when the underlying fetch fails.
    fn news(&self, ticker: &str) -> Result<Vec<NewsItem>, DataError>;

    /// Fetch the recent price series (OHLCV bars) for `ticker`, oldest first.
    ///
    /// # Errors
    ///
    /// Returns a [`DataError`] when the underlying fetch fails.
    fn history(&self, ticker: &str) -> Result<Vec<MarketDataBar>, DataError>;
}

#[cfg(test)]
pub mod fake {
    //! In-memory test double for the data seam (test builds only).

    use super::{DataError, DataSource, MarketDataBar, NewsItem, Quote};
    use std::collections::BTreeMap;

    /// A deterministic in-memory [`DataSource`] for the core unit tests.
    ///
    /// Unknown tickers (those not seeded) yield a [`DataError`], so tests can
    /// drive both the happy path and the fetch-error path without a daemon.
    #[derive(Default)]
    pub struct FakeDataSource {
        quotes: BTreeMap<String, Quote>,
        news: BTreeMap<String, Vec<NewsItem>>,
        history: BTreeMap<String, Vec<MarketDataBar>>,
    }

    impl FakeDataSource {
        #[must_use]
        pub fn with_quote(mut self, ticker: &str, quote: Quote) -> Self {
            self.quotes.insert(ticker.to_string(), quote);
            self
        }

        #[must_use]
        pub fn with_news(mut self, ticker: &str, items: Vec<NewsItem>) -> Self {
            self.news.insert(ticker.to_string(), items);
            self
        }

        #[must_use]
        pub fn with_history(mut self, ticker: &str, bars: Vec<MarketDataBar>) -> Self {
            self.history.insert(ticker.to_string(), bars);
            self
        }
    }

    impl DataSource for FakeDataSource {
        fn quote(&self, ticker: &str) -> Result<Quote, DataError> {
            self.quotes
                .get(ticker)
                .cloned()
                .ok_or_else(|| DataError::new(format!("no quote for {ticker}")))
        }

        fn news(&self, ticker: &str) -> Result<Vec<NewsItem>, DataError> {
            self.news
                .get(ticker)
                .cloned()
                .ok_or_else(|| DataError::new(format!("no news for {ticker}")))
        }

        fn history(&self, ticker: &str) -> Result<Vec<MarketDataBar>, DataError> {
            self.history
                .get(ticker)
                .cloned()
                .ok_or_else(|| DataError::new(format!("no history for {ticker}")))
        }
    }
}
