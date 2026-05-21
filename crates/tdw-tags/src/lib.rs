#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDefinition {
    pub tag_id: String,
    pub parent: Option<String>,
    pub ttl_days: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagAssignment {
    pub entity_id: String,
    pub tag_id: String,
    pub assigned_at: String,
    pub expires_at: Option<String>,
    pub provenance: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TagError {
    #[error("tag cycle detected at {0}")]
    Cycle(String),
    #[error("unknown tag: {0}")]
    UnknownTag(String),
}

#[derive(Clone, Debug, Default)]
pub struct TagStore {
    definitions: BTreeMap<String, TagDefinition>,
    assignments: Vec<TagAssignment>,
}

impl TagStore {
    pub fn define(&mut self, definition: TagDefinition) -> Result<(), TagError> {
        self.definitions
            .insert(definition.tag_id.clone(), definition.clone());
        if self.has_cycle(&definition.tag_id) {
            self.definitions.remove(&definition.tag_id);
            return Err(TagError::Cycle(definition.tag_id));
        }
        Ok(())
    }

    pub fn assign(&mut self, assignment: TagAssignment) -> Result<(), TagError> {
        if !self.definitions.contains_key(&assignment.tag_id) {
            return Err(TagError::UnknownTag(assignment.tag_id));
        }
        self.assignments.push(assignment);
        Ok(())
    }

    pub fn active_tags(&self, entity_id: &str, as_of: &str) -> Vec<String> {
        self.assignments
            .iter()
            .filter(|assignment| assignment.entity_id == entity_id)
            .filter(|assignment| {
                assignment
                    .expires_at
                    .as_deref()
                    .is_none_or(|expires| expires > as_of)
            })
            .map(|assignment| assignment.tag_id.clone())
            .collect()
    }

    pub fn taxonomy_stats(&self) -> BTreeMap<String, usize> {
        let mut stats = BTreeMap::new();
        for assignment in &self.assignments {
            *stats.entry(assignment.tag_id.clone()).or_insert(0) += 1;
        }
        stats
    }

    pub fn assignments(&self) -> &[TagAssignment] {
        &self.assignments
    }

    fn has_cycle(&self, start: &str) -> bool {
        let mut seen = BTreeSet::new();
        let mut current = Some(start.to_string());
        while let Some(tag_id) = current {
            if !seen.insert(tag_id.clone()) {
                return true;
            }
            current = self
                .definitions
                .get(&tag_id)
                .and_then(|definition| definition.parent.clone());
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manages_tag_dag_ttl_provenance_and_stats() {
        let mut store = TagStore::default();
        store
            .define(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        store
            .define(TagDefinition {
                tag_id: "style:momentum".to_string(),
                parent: Some("asset:equity".to_string()),
                ttl_days: Some(30),
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        store
            .assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "style:momentum".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: Some("2026-06-20".to_string()),
                provenance: "rule:price_momentum".to_string(),
            })
            .unwrap_or_else(|error| panic!("assignment should persist: {error}"));

        assert_eq!(
            store.active_tags("instrument:AAPL", "2026-05-22"),
            vec!["style:momentum".to_string()]
        );
        assert!(
            store
                .active_tags("instrument:AAPL", "2026-07-01")
                .is_empty()
        );
        assert_eq!(store.taxonomy_stats().get("style:momentum"), Some(&1));
        assert_eq!(store.assignments()[0].provenance, "rule:price_momentum");
    }
}
