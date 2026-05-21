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

    pub fn add_relationship(&mut self, relationship: Relationship) {
        self.edges.push(relationship);
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
        self.merge_audit
            .push(format!("{source}->{target} approved_by={approved_by}"));
        true
    }

    pub fn merge_audit(&self) -> &[String] {
        &self.merge_audit
    }
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
}
