# tdw-graph

A tiny, dependency-free directed graph used for dependency ordering and
cycle detection across the platform (job DAGs, lineage, entity links).

## Purpose

`DirectedGraph` is an adjacency-set graph keyed by string node ids. It provides:

- `add_edge` / `try_add_edge` — insert an edge (the checked variant rejects
  invalid node ids and self-loops);
- `traverse` — deterministic breadth-first reachable order from a start node;
- `has_cycle` — whether any node is reachable back to itself.

Node ids are validated against a safe grammar (alphanumeric plus `: . _ -`), so
the graph doubles as a guard against malformed identifiers in DAG specs.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `DirectedGraph` is `Serialize`/`Deserialize` so DAGs can be persisted.

## Quickstart

```rust
use tdw_graph::DirectedGraph;

let mut graph = DirectedGraph::default();
graph.add_edge("account", "position");
graph.add_edge("position", "instrument");

assert_eq!(
    graph.traverse("account"),
    vec!["account".to_string(), "position".to_string(), "instrument".to_string()]
);
assert!(!graph.has_cycle());

// Closing the loop introduces a cycle.
graph.add_edge("instrument", "account");
assert!(graph.has_cycle());
```

Run the worked example:

```text
cargo run -p tdw-graph --example tdw-graph-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — traversal/cycle model and id grammar.
- `tdw-pipeline` — a job-DAG validator with the same dependency-ordering intent.
