//! The durable-tags seam (knowledge-system A5).
//!
//! One async [`TagEngine`] contract over the in-process [`TagStore`] and the
//! graph-backed store, held identical by a cross-backend conformance suite
//! (in `tdw-storage-graph`, which hosts the graph implementation).
//!
//! Temporal semantics are exactly [`TagStore`]'s: assignments are half-open
//! `[assigned_at, expires_at)` windows over `YYYY-MM-DD` dates. The graph
//! backend stores them as normalized UTC timestamps; [`date_to_timestamp`]
//! is the lossless boundary mapping (`D` ⇢ `DT00:00:00Z`), under which the
//! date-level comparisons and the graph layer's lexicographic timestamp
//! comparisons agree exactly.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::{TagAssignment, TagDefinition, TagError, TagStore};

/// Map a `YYYY-MM-DD` tag date onto the graph layer's normalized UTC form.
///
/// Half-open window semantics are preserved exactly: `assigned_at <= as_of`
/// over dates equals `vf <= as_ofT00:00:00Z` over timestamps, and
/// `expires_at > as_of` equals `vt > as_ofT00:00:00Z`.
#[must_use]
pub fn date_to_timestamp(date: &str) -> String {
    format!("{date}T00:00:00Z")
}

/// The async tag-store contract shared by every backend.
#[async_trait]
pub trait TagEngine: Send + Sync {
    /// Define or update a taxonomy node (parent must exist; cycles rejected).
    async fn define(&self, definition: TagDefinition) -> Result<(), TagError>;
    /// Record a time-bounded assignment (tag must be defined).
    async fn assign(&self, assignment: TagAssignment) -> Result<(), TagError>;
    /// Tags active on `entity_id` at `as_of` (`YYYY-MM-DD`), sorted.
    async fn active_tags(&self, entity_id: &str, as_of: &str) -> Result<Vec<String>, TagError>;
    /// All transitive descendants of `tag_id`, sorted; empty for leaves.
    async fn descendants(&self, tag_id: &str) -> Result<Vec<String>, TagError>;
    /// Whether `tag_id` has a definition (the writeback gate's existence
    /// probe, knowledge-system B9).
    async fn is_defined(&self, tag_id: &str) -> Result<bool, TagError>;
    /// Entities holding `tag_id` active at `as_of`, deduplicated and sorted.
    async fn entities_with_tag(&self, tag_id: &str, as_of: &str) -> Result<Vec<String>, TagError>;
}

/// The in-process reference backend: [`TagStore`] behind a mutex, exposing
/// the async contract. Deterministic, no I/O — the conformance baseline.
#[derive(Debug, Default)]
pub struct InMemoryTagEngine {
    store: Mutex<TagStore>,
}

impl InMemoryTagEngine {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, TagStore>, TagError> {
        self.store
            .lock()
            .map_err(|error| TagError::Engine(error.to_string()))
    }
}

#[async_trait]
impl TagEngine for InMemoryTagEngine {
    async fn define(&self, definition: TagDefinition) -> Result<(), TagError> {
        self.lock()?.define(definition)
    }

    async fn assign(&self, assignment: TagAssignment) -> Result<(), TagError> {
        self.lock()?.assign(assignment)
    }

    async fn active_tags(&self, entity_id: &str, as_of: &str) -> Result<Vec<String>, TagError> {
        let mut tags = self.lock()?.active_tags(entity_id, as_of);
        tags.sort();
        tags.dedup();
        Ok(tags)
    }

    async fn descendants(&self, tag_id: &str) -> Result<Vec<String>, TagError> {
        Ok(self.lock()?.descendants(tag_id))
    }

    async fn is_defined(&self, tag_id: &str) -> Result<bool, TagError> {
        Ok(self.lock()?.is_defined(tag_id))
    }

    async fn entities_with_tag(&self, tag_id: &str, as_of: &str) -> Result<Vec<String>, TagError> {
        Ok(self.lock()?.entities_with_tag(tag_id, as_of))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_mapping_preserves_half_open_comparisons() {
        // assigned_at <= as_of (dates) <=> vf <= as_ofT00 (timestamps)
        assert!(date_to_timestamp("2026-05-21") <= date_to_timestamp("2026-05-21"));
        assert!(date_to_timestamp("2026-05-21") <= date_to_timestamp("2026-05-22"));
        // expires_at > as_of (dates) <=> vt > as_ofT00 (timestamps)
        assert!(date_to_timestamp("2026-06-20") > date_to_timestamp("2026-06-19"));
        assert!(date_to_timestamp("2026-06-20") <= date_to_timestamp("2026-06-20"));
    }

    #[tokio::test]
    async fn in_memory_engine_exposes_the_store_contract() {
        let engine = InMemoryTagEngine::default();
        engine
            .define(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .await
            .unwrap_or_else(|error| panic!("define should succeed: {error}"));
        engine
            .assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "fixture".to_string(),
            })
            .await
            .unwrap_or_else(|error| panic!("assign should succeed: {error}"));
        assert_eq!(
            engine
                .active_tags("instrument:AAPL", "2026-05-22")
                .await
                .unwrap_or_else(|error| panic!("active_tags should succeed: {error}")),
            vec!["asset:equity".to_string()]
        );
        assert_eq!(
            engine
                .entities_with_tag("asset:equity", "2026-05-22")
                .await
                .unwrap_or_else(|error| panic!("entities should succeed: {error}")),
            vec!["instrument:AAPL".to_string()]
        );
    }
}
