//! Tags as first-class graph citizens (knowledge-system A5): the
//! [`TagEngine`] contract implemented over any [`GraphEngine`].
//!
//! # Graph model
//!
//! * A tag definition is a node `{ id: <tag_id>, kind: Tag }` with `ttl_days`
//!   in props. Tag ids share the graph id namespace (the `:`-bearing tag
//!   grammar is a subset of the graph id grammar).
//! * Hierarchy is a `subtag_of` edge from child tag to parent tag;
//!   re-parenting swaps the edge atomically via `GraphEngine::replace_edges`.
//!   Cycles are rejected BEFORE any write by walking the prospective parent
//!   chain — no rollback needed, unlike the in-process store.
//! * An assignment is a `tagged` edge from the entity node to the tag node,
//!   with the `[assigned_at, expires_at)` date window mapped losslessly onto
//!   the graph's `[valid_from, valid_to)` timestamps via
//!   [`date_to_timestamp`]. Multiple windows per (entity, tag) coexist (edge
//!   identity includes `valid_from`).
//! * The in-process store imposes no entity-existence precondition, so this
//!   backend auto-creates a STUB node (`kind: Primitive`,
//!   `props.tag_stub: true`) for an unknown entity; a later real
//!   `upsert_nodes` overwrites it by id.

use async_trait::async_trait;
use serde_json::json;
use tdw_core::{Direction, GraphEdge, GraphEngine, GraphNode, Provenance, TraversalFilter};
use tdw_tags::{TagAssignment, TagDefinition, TagEngine, TagError, date_to_timestamp};
use tdw_taxonomy::EntityKind;

/// Node-count guard for taxonomy walks — not a depth cap: a (corrupted)
/// store errs loudly instead of silently truncating.
const MAX_TAXONOMY_NODES: usize = 100_000;

/// [`TagEngine`] over any [`GraphEngine`] backend.
pub struct GraphTagEngine<G> {
    graph: G,
}

impl<G: GraphEngine> GraphTagEngine<G> {
    /// Wrap a graph engine (e.g. the in-memory reference or the Bolt backend).
    pub const fn new(graph: G) -> Self {
        Self { graph }
    }

    fn storage(error: impl std::fmt::Display) -> TagError {
        TagError::Engine(error.to_string())
    }

    async fn tag_node(&self, tag_id: &str) -> Result<Option<GraphNode>, TagError> {
        let node = self.graph.node(tag_id).await.map_err(Self::storage)?;
        Ok(node.filter(|node| node.kind == EntityKind::Tag))
    }

    /// Walk the parent chain from `start`; `true` if it reaches `needle`.
    async fn parent_chain_reaches(&self, start: &str, needle: &str) -> Result<bool, TagError> {
        let filter = TraversalFilter {
            rels: Some(vec!["subtag_of".to_string()]),
            direction: Direction::Out,
            max_hops: 1,
            ..TraversalFilter::default()
        };
        let mut current = start.to_string();
        // The taxonomy is acyclic by construction (this guard), so the walk
        // terminates; the hop cap is defense-in-depth against a corrupted
        // store.
        for _ in 0..64 {
            if current == needle {
                return Ok(true);
            }
            let parents = self
                .graph
                .neighbors(&current, &filter)
                .await
                .map_err(Self::storage)?;
            match parents.first() {
                Some((_, parent)) => current.clone_from(&parent.id),
                None => return Ok(false),
            }
        }
        Err(TagError::Engine(
            "taxonomy parent chain exceeded 64 hops (corrupted store?)".to_string(),
        ))
    }

    async fn ensure_entity_node(&self, entity_id: &str) -> Result<(), TagError> {
        if self
            .graph
            .node(entity_id)
            .await
            .map_err(Self::storage)?
            .is_none()
        {
            self.graph
                .upsert_nodes(vec![GraphNode {
                    id: entity_id.to_string(),
                    kind: EntityKind::Primitive,
                    label: entity_id.to_string(),
                    aliases: Vec::new(),
                    props: json!({ "tag_stub": true }),
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .map_err(Self::storage)?;
        }
        Ok(())
    }
}

#[async_trait]
impl<G: GraphEngine> TagEngine for GraphTagEngine<G> {
    async fn define(&self, definition: TagDefinition) -> Result<(), TagError> {
        tdw_tags::validate_definition_shape(&definition)?;
        if let Some(parent) = &definition.parent {
            if self.tag_node(parent).await?.is_none() {
                return Err(TagError::UnknownParent(parent.clone()));
            }
            // Reject the cycle BEFORE writing anything: would the new parent's
            // chain lead back to the tag being defined?
            if self
                .parent_chain_reaches(parent, &definition.tag_id)
                .await?
            {
                return Err(TagError::Cycle(definition.tag_id));
            }
        }
        self.graph
            .upsert_nodes(vec![GraphNode {
                id: definition.tag_id.clone(),
                kind: EntityKind::Tag,
                label: definition.tag_id.clone(),
                aliases: Vec::new(),
                props: json!({ "ttl_days": definition.ttl_days }),
                valid_from: None,
                valid_to: None,
            }])
            .await
            .map_err(Self::storage)?;
        // Re-parenting replaces the hierarchy edge ATOMICALLY: retract + add
        // happen in one engine operation, so no failure window can silently
        // promote the tag to a root.
        let new_parent_edges = definition
            .parent
            .as_ref()
            .map(|parent| {
                vec![GraphEdge {
                    from: definition.tag_id.clone(),
                    to: parent.clone(),
                    rel: "subtag_of".to_string(),
                    props: serde_json::Value::Null,
                    provenance: Provenance::System {
                        detail: "tdw-tags".to_string(),
                    },
                    valid_from: None,
                    valid_to: None,
                }]
            })
            .unwrap_or_default();
        self.graph
            .replace_edges(&definition.tag_id, "subtag_of", new_parent_edges)
            .await
            .map_err(Self::storage)
    }

    async fn assign(&self, assignment: TagAssignment) -> Result<(), TagError> {
        tdw_tags::validate_assignment_shape(&assignment)?;
        if self.tag_node(&assignment.tag_id).await?.is_none() {
            return Err(TagError::UnknownTag(assignment.tag_id));
        }
        self.ensure_entity_node(&assignment.entity_id).await?;
        self.graph
            .upsert_edges(vec![GraphEdge {
                from: assignment.entity_id.clone(),
                to: assignment.tag_id.clone(),
                rel: "tagged".to_string(),
                props: json!({ "provenance": assignment.provenance }),
                provenance: Provenance::System {
                    detail: assignment.provenance.clone(),
                },
                valid_from: Some(date_to_timestamp(&assignment.assigned_at)),
                valid_to: assignment.expires_at.as_deref().map(date_to_timestamp),
            }])
            .await
            .map_err(Self::storage)
    }

    async fn active_tags(&self, entity_id: &str, as_of: &str) -> Result<Vec<String>, TagError> {
        let filter = TraversalFilter {
            rels: Some(vec!["tagged".to_string()]),
            kinds: Some(vec![EntityKind::Tag]),
            as_of: Some(date_to_timestamp(as_of)),
            direction: Direction::Out,
            max_hops: 1,
        };
        let mut tags: Vec<String> = self
            .graph
            .neighbors(entity_id, &filter)
            .await
            .map_err(Self::storage)?
            .into_iter()
            .map(|(_, node)| node.id)
            .collect();
        tags.sort();
        tags.dedup();
        Ok(tags)
    }

    async fn descendants(&self, tag_id: &str) -> Result<Vec<String>, TagError> {
        // Children point AT their parent, so descendants are the transitive
        // INCOMING subtag_of closure. Iterative per-level hops rather than
        // expand(): the trait promises ALL transitive descendants, and the
        // hop-capped expand would silently truncate taxonomies deeper than
        // MAX_HOPS while the in-process store does not.
        let filter = TraversalFilter {
            rels: Some(vec!["subtag_of".to_string()]),
            kinds: Some(vec![EntityKind::Tag]),
            direction: Direction::In,
            max_hops: 1,
            ..TraversalFilter::default()
        };
        let mut found = std::collections::BTreeSet::new();
        let mut frontier = vec![tag_id.to_string()];
        while let Some(current) = frontier.pop() {
            for (_, child) in self
                .graph
                .neighbors(&current, &filter)
                .await
                .map_err(Self::storage)?
            {
                if found.insert(child.id.clone()) {
                    frontier.push(child.id);
                }
                if found.len() > MAX_TAXONOMY_NODES {
                    return Err(TagError::Engine(
                        "taxonomy exceeded 100000 nodes (corrupted store?)".to_string(),
                    ));
                }
            }
        }
        found.remove(tag_id);
        Ok(found.into_iter().collect())
    }

    async fn entities_with_tag(&self, tag_id: &str, as_of: &str) -> Result<Vec<String>, TagError> {
        let filter = TraversalFilter {
            rels: Some(vec!["tagged".to_string()]),
            as_of: Some(date_to_timestamp(as_of)),
            direction: Direction::In,
            max_hops: 1,
            ..TraversalFilter::default()
        };
        let mut entities: Vec<String> = self
            .graph
            .neighbors(tag_id, &filter)
            .await
            .map_err(Self::storage)?
            .into_iter()
            .map(|(_, node)| node.id)
            .collect();
        entities.sort();
        entities.dedup();
        Ok(entities)
    }
}
