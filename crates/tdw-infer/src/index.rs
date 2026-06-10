//! The derivation index: which rule derived each fact, and on what support
//! (knowledge-system B7).
//!
//! Maps a fact key (`edge:<from>-><type>-><to>` or `tag:<entity>:<tag>`) to the
//! [`Derivation`] that produced it. The support set is the list of input fact keys the
//! match consumed, so [`crate::InferEngine::retract`] can chase a transitive support
//! closure. Persistence is the CALLER's concern — [`DerivationIndex::to_json`] /
//! [`DerivationIndex::from_json`] round-trip and NOTHING here touches the filesystem.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::InferError;

/// One derived fact's lineage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derivation {
    /// The rule that derived the fact.
    pub rule_id: String,
    /// The rule-set version that fired.
    pub version: u64,
    /// Fact keys of the inputs the match consumed (the support set).
    pub support: Vec<String>,
}

/// Fact-key → derivation map, maintained by `run_full`/`run_incremental`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationIndex {
    entries: BTreeMap<String, Derivation>,
}

/// The canonical key for a derived edge fact.
#[must_use]
pub fn edge_key(from: &str, derived_type: &str, to: &str) -> String {
    format!("edge:{from}->{derived_type}->{to}")
}

/// The canonical key for a derived tag-assignment fact.
#[must_use]
pub fn tag_key(entity: &str, tag: &str) -> String {
    format!("tag:{entity}:{tag}")
}

impl DerivationIndex {
    /// Whether a fact key is already recorded as derived.
    #[must_use]
    pub fn contains(&self, fact_key: &str) -> bool {
        self.entries.contains_key(fact_key)
    }

    /// The derivation recorded for `fact_key`, if any.
    #[must_use]
    pub fn get(&self, fact_key: &str) -> Option<&Derivation> {
        self.entries.get(fact_key)
    }

    /// Record (or overwrite) the derivation of `fact_key`.
    pub fn record(&mut self, fact_key: String, derivation: Derivation) {
        self.entries.insert(fact_key, derivation);
    }

    /// Drop a fact key's derivation (used during retraction).
    pub fn remove(&mut self, fact_key: &str) {
        self.entries.remove(fact_key);
    }

    /// Number of recorded derived facts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no derived facts are recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over every `(fact_key, derivation)` pair (sorted by key).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Derivation)> {
        self.entries.iter()
    }

    /// Serialize for persistence.
    ///
    /// # Errors
    ///
    /// Returns [`InferError::Serde`] if serialization fails.
    pub fn to_json(&self) -> Result<String, InferError> {
        serde_json::to_string_pretty(self).map_err(|error| InferError::Serde(error.to_string()))
    }

    /// Restore a persisted index.
    ///
    /// # Errors
    ///
    /// Returns [`InferError::Serde`] on malformed input.
    pub fn from_json(raw: &str) -> Result<Self, InferError> {
        serde_json::from_str(raw).map_err(|error| InferError::Serde(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let mut index = DerivationIndex::default();
        index.record(
            edge_key("a", "exposed_to", "x"),
            Derivation {
                rule_id: "r1".to_string(),
                version: 2,
                support: vec![
                    edge_key("a", "supplier_of", "b"),
                    edge_key("b", "listed_on", "x"),
                ],
            },
        );
        index.record(
            tag_key("instrument:AAPL", "risk:sanctioned"),
            Derivation {
                rule_id: "p1".to_string(),
                version: 2,
                support: vec![tag_key("entity:owner", "risk:sanctioned")],
            },
        );

        let json = index.to_json().expect("serialize");
        let restored = DerivationIndex::from_json(&json).expect("deserialize");
        assert_eq!(restored, index);
        assert_eq!(restored.len(), 2);
        assert!(restored.contains(&edge_key("a", "exposed_to", "x")));
    }
}
