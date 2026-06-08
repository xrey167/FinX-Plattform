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
    #[error("unknown parent tag: {0}")]
    UnknownParent(String),
    #[error("invalid tag id: {0}")]
    InvalidTagId(String),
    #[error("invalid tag assignment")]
    InvalidAssignment,
}

#[derive(Clone, Debug, Default)]
pub struct TagStore {
    definitions: BTreeMap<String, TagDefinition>,
    assignments: Vec<TagAssignment>,
}

impl TagStore {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn define(&mut self, definition: TagDefinition) -> Result<(), TagError> {
        validate_tag_id(&definition.tag_id)?;
        if let Some(parent) = &definition.parent {
            validate_tag_id(parent)?;
            if !self.definitions.contains_key(parent) {
                return Err(TagError::UnknownParent(parent.clone()));
            }
        }
        if matches!(definition.ttl_days, Some(0)) {
            return Err(TagError::InvalidTagId(definition.tag_id));
        }
        let previous = self
            .definitions
            .insert(definition.tag_id.clone(), definition.clone());
        if self.has_cycle(&definition.tag_id) {
            // Roll back to the prior state. Removing the tag outright would drop
            // a pre-existing valid definition and leave children pointing at a
            // now-missing parent; restoring `previous` keeps the store consistent.
            match previous {
                Some(prior) => {
                    self.definitions.insert(definition.tag_id.clone(), prior);
                }
                None => {
                    self.definitions.remove(&definition.tag_id);
                }
            }
            return Err(TagError::Cycle(definition.tag_id));
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn assign(&mut self, assignment: TagAssignment) -> Result<(), TagError> {
        validate_assignment(&assignment)?;
        if !self.definitions.contains_key(&assignment.tag_id) {
            return Err(TagError::UnknownTag(assignment.tag_id));
        }
        self.assignments.push(assignment);
        Ok(())
    }

    #[must_use]
    pub fn active_tags(&self, entity_id: &str, as_of: &str) -> Vec<String> {
        self.assignments
            .iter()
            .filter(|assignment| assignment.entity_id == entity_id)
            .filter(|assignment| assignment.assigned_at.as_str() <= as_of)
            .filter(|assignment| {
                assignment
                    .expires_at
                    .as_deref()
                    .is_none_or(|expires| expires > as_of)
            })
            .map(|assignment| assignment.tag_id.clone())
            .collect()
    }

    #[must_use]
    pub fn taxonomy_stats(&self) -> BTreeMap<String, usize> {
        let mut stats = BTreeMap::new();
        for assignment in &self.assignments {
            *stats.entry(assignment.tag_id.clone()).or_insert(0) += 1;
        }
        stats
    }

    #[must_use]
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

fn validate_tag_id(value: &str) -> Result<(), TagError> {
    if is_tag_id(value) {
        Ok(())
    } else {
        Err(TagError::InvalidTagId(value.to_string()))
    }
}

fn validate_assignment(assignment: &TagAssignment) -> Result<(), TagError> {
    validate_tag_id(&assignment.tag_id)?;
    if !is_entity_id(&assignment.entity_id)
        || !is_date(&assignment.assigned_at)
        || assignment.provenance.trim().is_empty()
        || assignment.provenance.chars().any(char::is_control)
        || assignment
            .expires_at
            .as_deref()
            .is_some_and(|expires| !is_date(expires) || expires <= assignment.assigned_at.as_str())
    {
        return Err(TagError::InvalidAssignment);
    }
    Ok(())
}

fn is_tag_id(value: &str) -> bool {
    !value.is_empty()
        && value.contains(':')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
        })
}

fn is_entity_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
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

    #[test]
    fn rejects_invalid_taxonomy_and_assignment_inputs() {
        let mut store = TagStore::default();
        assert_eq!(
            store.define(TagDefinition {
                tag_id: "asset/equity".to_string(),
                parent: None,
                ttl_days: None,
            }),
            Err(TagError::InvalidTagId("asset/equity".to_string()))
        );
        assert_eq!(
            store.define(TagDefinition {
                tag_id: "style:momentum".to_string(),
                parent: Some("asset:missing".to_string()),
                ttl_days: None,
            }),
            Err(TagError::UnknownParent("asset:missing".to_string()))
        );
        store
            .define(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        assert_eq!(
            store.assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: Some("2026-05-20".to_string()),
                provenance: "manual".to_string(),
            }),
            Err(TagError::InvalidAssignment)
        );
    }

    #[test]
    fn rejected_cycle_redefinition_restores_prior_definition() {
        let mut store = TagStore::default();
        store
            .define(TagDefinition {
                tag_id: "a:root".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("define a:root: {error}"));
        store
            .define(TagDefinition {
                tag_id: "b:child".to_string(),
                parent: Some("a:root".to_string()),
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("define b:child: {error}"));

        // Re-defining a:root to point at its own descendant forms a cycle.
        assert_eq!(
            store.define(TagDefinition {
                tag_id: "a:root".to_string(),
                parent: Some("b:child".to_string()),
                ttl_days: None,
            }),
            Err(TagError::Cycle("a:root".to_string()))
        );

        // The original a:root must survive the rejected re-definition (no dangling
        // parent for b:child) — proven by a subsequent valid assignment to a:root.
        store
            .assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "a:root".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "manual".to_string(),
            })
            .unwrap_or_else(|error| panic!("a:root should still exist after rollback: {error}"));
        assert_eq!(
            store.active_tags("instrument:AAPL", "2026-05-22"),
            vec!["a:root".to_string()]
        );
    }
}
