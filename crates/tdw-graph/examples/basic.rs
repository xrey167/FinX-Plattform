//! Offline `tdw-graph` example: build a small dependency graph, traverse it, and
//! show how closing a loop is detected as a cycle.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-graph --example tdw-graph-basic
//! ```

use tdw_graph::DirectedGraph;

fn main() {
    let mut graph = DirectedGraph::default();
    graph.add_edge("account", "position");
    graph.add_edge("position", "instrument");

    // Meaningful operation: deterministic reachable order from a start node.
    let order = graph.traverse("account");
    println!("traversal from account: {order:?}");
    println!("acyclic? {}", !graph.has_cycle());

    // The checked API rejects self-loops and malformed node ids.
    println!(
        "self-loop rejected: {}",
        graph.try_add_edge("account", "account").is_err()
    );
    println!(
        "bad id rejected: {}",
        graph.try_add_edge("../account", "position").is_err()
    );

    // Closing the loop turns the DAG into a cyclic graph.
    graph.add_edge("instrument", "account");
    println!("cycle after closing loop? {}", graph.has_cycle());
}
