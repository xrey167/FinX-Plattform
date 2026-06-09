#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectedGraph {
    edges: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    InvalidNodeId,
    SelfLoop,
}

impl DirectedGraph {
    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.entry(from.into()).or_default().insert(to.into());
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_add_edge(&mut self, from: &str, to: &str) -> Result<(), GraphError> {
        if !is_node_id(from) || !is_node_id(to) {
            return Err(GraphError::InvalidNodeId);
        }
        if from == to {
            return Err(GraphError::SelfLoop);
        }
        self.add_edge(from, to);
        Ok(())
    }

    #[must_use]
    pub fn traverse(&self, start: &str) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::from([start.to_string()]);
        let mut ordered = Vec::new();
        while let Some(node) = queue.pop_front() {
            if !seen.insert(node.clone()) {
                continue;
            }
            ordered.push(node.clone());
            if let Some(next) = self.edges.get(&node) {
                queue.extend(next.iter().cloned());
            }
        }
        ordered
    }

    #[must_use]
    pub fn has_cycle(&self) -> bool {
        self.edges.keys().any(|node| {
            self.edges.get(node).is_some_and(|targets| {
                targets
                    .iter()
                    .any(|target| self.traverse(target).contains(node))
            })
        })
    }
}

fn is_node_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traverses_graph_and_detects_cycle() {
        let mut graph = DirectedGraph::default();
        graph.add_edge("account", "position");
        graph.add_edge("position", "instrument");
        assert_eq!(
            graph.traverse("account"),
            vec![
                "account".to_string(),
                "position".to_string(),
                "instrument".to_string()
            ]
        );
        assert!(!graph.has_cycle());
        graph.add_edge("instrument", "account");
        assert!(graph.has_cycle());
    }

    #[test]
    fn checked_edges_reject_invalid_nodes_and_self_loops() {
        let mut graph = DirectedGraph::default();
        assert_eq!(graph.try_add_edge("account", "position"), Ok(()));
        assert_eq!(
            graph.try_add_edge("account", "account"),
            Err(GraphError::SelfLoop)
        );
        assert_eq!(
            graph.try_add_edge("../account", "position"),
            Err(GraphError::InvalidNodeId)
        );
    }
}
