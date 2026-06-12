//! Scheduled document-feed abstractions (knowledge-system K-L6).
//!
//! This module defines the provider-agnostic seam ([`FeedSource`]) and the
//! per-feed observability status ([`FeedFreshness`]).  The cron-driven poll
//! loop lives in `tdw-backend` where `tdw-cron` is available (adding
//! `tdw-cron` here would create a dependency cycle through `tdw-worker` →
//! `tdw-service-api` → `tdw-agent-store` → `tdw-knowledge`).
//!
//! # Idempotency
//!
//! Content-hash idempotency in the K-E3 manifest makes re-polls safe by
//! construction: an item already indexed produces
//! [`crate::indexer::IndexOutcome::SkippedUnchanged`] and is counted as a
//! duplicate, never double-indexed or double-tagged.
//!
//! # Backoff
//!
//! A fetch error (network, auth, parse) starts a per-feed backoff counter in
//! the backend-level spawn loop. After `MAX_CONSECUTIVE_ERRORS` consecutive
//! errors the feed logs loudly and skips until the next cron slot.
//!
//! # Post-ingest inference
//!
//! Inference fires automatically as a post-condition of [`crate::indexer::KnowledgeIndexer`]
//! ingest (K-L1 hook). The feed task goes through the indexer directly; no
//! separate inference call is needed.

#![forbid(unsafe_code)]

// Re-export so callers can import the Article type without a direct dep on
// tdw-news-compose.
pub use tdw_news_compose::Article;

// ---------------------------------------------------------------------------
// FeedSource — the provider-agnostic seam
// ---------------------------------------------------------------------------

/// A provider-agnostic poll source for one feed slot.
///
/// Implementations are responsible for:
/// - Fetching at most `max_items` articles per call.
/// - Returning an empty `Vec` when no new items are available.
/// - Propagating transient errors so the caller can record them and back off.
///
/// The trait is `Send + Sync + 'static` so it can be stored behind an `Arc`
/// and shared with the spawned tokio task.
#[async_trait::async_trait]
pub trait FeedSource: Send + Sync + 'static {
    /// Poll the source and return up to `max_items` articles.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error string on any transient failure (network,
    /// auth, parse). The feed task records the error and applies backoff.
    async fn poll(&self, max_items: usize) -> Result<Vec<Article>, String>;

    /// Human-readable description of this source (for log lines).
    fn description(&self) -> &str;
}

// ---------------------------------------------------------------------------
// FixtureFeedSource — offline/test source backed by an in-repo article list
// ---------------------------------------------------------------------------

/// An offline fixture feed source backed by a static article list.
///
/// Used in tests and in CI so the always-run test suite never touches the
/// network. Articles are returned in declaration order, capped at `max_items`.
pub struct FixtureFeedSource {
    articles: Vec<Article>,
    description: String,
}

impl FixtureFeedSource {
    /// Build a fixture source from a pre-built article list.
    #[must_use]
    pub fn new(articles: Vec<Article>, description: impl Into<String>) -> Self {
        Self {
            articles,
            description: description.into(),
        }
    }

    /// An empty fixture (zero articles). Returns an empty `Vec` on every poll.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Vec::new(), "empty-fixture")
    }
}

#[async_trait::async_trait]
impl FeedSource for FixtureFeedSource {
    async fn poll(&self, max_items: usize) -> Result<Vec<Article>, String> {
        Ok(self.articles.iter().take(max_items).cloned().collect())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

// ---------------------------------------------------------------------------
// FeedFreshness — per-feed observability status
// ---------------------------------------------------------------------------

/// Freshness / observability status for one scheduled feed (K-L6).
///
/// Surfaced as part of [`crate::runtime::KgStatus`] so operators see at a
/// glance whether each feed is polling successfully.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum FeedFreshness {
    /// The feed is configured and enabled but has not fired yet.
    Pending {
        /// The configured feed id.
        feed_id: String,
    },
    /// The last poll completed without error.
    Ok {
        /// Epoch-ms timestamp of the last successful poll.
        last_poll_ms: i64,
        /// The feed id.
        feed_id: String,
        /// Number of new documents indexed in the last poll.
        indexed: usize,
        /// Number of items skipped (already indexed — idempotent re-poll).
        duplicates: usize,
    },
    /// The last poll returned a fetch error; the feed is backing off.
    Error {
        /// Epoch-ms timestamp of the last poll attempt.
        last_poll_ms: i64,
        /// The feed id.
        feed_id: String,
        /// The error message from the last fetch failure.
        error: String,
        /// How many consecutive errors have occurred.
        consecutive_errors: u32,
    },
    /// The feed is configured but `enabled = false`; no task is running.
    Disabled {
        /// The feed id.
        feed_id: String,
    },
}

impl FeedFreshness {
    /// Whether this status warrants operator attention (error state).
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error { .. })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_article(url: &str) -> Article {
        Article::new(
            "Test headline",
            url,
            "TestSource",
            1_749_297_600_000_i64,
            "Test summary.",
            vec!["AAPL".to_string()],
        )
    }

    // ── FixtureFeedSource ────────────────────────────────────────────────────

    #[tokio::test]
    async fn fixture_source_returns_up_to_max_items() {
        let articles = vec![
            sample_article("https://a.com"),
            sample_article("https://b.com"),
            sample_article("https://c.com"),
        ];
        let source = FixtureFeedSource::new(articles, "test-fixture");
        let polled = source.poll(2).await.expect("poll succeeds");
        assert_eq!(polled.len(), 2);
    }

    #[tokio::test]
    async fn fixture_source_empty_returns_empty() {
        let source = FixtureFeedSource::empty();
        let polled = source.poll(50).await.expect("poll succeeds");
        assert!(polled.is_empty());
    }

    #[tokio::test]
    async fn fixture_source_description_is_readable() {
        let source = FixtureFeedSource::new(vec![], "my-feed-description");
        assert_eq!(source.description(), "my-feed-description");
    }

    // ── FeedFreshness ────────────────────────────────────────────────────────

    #[test]
    fn feed_freshness_error_is_alarm() {
        let freshness = FeedFreshness::Error {
            last_poll_ms: 0,
            feed_id: "f".to_string(),
            error: "timeout".to_string(),
            consecutive_errors: 1,
        };
        assert!(freshness.is_error());
    }

    #[test]
    fn feed_freshness_ok_is_not_alarm() {
        let freshness = FeedFreshness::Ok {
            last_poll_ms: 0,
            feed_id: "f".to_string(),
            indexed: 3,
            duplicates: 1,
        };
        assert!(!freshness.is_error());
    }

    #[test]
    fn feed_freshness_serializes_with_state_tag() {
        let freshness = FeedFreshness::Pending {
            feed_id: "feed-a".to_string(),
        };
        let json = serde_json::to_value(&freshness).expect("serializes");
        assert_eq!(json["state"], "pending");
        assert_eq!(json["feed_id"], "feed-a");
    }

    #[test]
    fn disabled_feed_freshness_is_not_error() {
        let freshness = FeedFreshness::Disabled {
            feed_id: "disabled-feed".to_string(),
        };
        assert!(!freshness.is_error());
        let json = serde_json::to_value(&freshness).expect("serializes");
        assert_eq!(json["state"], "disabled");
    }
}
