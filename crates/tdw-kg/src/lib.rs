#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Knowledge-graph primitives over the unified taxonomy (knowledge-system A3).
//!
//! A3 rebuilds this crate's internals while keeping its method surface:
//!
//! * [`EntityKind`] is now the platform-wide 50-kind taxonomy re-exported from
//!   `tdw-taxonomy` — the 5 local kinds this crate used to define (Instrument,
//!   Account, Strategy, Agent, Dataset) are a subset of it, so existing
//!   constructors compile unchanged. SERDE NOTE: kinds now serialize to the
//!   registry's lowercase tokens (`"instrument"`), not the old `PascalCase`
//!   (`"Instrument"`). Nothing durable stored the old tokens (the Postgres KG
//!   tables were never wired); in-repo fixtures are updated in this change.
//! * [`KnowledgeGraph::manual_merge`] is now a REAL merge with the same
//!   semantics as `GraphEngine::merge_entities` (A2): alias union, edge
//!   rewiring with duplicate/self-loop dropping, a tombstone marker, and an
//!   audit entry — not just an audit string. Repeat merges are rejected.
//! * [`Entity`]/[`Relationship`] convert to the `tdw-core` graph types
//!   ([`Entity::to_graph_node`], [`Relationship::to_graph_edge`]), the bridge
//!   the durable engine swap (A5) and the hybrid retriever (B4) build on.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::{GraphEdge, GraphNode, Provenance};

pub use tdw_taxonomy::EntityKind;

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

impl Entity {
    /// Project onto the durable graph contract's node type (A2). Properties and
    /// validity windows have no source here and default to empty/open.
    #[must_use]
    pub fn to_graph_node(&self) -> GraphNode {
        GraphNode {
            id: self.entity_id.clone(),
            kind: self.kind,
            label: self.label.clone(),
            aliases: self.aliases.clone(),
            props: Value::Null,
            valid_from: None,
            valid_to: None,
        }
    }

    /// Recover an entity from a graph node (drops props/validity, which this
    /// in-process model does not carry).
    #[must_use]
    pub fn from_graph_node(node: &GraphNode) -> Self {
        Self {
            entity_id: node.id.clone(),
            kind: node.kind,
            label: node.label.clone(),
            aliases: node.aliases.clone(),
        }
    }
}

impl Relationship {
    /// Project onto the durable graph contract's edge type (A2). The free-string
    /// provenance this model carries wraps into [`Provenance::System`]; richer
    /// structured provenance is written natively by engine-first callers.
    #[must_use]
    pub fn to_graph_edge(&self) -> GraphEdge {
        GraphEdge {
            from: self.from.clone(),
            to: self.to.clone(),
            rel: self.rel_type.clone(),
            props: Value::Null,
            provenance: Provenance::System {
                detail: self.provenance.clone(),
            },
            valid_from: None,
            valid_to: None,
        }
    }
}

/// What a [`KnowledgeGraph::manual_merge`] actually did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeOutcome {
    /// Edges re-pointed from source to target.
    pub rewired_edges: usize,
    /// Aliases newly carried over to the target.
    pub absorbed_aliases: usize,
}

#[derive(Clone, Debug, Default)]
pub struct KnowledgeGraph {
    entities: BTreeMap<String, Entity>,
    edges: Vec<Relationship>,
    /// `source -> target` for every merged (tombstoned) entity.
    merged: BTreeMap<String, String>,
    merge_audit: Vec<String>,
}

impl KnowledgeGraph {
    pub fn upsert_entity(&mut self, entity: Entity) {
        self.entities.insert(entity.entity_id.clone(), entity);
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_upsert_entity(&mut self, entity: Entity) -> Result<(), KnowledgeGraphError> {
        validate_entity(&entity)?;
        self.upsert_entity(entity);
        Ok(())
    }

    pub fn add_relationship(&mut self, relationship: Relationship) {
        self.edges.push(relationship);
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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

    #[must_use]
    pub fn entity(&self, entity_id: &str) -> Option<&Entity> {
        self.entities.get(entity_id)
    }

    /// The merge target a tombstoned entity was absorbed into, if any.
    #[must_use]
    pub fn merged_into(&self, entity_id: &str) -> Option<&str> {
        self.merged.get(entity_id).map(String::as_str)
    }

    /// Every NON-tombstoned entity, in id order. This is the set entity
    /// resolution must run over — a tombstoned source still answers
    /// [`KnowledgeGraph::entity`] for audit purposes but must never win
    /// resolution again (its aliases were absorbed by the merge target).
    pub fn live_entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities
            .values()
            .filter(|entity| !self.merged.contains_key(&entity.entity_id))
    }

    #[must_use]
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

    /// Really merge `source` into `target` (A3 — previously this only recorded
    /// an audit string): union the source's aliases and label onto the target,
    /// re-point every source edge at the target (duplicates and would-be
    /// self-loops drop), tombstone the source (it stays addressable;
    /// [`KnowledgeGraph::merged_into`] reports its target), and append an audit
    /// entry. Same semantics as `GraphEngine::merge_entities`, so the durable
    /// engine swap (A5) is behavior-preserving.
    ///
    /// Returns `false` — and changes nothing — for a self-merge, a missing
    /// endpoint, an already-merged source, or an invalid approver.
    pub fn manual_merge(&mut self, source: &str, target: &str, approved_by: &str) -> bool {
        self.try_manual_merge(source, target, approved_by).is_some()
    }

    /// [`KnowledgeGraph::manual_merge`] with the merge report.
    pub fn try_manual_merge(
        &mut self,
        source: &str,
        target: &str,
        approved_by: &str,
    ) -> Option<MergeOutcome> {
        if source == target
            || self.merged.contains_key(source)
            || !self.entities.contains_key(source)
            || !self.entities.contains_key(target)
        {
            return None;
        }
        if approved_by.trim().is_empty() || approved_by.chars().any(char::is_control) {
            return None;
        }

        // Alias union: the source's aliases and label survive on the target.
        let source_entity = self.entities.get(source)?.clone();
        let target_entity = self.entities.get_mut(target)?;
        let mut absorbed_aliases = 0;
        for alias in source_entity
            .aliases
            .iter()
            .chain(std::iter::once(&source_entity.label))
        {
            if alias != &target_entity.label && !target_entity.aliases.contains(alias) {
                target_entity.aliases.push(alias.clone());
                absorbed_aliases += 1;
            }
        }
        target_entity.aliases.sort();

        // Edge rewiring with duplicate/self-loop dropping (the A2 semantics).
        let identity =
            |edge: &Relationship| (edge.from.clone(), edge.to.clone(), edge.rel_type.clone());
        let surviving: BTreeSet<_> = self
            .edges
            .iter()
            .filter(|edge| edge.from != source && edge.to != source)
            .map(identity)
            .collect();
        let mut seen = BTreeSet::new();
        let mut rewired_edges = 0;
        let mut kept = Vec::with_capacity(self.edges.len());
        for mut edge in std::mem::take(&mut self.edges) {
            let touches_source = edge.from == source || edge.to == source;
            if touches_source {
                if edge.from == source {
                    edge.from = target.to_string();
                }
                if edge.to == source {
                    edge.to = target.to_string();
                }
                if edge.from == edge.to {
                    continue;
                }
                let key = identity(&edge);
                if surviving.contains(&key) || !seen.insert(key) {
                    continue;
                }
                rewired_edges += 1;
            }
            kept.push(edge);
        }
        self.edges = kept;

        // Tombstone + audit. The audit is a REAL traversable edge (A2 parity:
        // `neighbors(source)` surfaces the merge target), plus the human-readable
        // log line.
        self.merged.insert(source.to_string(), target.to_string());
        self.edges.push(Relationship {
            from: source.to_string(),
            to: target.to_string(),
            rel_type: "merged_into".to_string(),
            provenance: format!("manual:{approved_by}"),
        });
        self.merge_audit
            .push(format!("{source}->{target} approved_by={approved_by}"));
        Some(MergeOutcome {
            rewired_edges,
            absorbed_aliases,
        })
    }

    #[must_use]
    pub fn merge_audit(&self) -> &[String] {
        &self.merge_audit
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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
    tdw_core::is_graph_id(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(id: &str, label: &str) -> Entity {
        Entity {
            entity_id: id.to_string(),
            kind: EntityKind::Instrument,
            label: label.to_string(),
            aliases: Vec::new(),
        }
    }

    #[test]
    fn queries_entities_edges_and_merge_audit() {
        let mut kg = KnowledgeGraph::default();
        kg.upsert_entity(Entity {
            aliases: vec!["AAPL".to_string()],
            ..entity("instrument:AAPL", "Apple")
        });
        kg.upsert_entity(Entity {
            kind: EntityKind::Dataset,
            ..entity("dataset:ohlcv", "OHLCV")
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
            kg.try_upsert_entity(entity("../instrument", "Apple")),
            Err(KnowledgeGraphError::InvalidEntityId)
        );

        kg.try_upsert_entity(entity("instrument:AAPL", "Apple"))
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

    #[test]
    fn validate_entity_and_relationship_reject_each_bad_field() {
        assert_eq!(
            validate_entity(&Entity {
                label: "  ".to_string(),
                ..entity("instrument:AAPL", "x")
            }),
            Err(KnowledgeGraphError::EmptyLabel)
        );
        assert_eq!(
            validate_entity(&Entity {
                aliases: vec![" ".to_string()],
                ..entity("instrument:AAPL", "Apple")
            }),
            Err(KnowledgeGraphError::InvalidAlias)
        );
        assert_eq!(
            validate_relationship(&Relationship {
                from: "a:1".to_string(),
                to: "b:2".to_string(),
                rel_type: "bad type".to_string(),
                provenance: "fixture".to_string(),
            }),
            Err(KnowledgeGraphError::InvalidRelationship)
        );
        assert_eq!(
            validate_relationship(&Relationship {
                from: "a:1".to_string(),
                to: "b:2".to_string(),
                rel_type: "rel".to_string(),
                provenance: " ".to_string(),
            }),
            Err(KnowledgeGraphError::EmptyProvenance)
        );
    }

    #[test]
    fn manual_merge_really_merges_with_a2_semantics() {
        let mut kg = KnowledgeGraph::default();
        kg.upsert_entity(Entity {
            aliases: vec!["AAPL".to_string()],
            ..entity("instrument:AAPL", "Apple Inc.")
        });
        kg.upsert_entity(Entity {
            aliases: vec!["APPLE".to_string()],
            ..entity("instrument:APPLE-DUP", "Apple (duplicate)")
        });
        kg.upsert_entity(Entity {
            kind: EntityKind::Account,
            ..entity("account:alpha", "Alpha Book")
        });
        for to in ["instrument:AAPL", "instrument:APPLE-DUP"] {
            kg.add_relationship(Relationship {
                from: "account:alpha".to_string(),
                to: to.to_string(),
                rel_type: "holds".to_string(),
                provenance: "fixture".to_string(),
            });
        }

        // Self-merge is rejected without state changes.
        assert!(!kg.manual_merge("instrument:APPLE-DUP", "instrument:APPLE-DUP", "architect"));
        assert!(kg.merge_audit().is_empty());

        let outcome = kg
            .try_manual_merge("instrument:APPLE-DUP", "instrument:AAPL", "architect")
            .unwrap_or_else(|| panic!("merge should succeed"));
        // The duplicate holds-edge collapses onto the surviving one.
        assert_eq!(outcome.rewired_edges, 0);
        assert_eq!(outcome.absorbed_aliases, 2, "alias + label absorbed");

        let target = kg
            .entity("instrument:AAPL")
            .unwrap_or_else(|| panic!("target must exist"));
        assert!(target.aliases.contains(&"APPLE".to_string()));
        assert!(target.aliases.contains(&"Apple (duplicate)".to_string()));
        // Tombstone: still addressable, marked merged; repeat merge rejected.
        assert!(kg.entity("instrument:APPLE-DUP").is_some());
        assert_eq!(
            kg.merged_into("instrument:APPLE-DUP"),
            Some("instrument:AAPL")
        );
        assert!(!kg.manual_merge("instrument:APPLE-DUP", "instrument:AAPL", "architect"));
        assert_eq!(kg.merge_audit().len(), 1, "no duplicate audit entry");
        // The audit is a TRAVERSABLE edge (A2 parity): the tombstoned source's
        // only neighbor is its merge target.
        assert_eq!(
            kg.neighbors("instrument:APPLE-DUP")
                .iter()
                .map(|entity| entity.entity_id.as_str())
                .collect::<Vec<_>>(),
            vec!["instrument:AAPL"]
        );
        // Resolution must run over live entities only: the tombstoned source
        // is excluded, so its stale aliases can never win resolution again.
        assert_eq!(
            kg.live_entities()
                .map(|entity| entity.entity_id.as_str())
                .collect::<Vec<_>>(),
            vec!["account:alpha", "instrument:AAPL"]
        );
        // The account's holds-neighborhood is exactly the target, once.
        assert_eq!(
            kg.neighbors("account:alpha")
                .iter()
                .map(|entity| entity.entity_id.as_str())
                .collect::<Vec<_>>(),
            vec!["instrument:AAPL"]
        );
    }

    #[test]
    fn converts_to_graph_contract_types() {
        let apple = Entity {
            aliases: vec!["AAPL".to_string()],
            ..entity("instrument:AAPL", "Apple")
        };
        let node = apple.to_graph_node();
        assert_eq!(node.id, "instrument:AAPL");
        assert_eq!(node.kind, EntityKind::Instrument);
        assert_eq!(Entity::from_graph_node(&node), apple);

        let edge = Relationship {
            from: "instrument:AAPL".to_string(),
            to: "dataset:ohlcv".to_string(),
            rel_type: "has_prices".to_string(),
            provenance: "tdw-knowledge".to_string(),
        }
        .to_graph_edge();
        assert_eq!(edge.rel, "has_prices");
        assert_eq!(
            edge.provenance,
            Provenance::System {
                detail: "tdw-knowledge".to_string(),
            }
        );
    }

    #[test]
    fn entity_kind_is_the_unified_taxonomy() {
        // The unification itself: tdw-kg's kind IS the 50-kind taxonomy; the
        // old local 5 variants are members of it, and warehouse kinds beyond
        // the old enum are usable directly.
        let venue = Entity {
            kind: EntityKind::Venue,
            ..entity("venue:XNAS", "Nasdaq")
        };
        assert_eq!(
            serde_json::to_value(venue.kind).expect("kind should serialize"),
            serde_json::Value::String("venue".to_string()),
            "kinds serialize to the registry's lowercase tokens"
        );
        assert_eq!(EntityKind::ALL.len(), 51);
    }
}
