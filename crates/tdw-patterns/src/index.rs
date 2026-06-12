//! Caller-owned persistence for discovered patterns (knowledge-system K-R4).
//!
//! [`PatternIndex`] maps a pattern's canonical motif string to its
//! [`PatternRecord`]: the support count, the temporal window it was mined over,
//! and the list of instance entity-id pairs whose edges matched the motif.
//!
//! Nothing here touches the filesystem. The caller round-trips the index via
//! [`PatternIndex::to_json`] / [`PatternIndex::from_json`] and hands it back to
//! [`crate::mining::PatternEngine::run_pattern_mining_at`] on the next invocation
//! so the engine can update support counts without duplicating pattern nodes in
//! the graph.
//!
//! The graph-node ids for mined patterns are stable: `pattern:<canonical>`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::PatternError;

/// One discovered pattern's persisted record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternRecord {
    /// The canonical motif encoding (e.g. `"listed_on::supplier_of"`).
    pub canonical: String,
    /// Number of distinct entity-pair instances that matched the motif.
    pub support: usize,
    /// Temporal window this support count was computed over (`YYYY-MM-DD` or
    /// `"all"` when no window is specified).
    pub window: String,
    /// The entity-pair instances (bounded to
    /// [`MiningLimits::max_provenance_edges`] items).
    ///
    /// Each entry is `(from_entity_id, to_entity_id)` of the first and last
    /// node in the matched walk. Bounded so the record never grows unbounded.
    pub instances: Vec<(String, String)>,
    /// The graph-node id for this pattern's [`EntityKind::Pattern`] node.
    /// Stable across re-mining: `"pattern:<canonical>"`.
    pub node_id: String,
}

impl PatternRecord {
    /// Build a new record from scratch.
    ///
    /// `node_id` is derived deterministically from `canonical`.
    #[must_use]
    pub fn new(
        canonical: String,
        support: usize,
        window: String,
        instances: Vec<(String, String)>,
    ) -> Self {
        let node_id = pattern_node_id(&canonical);
        Self {
            canonical,
            support,
            window,
            instances,
            node_id,
        }
    }
}

/// The stable graph-node id for a pattern with the given canonical motif string.
#[must_use]
pub fn pattern_node_id(canonical: &str) -> String {
    // Replace "::" with "-" so the id satisfies the graph node grammar
    // `[A-Za-z0-9:._-]+` (no consecutive colons not from the "pattern:" prefix).
    let safe = canonical.replace("::", "--");
    format!("pattern:{safe}")
}

/// Caller-owned map from canonical motif string to [`PatternRecord`].
///
/// Passed back to [`crate::mining::PatternEngine::run_pattern_mining_at`] on
/// each invocation; the engine uses it for idempotent re-mining (update, not
/// duplicate).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatternIndex {
    entries: BTreeMap<String, PatternRecord>,
}

impl PatternIndex {
    /// Whether the index contains a record for `canonical`.
    #[must_use]
    pub fn contains(&self, canonical: &str) -> bool {
        self.entries.contains_key(canonical)
    }

    /// The record for `canonical`, if any.
    #[must_use]
    pub fn get(&self, canonical: &str) -> Option<&PatternRecord> {
        self.entries.get(canonical)
    }

    /// Insert or replace the record for `canonical`.
    pub fn record(&mut self, record: PatternRecord) {
        self.entries.insert(record.canonical.clone(), record);
    }

    /// Number of patterns in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate all `(canonical, record)` pairs in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PatternRecord)> {
        self.entries.iter()
    }

    /// Serialize for persistence.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError::Serde`] if serialization fails.
    pub fn to_json(&self) -> Result<String, PatternError> {
        serde_json::to_string_pretty(self).map_err(|e| PatternError::Serde(e.to_string()))
    }

    /// Restore a persisted index.
    ///
    /// # Errors
    ///
    /// Returns [`PatternError::Serde`] on malformed input.
    pub fn from_json(raw: &str) -> Result<Self, PatternError> {
        serde_json::from_str(raw).map_err(|e| PatternError::Serde(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_node_id_is_stable_and_grammar_safe() {
        let id = pattern_node_id("listed_on::supplier_of");
        assert_eq!(id, "pattern:listed_on--supplier_of");
        // Must contain only [A-Za-z0-9:._-]+ characters.
        assert!(
            id.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
        );
    }

    #[test]
    fn round_trips_through_json() {
        let mut index = PatternIndex::default();
        index.record(PatternRecord::new(
            "listed_on::supplier_of".to_string(),
            4,
            "2025-01-01".to_string(),
            vec![
                ("instrument:AAPL".to_string(), "venue:NASDAQ".to_string()),
                ("instrument:MSFT".to_string(), "venue:NASDAQ".to_string()),
            ],
        ));
        let json = index.to_json().expect("serialize");
        let restored = PatternIndex::from_json(&json).expect("deserialize");
        assert_eq!(restored, index);
        assert_eq!(restored.len(), 1);
        assert!(restored.contains("listed_on::supplier_of"));
    }
}
