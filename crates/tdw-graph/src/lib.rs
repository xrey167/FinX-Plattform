#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectedGraph {
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl DirectedGraph {
    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.entry(from.into()).or_default().insert(to.into());
    }

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
}
