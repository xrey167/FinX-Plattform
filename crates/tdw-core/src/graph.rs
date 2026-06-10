//! The graph-engine contract: typed temporal nodes/edges with structured provenance.
//!
//! Mirrors the [`VectorEngine`](crate::VectorEngine) precedent — a small async trait in
//! tdw-core, an in-memory reference implementation in `tdw-storage-graph`, and real
//! backends behind feature gates, all held identical by one cross-backend conformance
//! suite (knowledge-system overhaul A2).
//!
//! # Temporal model
//!
//! Nodes and edges carry an optional half-open validity window
//! `[valid_from, valid_to)`. Timestamps are RFC 3339 strings in a normalized form
//! (UTC `Z`), compared lexicographically — the same convention tag assignments use.
//! A `None` bound is open. A read filter with `as_of: None` applies no temporal
//! filtering at all (every record matches, including closed windows).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_taxonomy::EntityKind;

use crate::{Error, Result};

/// A typed graph node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Namespaced graph id (e.g. `instrument:AAPL`), grammar `[A-Za-z0-9:._-]+`.
    pub id: String,
    /// Taxonomy classification.
    pub kind: EntityKind,
    /// Human-readable label.
    pub label: String,
    /// Alternative names/tickers.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Free-form typed properties.
    #[serde(default)]
    pub props: Value,
    /// Inclusive validity start (RFC 3339, lexicographically comparable).
    #[serde(default)]
    pub valid_from: Option<String>,
    /// Exclusive validity end.
    #[serde(default)]
    pub valid_to: Option<String>,
}

/// A typed, provenance-carrying graph edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Relationship type, grammar `[A-Za-z0-9:._-]+` (e.g. `listed_on`).
    pub rel: String,
    /// Free-form typed properties.
    #[serde(default)]
    pub props: Value,
    /// Where this edge came from — structured, not a free string.
    pub provenance: Provenance,
    /// Inclusive validity start.
    #[serde(default)]
    pub valid_from: Option<String>,
    /// Exclusive validity end.
    #[serde(default)]
    pub valid_to: Option<String>,
}

/// Structured fact provenance.
///
/// This is data lineage (where a particular node/edge came from), deliberately distinct
/// from the taxonomy's `Origin`, which classifies entity *kinds*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Provenance {
    /// Written by an ingestion path (provider, news, dataset sweep).
    Ingest {
        /// The ingest source (e.g. `provider:sec`, `tdw-knowledge:index`).
        source: String,
    },
    /// Derived by an inference/tag rule.
    Rule {
        /// The deriving rule id.
        rule_id: String,
        /// The rule-set version that fired.
        version: u64,
    },
    /// Written by an agent (through the gated proposal path).
    Agent {
        /// The writing agent id.
        agent_id: String,
        /// Whether the write went through the validation gate.
        gated: bool,
    },
    /// A human decision (e.g. a manual merge).
    Manual {
        /// Who approved it.
        approved_by: String,
    },
    /// Platform-internal bookkeeping.
    System {
        /// What wrote it.
        detail: String,
    },
}

/// Traversal direction relative to the anchor node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Follow outgoing edges only.
    #[default]
    Out,
    /// Follow incoming edges only.
    In,
    /// Follow both.
    Both,
}

/// Filter applied by every graph read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraversalFilter {
    /// Restrict to these relationship types; `None` = all.
    #[serde(default)]
    pub rels: Option<Vec<String>>,
    /// Restrict reached nodes to these kinds; `None` = all.
    #[serde(default)]
    pub kinds: Option<Vec<EntityKind>>,
    /// Temporal point of view; `None` = no temporal filtering.
    #[serde(default)]
    pub as_of: Option<String>,
    /// Traversal direction.
    #[serde(default)]
    pub direction: Direction,
    /// Hop budget for [`GraphEngine::expand`] / [`GraphEngine::shortest_path`], `1..=MAX_HOPS`.
    pub max_hops: u8,
}

/// The hop-budget ceiling — defense against accidentally unbounded traversals.
pub const MAX_HOPS: u8 = 8;

impl Default for TraversalFilter {
    fn default() -> Self {
        Self {
            rels: None,
            kinds: None,
            as_of: None,
            direction: Direction::default(),
            max_hops: 1,
        }
    }
}

/// A deterministic (id-sorted) subgraph snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Subgraph {
    /// Reached nodes, sorted by id.
    pub nodes: Vec<GraphNode>,
    /// Traversed edges, sorted by (from, rel, to, valid_from).
    pub edges: Vec<GraphEdge>,
}

/// An approved entity-merge decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeDecision {
    /// Who approved the merge (non-empty, control-char-free).
    pub approved_by: String,
}

/// What a merge actually did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeReport {
    /// The merged (tombstoned) entity.
    pub source: String,
    /// The surviving entity.
    pub target: String,
    /// Edges re-pointed from source to target.
    pub rewired_edges: usize,
    /// Aliases newly carried over to the target.
    pub absorbed_aliases: usize,
}

/// The graph storage contract.
///
/// Same shape across the in-memory reference engine and real graph databases; the
/// cross-backend conformance suite in `tdw-storage-graph` holds implementations
/// identical.
#[async_trait]
pub trait GraphEngine: Send + Sync {
    /// Insert or replace nodes by id.
    async fn upsert_nodes(&self, nodes: Vec<GraphNode>) -> Result<()>;

    /// Insert or replace edges by identity `(from, to, rel, valid_from)`.
    /// Both endpoints must already exist.
    async fn upsert_edges(&self, edges: Vec<GraphEdge>) -> Result<()>;

    /// Fetch one node by id.
    async fn node(&self, id: &str) -> Result<Option<GraphNode>>;

    /// One-hop neighborhood of `id` under `filter` (direction, rels, kinds, as_of).
    /// Deterministic order: (rel, neighbor id, valid_from).
    async fn neighbors(
        &self,
        id: &str,
        filter: &TraversalFilter,
    ) -> Result<Vec<(GraphEdge, GraphNode)>>;

    /// Breadth-first expansion from `seeds` up to `filter.max_hops`, returning the
    /// reached subgraph (seeds included when they exist and pass the filter).
    async fn expand(&self, seeds: &[String], filter: &TraversalFilter) -> Result<Subgraph>;

    /// Shortest path from `from` to `to` under `filter` (hop-bounded by
    /// `filter.max_hops`), as the ordered edge list; `None` when unreachable.
    async fn shortest_path(
        &self,
        from: &str,
        to: &str,
        filter: &TraversalFilter,
    ) -> Result<Option<Vec<GraphEdge>>>;

    /// Really merge `source` into `target`: union aliases (plus the source label),
    /// rewire every source edge to the target (provenance preserved, would-be
    /// self-loops dropped), tombstone the source (`props.merged_into = target`), and
    /// record an audit `merged_into` edge with [`Provenance::Manual`].
    async fn merge_entities(
        &self,
        source: &str,
        target: &str,
        decision: &MergeDecision,
    ) -> Result<MergeReport>;
}

/// Whether a `[valid_from, valid_to)` window is active at `as_of` (lexicographic
/// RFC 3339 comparison; open bounds always pass).
#[must_use]
pub fn active_at(valid_from: Option<&str>, valid_to: Option<&str>, as_of: &str) -> bool {
    valid_from.is_none_or(|from| from <= as_of) && valid_to.is_none_or(|to| as_of < to)
}

/// # Errors
///
/// Returns [`Error::Storage`] naming the offending field when the node is invalid.
pub fn validate_graph_node(node: &GraphNode) -> Result<()> {
    if !is_graph_id(&node.id) {
        return Err(Error::Storage(format!(
            "invalid graph node id: {}",
            node.id
        )));
    }
    if node.label.trim().is_empty() {
        return Err(Error::Storage(format!("empty label for node {}", node.id)));
    }
    if node
        .aliases
        .iter()
        .any(|alias| alias.trim().is_empty() || alias.chars().any(char::is_control))
    {
        return Err(Error::Storage(format!("invalid alias on node {}", node.id)));
    }
    validate_window(&node.valid_from, &node.valid_to)
        .map_err(|detail| Error::Storage(format!("node {}: {detail}", node.id)))
}

/// # Errors
///
/// Returns [`Error::Storage`] naming the offending field when the edge is invalid.
pub fn validate_graph_edge(edge: &GraphEdge) -> Result<()> {
    if !is_graph_id(&edge.from) || !is_graph_id(&edge.to) || !is_graph_id(&edge.rel) {
        return Err(Error::Storage(format!(
            "invalid edge endpoints/rel: {} -{}-> {}",
            edge.from, edge.rel, edge.to
        )));
    }
    if let Err(detail) = validate_provenance(&edge.provenance) {
        return Err(Error::Storage(format!(
            "edge {} -{}-> {}: {detail}",
            edge.from, edge.rel, edge.to
        )));
    }
    validate_window(&edge.valid_from, &edge.valid_to).map_err(|detail| {
        Error::Storage(format!(
            "edge {} -{}-> {}: {detail}",
            edge.from, edge.rel, edge.to
        ))
    })
}

/// # Errors
///
/// Returns [`Error::InvalidQuery`] when the filter is out of contract.
pub fn validate_traversal_filter(filter: &TraversalFilter) -> Result<()> {
    if filter.max_hops == 0 || filter.max_hops > MAX_HOPS {
        return Err(Error::InvalidQuery(format!(
            "max_hops must be 1..={MAX_HOPS}, got {}",
            filter.max_hops
        )));
    }
    if let Some(rels) = &filter.rels
        && rels.iter().any(|rel| !is_graph_id(rel))
    {
        return Err(Error::InvalidQuery("invalid rel in filter".to_string()));
    }
    if let Some(as_of) = &filter.as_of
        && !is_timestampish(as_of)
    {
        return Err(Error::InvalidQuery(format!("invalid as_of: {as_of}")));
    }
    Ok(())
}

fn validate_provenance(provenance: &Provenance) -> std::result::Result<(), String> {
    let field = match provenance {
        Provenance::Ingest { source } => ("provenance source", source),
        Provenance::Rule { rule_id, .. } => ("provenance rule_id", rule_id),
        Provenance::Agent { agent_id, .. } => ("provenance agent_id", agent_id),
        Provenance::Manual { approved_by } => ("provenance approved_by", approved_by),
        Provenance::System { detail } => ("provenance detail", detail),
    };
    if field.1.trim().is_empty() || field.1.chars().any(char::is_control) {
        return Err(format!("empty or control-char {}", field.0));
    }
    Ok(())
}

fn validate_window(
    valid_from: &Option<String>,
    valid_to: &Option<String>,
) -> std::result::Result<(), String> {
    for bound in [valid_from, valid_to].into_iter().flatten() {
        if !is_timestampish(bound) {
            return Err(format!("invalid validity bound: {bound}"));
        }
    }
    if let (Some(from), Some(to)) = (valid_from, valid_to)
        && from >= to
    {
        return Err(format!("empty validity window: [{from}, {to})"));
    }
    Ok(())
}

/// The graph id grammar shared by node ids and relationship types.
#[must_use]
pub fn is_graph_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

fn is_timestampish(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().all(|character| !character.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
            props: json!({}),
            valid_from: None,
            valid_to: None,
        }
    }

    #[test]
    fn validates_nodes_edges_and_filters() {
        assert!(validate_graph_node(&node("instrument:AAPL")).is_ok());
        assert!(validate_graph_node(&node("../escape")).is_err());
        assert!(
            validate_graph_node(&GraphNode {
                label: "  ".to_string(),
                ..node("instrument:AAPL")
            })
            .is_err()
        );

        let edge = GraphEdge {
            from: "instrument:AAPL".to_string(),
            to: "venue:XNAS".to_string(),
            rel: "listed_on".to_string(),
            props: json!({}),
            provenance: Provenance::Ingest {
                source: "fixture".to_string(),
            },
            valid_from: Some("2020-01-01T00:00:00Z".to_string()),
            valid_to: Some("2030-01-01T00:00:00Z".to_string()),
        };
        assert!(validate_graph_edge(&edge).is_ok());
        assert!(
            validate_graph_edge(&GraphEdge {
                provenance: Provenance::Manual {
                    approved_by: " ".to_string(),
                },
                ..edge.clone()
            })
            .is_err()
        );
        assert!(
            validate_graph_edge(&GraphEdge {
                valid_to: Some("2010-01-01T00:00:00Z".to_string()),
                ..edge
            })
            .is_err()
        );

        assert!(validate_traversal_filter(&TraversalFilter::default()).is_ok());
        assert!(
            validate_traversal_filter(&TraversalFilter {
                max_hops: 0,
                ..TraversalFilter::default()
            })
            .is_err()
        );
        assert!(
            validate_traversal_filter(&TraversalFilter {
                max_hops: MAX_HOPS + 1,
                ..TraversalFilter::default()
            })
            .is_err()
        );
    }

    #[test]
    fn active_at_is_half_open_with_open_bounds() {
        let from = Some("2024-01-01T00:00:00Z");
        let to = Some("2025-01-01T00:00:00Z");
        assert!(active_at(from, to, "2024-01-01T00:00:00Z"));
        assert!(active_at(from, to, "2024-06-15T12:00:00Z"));
        assert!(!active_at(from, to, "2025-01-01T00:00:00Z"));
        assert!(!active_at(from, to, "2023-12-31T23:59:59Z"));
        assert!(active_at(None, None, "1970-01-01T00:00:00Z"));
        assert!(active_at(None, to, "2024-12-31T00:00:00Z"));
        assert!(active_at(from, None, "2099-01-01T00:00:00Z"));
    }

    #[test]
    fn provenance_serializes_tagged_snake_case() {
        let encoded = serde_json::to_value(Provenance::Rule {
            rule_id: "tag-propagate-1".to_string(),
            version: 3,
        })
        .expect("provenance should serialize");
        assert_eq!(encoded["kind"], "rule");
        assert_eq!(encoded["rule_id"], "tag-propagate-1");
        let decoded: Provenance =
            serde_json::from_value(encoded).expect("provenance should deserialize");
        assert_eq!(
            decoded,
            Provenance::Rule {
                rule_id: "tag-propagate-1".to_string(),
                version: 3,
            }
        );
    }
}
