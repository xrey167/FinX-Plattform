//! The in-memory reference [`GraphEngine`] (knowledge-system overhaul A2).
//!
//! Deterministic, no I/O — the test/conformance/embedded backend, exactly as
//! `InMemoryVectorEngine` is for the vector contract. Real graph databases (the Bolt
//! backend, slice A4) implement the same trait and must pass the same conformance
//! suite in `tests/conformance.rs`.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tdw_core::{
    Direction, Error, GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance,
    Result, Subgraph, TraversalFilter, active_at, validate_graph_edge, validate_graph_node,
    validate_traversal_filter,
};

#[derive(Debug, Default)]
struct GraphState {
    nodes: BTreeMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
}

/// In-memory, mutex-guarded reference implementation of [`GraphEngine`].
#[derive(Debug, Default)]
pub struct InMemoryGraphEngine {
    state: Mutex<GraphState>,
}

/// Edge identity for upsert-replacement: `(from, to, rel, valid_from)`.
fn edge_identity(edge: &GraphEdge) -> (String, String, String, Option<String>) {
    (
        edge.from.clone(),
        edge.to.clone(),
        edge.rel.clone(),
        edge.valid_from.clone(),
    )
}

/// Deterministic edge order: `(from, rel, to, valid_from)`.
fn edge_sort_key(edge: &GraphEdge) -> (String, String, String, Option<String>) {
    (
        edge.from.clone(),
        edge.rel.clone(),
        edge.to.clone(),
        edge.valid_from.clone(),
    )
}

fn record_active(valid_from: Option<&str>, valid_to: Option<&str>, as_of: Option<&str>) -> bool {
    as_of.is_none_or(|at| active_at(valid_from, valid_to, at))
}

fn edge_passes(edge: &GraphEdge, filter: &TraversalFilter) -> bool {
    filter
        .rels
        .as_ref()
        .is_none_or(|rels| rels.contains(&edge.rel))
        && record_active(
            edge.valid_from.as_deref(),
            edge.valid_to.as_deref(),
            filter.as_of.as_deref(),
        )
}

fn node_passes(node: &GraphNode, filter: &TraversalFilter) -> bool {
    filter
        .kinds
        .as_ref()
        .is_none_or(|kinds| kinds.contains(&node.kind))
        && record_active(
            node.valid_from.as_deref(),
            node.valid_to.as_deref(),
            filter.as_of.as_deref(),
        )
}

/// The neighbor an edge reaches from `anchor` under `direction`, if the edge leaves the
/// anchor in an allowed direction.
fn reached_id<'e>(edge: &'e GraphEdge, anchor: &str, direction: Direction) -> Option<&'e str> {
    let outgoing = edge.from == anchor;
    let incoming = edge.to == anchor;
    match direction {
        Direction::Out if outgoing => Some(&edge.to),
        Direction::In if incoming => Some(&edge.from),
        Direction::Both if outgoing => Some(&edge.to),
        Direction::Both if incoming => Some(&edge.from),
        _ => None,
    }
}

impl InMemoryGraphEngine {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, GraphState>> {
        self.state
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))
    }

    /// One filtered hop from `anchor` within a held state, deterministic order.
    fn hop(
        state: &GraphState,
        anchor: &str,
        filter: &TraversalFilter,
    ) -> Vec<(GraphEdge, GraphNode)> {
        let mut reached = state
            .edges
            .iter()
            .filter(|edge| edge_passes(edge, filter))
            .filter_map(|edge| {
                reached_id(edge, anchor, filter.direction).and_then(|neighbor_id| {
                    state
                        .nodes
                        .get(neighbor_id)
                        .filter(|node| node_passes(node, filter))
                        .map(|node| (edge.clone(), node.clone()))
                })
            })
            .collect::<Vec<_>>();
        reached.sort_by_key(|(edge, node)| {
            (edge.rel.clone(), node.id.clone(), edge.valid_from.clone())
        });
        reached
    }
}

#[async_trait]
impl GraphEngine for InMemoryGraphEngine {
    async fn upsert_nodes(&self, nodes: Vec<GraphNode>) -> Result<()> {
        if nodes.is_empty() {
            return Err(Error::Storage(
                "node upsert must include at least one node".to_string(),
            ));
        }
        for node in &nodes {
            validate_graph_node(node)?;
        }
        let mut state = self.lock()?;
        for node in nodes {
            state.nodes.insert(node.id.clone(), node);
        }
        Ok(())
    }

    async fn upsert_edges(&self, edges: Vec<GraphEdge>) -> Result<()> {
        if edges.is_empty() {
            return Err(Error::Storage(
                "edge upsert must include at least one edge".to_string(),
            ));
        }
        for edge in &edges {
            validate_graph_edge(edge)?;
        }
        let mut state = self.lock()?;
        for edge in &edges {
            if !state.nodes.contains_key(&edge.from) || !state.nodes.contains_key(&edge.to) {
                return Err(Error::Storage(format!(
                    "edge endpoint missing: {} -{}-> {}",
                    edge.from, edge.rel, edge.to
                )));
            }
        }
        for edge in edges {
            let identity = edge_identity(&edge);
            if let Some(existing) = state
                .edges
                .iter_mut()
                .find(|candidate| edge_identity(candidate) == identity)
            {
                *existing = edge;
            } else {
                state.edges.push(edge);
            }
        }
        Ok(())
    }

    async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
        Ok(self.lock()?.nodes.get(id).cloned())
    }

    async fn neighbors(
        &self,
        id: &str,
        filter: &TraversalFilter,
    ) -> Result<Vec<(GraphEdge, GraphNode)>> {
        validate_traversal_filter(filter)?;
        let state = self.lock()?;
        Ok(Self::hop(&state, id, filter))
    }

    async fn expand(&self, seeds: &[String], filter: &TraversalFilter) -> Result<Subgraph> {
        validate_traversal_filter(filter)?;
        if seeds.is_empty() {
            return Err(Error::InvalidQuery(
                "expand requires at least one seed".to_string(),
            ));
        }
        let state = self.lock()?;
        let mut node_ids = BTreeSet::new();
        let mut edge_keys = BTreeSet::new();
        let mut edges = Vec::new();
        let mut frontier = VecDeque::new();
        for seed in seeds {
            if let Some(node) = state.nodes.get(seed)
                && node_passes(node, filter)
                && node_ids.insert(seed.clone())
            {
                frontier.push_back((seed.clone(), 0_u8));
            }
        }
        while let Some((anchor, depth)) = frontier.pop_front() {
            if depth >= filter.max_hops {
                continue;
            }
            for (edge, node) in Self::hop(&state, &anchor, filter) {
                if edge_keys.insert(edge_identity(&edge)) {
                    edges.push(edge);
                }
                if node_ids.insert(node.id.clone()) {
                    frontier.push_back((node.id, depth + 1));
                }
            }
        }
        let nodes = node_ids
            .iter()
            .filter_map(|id| state.nodes.get(id).cloned())
            .collect();
        edges.sort_by_key(edge_sort_key);
        Ok(Subgraph { nodes, edges })
    }

    async fn shortest_path(
        &self,
        from: &str,
        to: &str,
        filter: &TraversalFilter,
    ) -> Result<Option<Vec<GraphEdge>>> {
        validate_traversal_filter(filter)?;
        let state = self.lock()?;
        if !state.nodes.contains_key(from) || !state.nodes.contains_key(to) {
            return Ok(None);
        }
        if from == to {
            return Ok(Some(Vec::new()));
        }
        let mut visited = BTreeSet::from([from.to_string()]);
        let mut frontier = VecDeque::from([(from.to_string(), Vec::<GraphEdge>::new())]);
        while let Some((anchor, path)) = frontier.pop_front() {
            if path.len() >= usize::from(filter.max_hops) {
                continue;
            }
            for (edge, node) in Self::hop(&state, &anchor, filter) {
                if !visited.insert(node.id.clone()) {
                    continue;
                }
                let mut next_path = path.clone();
                next_path.push(edge);
                if node.id == to {
                    return Ok(Some(next_path));
                }
                frontier.push_back((node.id, next_path));
            }
        }
        Ok(None)
    }

    async fn merge_entities(
        &self,
        source: &str,
        target: &str,
        decision: &MergeDecision,
    ) -> Result<MergeReport> {
        if source == target {
            return Err(Error::Storage(format!("self-merge rejected: {source}")));
        }
        if decision.approved_by.trim().is_empty()
            || decision.approved_by.chars().any(char::is_control)
        {
            return Err(Error::Storage(
                "merge approved_by must be non-empty and control-char-free".to_string(),
            ));
        }
        let mut state = self.lock()?;
        let source_node = state
            .nodes
            .get(source)
            .cloned()
            .ok_or_else(|| Error::Storage(format!("merge source missing: {source}")))?;
        let mut target_node = state
            .nodes
            .get(target)
            .cloned()
            .ok_or_else(|| Error::Storage(format!("merge target missing: {target}")))?;

        // Alias union: the source's aliases and label survive on the target.
        let mut absorbed_aliases = 0;
        for alias in source_node
            .aliases
            .iter()
            .chain(std::iter::once(&source_node.label))
        {
            if alias != &target_node.label && !target_node.aliases.contains(alias) {
                target_node.aliases.push(alias.clone());
                absorbed_aliases += 1;
            }
        }
        target_node.aliases.sort();

        // Edge rewiring: re-point every source edge at the target, preserving props,
        // provenance, and validity. Would-be self-loops and duplicates drop.
        let mut rewired_edges = 0;
        let mut kept = Vec::with_capacity(state.edges.len());
        let mut seen: BTreeSet<_> = BTreeSet::new();
        let existing_identities: BTreeSet<_> = state
            .edges
            .iter()
            .filter(|edge| edge.from != source && edge.to != source)
            .map(edge_identity)
            .collect();
        for mut edge in std::mem::take(&mut state.edges) {
            let touches_source = edge.from == source || edge.to == source;
            if touches_source {
                if edge.from == source {
                    edge.from = target.to_string();
                }
                if edge.to == source {
                    edge.to = target.to_string();
                }
                if edge.from == edge.to {
                    continue; // would-be self-loop on the target
                }
                let identity = edge_identity(&edge);
                if existing_identities.contains(&identity) || !seen.insert(identity) {
                    continue; // duplicate of a surviving or already-rewired edge
                }
                rewired_edges += 1;
            }
            kept.push(edge);
        }
        state.edges = kept;

        // Tombstone the source: it stays addressable but is marked merged.
        let mut tombstone = source_node;
        if let Value::Object(props) = &mut tombstone.props {
            props.insert("merged_into".to_string(), Value::String(target.to_string()));
        } else {
            tombstone.props = serde_json::json!({ "merged_into": target });
        }
        state.nodes.insert(source.to_string(), tombstone);
        state.nodes.insert(target.to_string(), target_node);

        // Audit edge with structured manual provenance.
        state.edges.push(GraphEdge {
            from: source.to_string(),
            to: target.to_string(),
            rel: "merged_into".to_string(),
            props: Value::Null,
            provenance: Provenance::Manual {
                approved_by: decision.approved_by.clone(),
            },
            valid_from: None,
            valid_to: None,
        });

        Ok(MergeReport {
            source: source.to_string(),
            target: target.to_string(),
            rewired_edges,
            absorbed_aliases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_graph_engine_contract() {
        fn assert_engine<T: GraphEngine>() {}

        assert_engine::<InMemoryGraphEngine>();
    }
}
