#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    Instrument,
    Account,
    Strategy,
    Agent,
    Dataset,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub entity_id: String,
    pub kind: EntityKind,
    pub label: String,
    pub aliases: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub from: String,
    pub to: String,
    pub rel_type: String,
    pub provenance: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnowledgeGraphError {
    InvalidEntityId,
    EmptyLabel,
    InvalidAlias,
    InvalidRelationship,
    MissingEndpoint,
    EmptyProvenance,
}

#[derive(Clone, Debug, Default)]
pub struct KnowledgeGraph {
    entities: BTreeMap<String, Entity>,
    edges: Vec<Relationship>,
    merge_audit: Vec<String>,
}

impl KnowledgeGraph {
    pub fn upsert_entity(&mut self, entity: Entity) {
        self.entities.insert(entity.entity_id.clone(), entity);
    }

    pub fn try_upsert_entity(&mut self, entity: Entity) -> Result<(), KnowledgeGraphError> {
        validate_entity(&entity)?;
        self.upsert_entity(entity);
        Ok(())
    }

    pub fn add_relationship(&mut self, relationship: Relationship) {
        self.edges.push(relationship);
    }

    pub fn try_add_relationship(
        &mut self,
        relationship: Relationship,
    ) -> Result<(), KnowledgeGraphError> {
        validate_relationship(&relationship)?;
        if !self.entities.contains_key(&relationship.from)
            || !self.entities.contains_key(&relationship.to)
        {
            return Err(KnowledgeGraphError::MissingEndpoint);
        }
        self.add_relationship(relationship);
        Ok(())
    }

    pub fn entity(&self, entity_id: &str) -> Option<&Entity> {
        self.entities.get(entity_id)
    }

    pub fn neighbors(&self, entity_id: &str) -> Vec<&Entity> {
        let ids = self
            .edges
            .iter()
            .filter_map(|edge| (edge.from == entity_id).then_some(edge.to.clone()))
            .collect::<BTreeSet<_>>();
        ids.iter()
            .filter_map(|neighbor_id| self.entities.get(neighbor_id))
            .collect()
    }

    pub fn manual_merge(&mut self, source: &str, target: &str, approved_by: &str) -> bool {
        if !self.entities.contains_key(source) || !self.entities.contains_key(target) {
            return false;
        }
        if approved_by.trim().is_empty() || approved_by.chars().any(char::is_control) {
            return false;
        }
        self.merge_audit
            .push(format!("{source}->{target} approved_by={approved_by}"));
        true
    }

    pub fn merge_audit(&self) -> &[String] {
        &self.merge_audit
    }
}

pub fn validate_entity(entity: &Entity) -> Result<(), KnowledgeGraphError> {
    if !is_graph_id(&entity.entity_id) {
        return Err(KnowledgeGraphError::InvalidEntityId);
    }
    if entity.label.trim().is_empty() {
        return Err(KnowledgeGraphError::EmptyLabel);
    }
    if entity
        .aliases
        .iter()
        .any(|alias| alias.trim().is_empty() || alias.chars().any(char::is_control))
    {
        return Err(KnowledgeGraphError::InvalidAlias);
    }
    Ok(())
}

pub fn validate_relationship(relationship: &Relationship) -> Result<(), KnowledgeGraphError> {
    if !is_graph_id(&relationship.from)
        || !is_graph_id(&relationship.to)
        || !is_graph_id(&relationship.rel_type)
    {
        return Err(KnowledgeGraphError::InvalidRelationship);
    }
    if relationship.provenance.trim().is_empty()
        || relationship.provenance.chars().any(char::is_control)
    {
        return Err(KnowledgeGraphError::EmptyProvenance);
    }
    Ok(())
}

fn is_graph_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_entities_edges_and_manual_merge_audit() {
        let mut kg = KnowledgeGraph::default();
        kg.upsert_entity(Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        });
        kg.upsert_entity(Entity {
            entity_id: "dataset:ohlcv".to_string(),
            kind: EntityKind::Dataset,
            label: "OHLCV".to_string(),
            aliases: Vec::new(),
        });
        kg.add_relationship(Relationship {
            from: "instrument:AAPL".to_string(),
            to: "dataset:ohlcv".to_string(),
            rel_type: "has_prices".to_string(),
            provenance: "fixture".to_string(),
        });

        assert_eq!(
            kg.neighbors("instrument:AAPL")[0].entity_id,
            "dataset:ohlcv"
        );
        assert!(kg.manual_merge("instrument:AAPL", "dataset:ohlcv", "architect"));
        assert_eq!(kg.merge_audit().len(), 1);
    }

    #[test]
    fn checked_paths_reject_invalid_entities_and_missing_edges() {
        let mut kg = KnowledgeGraph::default();
        assert_eq!(
            kg.try_upsert_entity(Entity {
                entity_id: "../instrument".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            }),
            Err(KnowledgeGraphError::InvalidEntityId)
        );

        kg.try_upsert_entity(Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        })
        .unwrap_or_else(|error| panic!("entity should insert: {error:?}"));
        assert_eq!(
            kg.try_add_relationship(Relationship {
                from: "instrument:AAPL".to_string(),
                to: "dataset:missing".to_string(),
                rel_type: "has_prices".to_string(),
                provenance: "fixture".to_string(),
            }),
            Err(KnowledgeGraphError::MissingEndpoint)
        );
        assert!(!kg.manual_merge("instrument:AAPL", "instrument:AAPL", ""));
    }
}
