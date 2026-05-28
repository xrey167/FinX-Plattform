#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-graph: traversal ordering, cycle detection,
// disconnected components, self-loop edge case, edge dedup via set semantics.
// Authored offline. Verify with `cargo test --package tdw-graph`.

use tdw_graph::DirectedGraph;

#[test]
fn empty_graph_traverse_returns_only_start_node() {
    let graph = DirectedGraph::default();
    assert_eq!(graph.traverse("alone"), vec!["alone".to_string()]);
}

#[test]
fn traversal_visits_each_node_exactly_once() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("a", "c");
    graph.add_edge("b", "c");
    let order = graph.traverse("a");
    assert_eq!(order.len(), 3);
    assert!(order.contains(&"a".to_string()));
    assert!(order.contains(&"b".to_string()));
    assert!(order.contains(&"c".to_string()));
}

#[test]
fn traversal_starting_from_disconnected_node_does_not_cross_components() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("x", "y");
    let order_a = graph.traverse("a");
    assert!(order_a.contains(&"a".to_string()));
    assert!(order_a.contains(&"b".to_string()));
    assert!(!order_a.contains(&"x".to_string()));
    assert!(!order_a.contains(&"y".to_string()));
}

#[test]
fn duplicate_edge_inserts_are_deduplicated() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("a", "b");
    graph.add_edge("a", "b");
    let order = graph.traverse("a");
    assert_eq!(order, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn has_cycle_false_for_acyclic_chain() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("b", "c");
    graph.add_edge("c", "d");
    assert!(!graph.has_cycle());
}

#[test]
fn has_cycle_true_for_three_node_cycle() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("b", "c");
    graph.add_edge("c", "a");
    assert!(graph.has_cycle());
}

#[test]
fn has_cycle_true_for_self_loop() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "a");
    assert!(graph.has_cycle());
}

#[test]
fn has_cycle_false_for_diamond() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("top", "left");
    graph.add_edge("top", "right");
    graph.add_edge("left", "bottom");
    graph.add_edge("right", "bottom");
    assert!(!graph.has_cycle());
}

#[test]
fn graph_round_trips_via_serde() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("a", "b");
    graph.add_edge("b", "c");
    let json = serde_json::to_string(&graph).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: DirectedGraph =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, graph);
}
