# tdw-graph — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `DirectedGraph` | Adjacency-set graph: `BTreeMap<String, BTreeSet<String>>`. |
| `GraphError` | `InvalidNodeId`, `SelfLoop`. |
| `add_edge` / `try_add_edge` | Insert an edge (unchecked / validated). |
| `traverse` | BFS reachable order from a start node. |
| `has_cycle` | Reachability-based cycle detection. |
| `is_node_id` | Internal node-id grammar check. |

## Key types and traits

- **`DirectedGraph`** derives `Clone, Debug, Default, PartialEq, Eq, Serialize,
  Deserialize`. The edge map is private; mutation goes only through `add_edge` /
  `try_add_edge`, keeping the adjacency representation an implementation detail.
- **`GraphError`** is a `Copy` enum (no `serde`).

## Data flow

```
add_edge(from, to) ──▶ edges[from] ∪= {to}        (BTreeSet dedups parallel edges)

traverse(start):
    queue = [start]; seen = {}
    while queue: pop front; if newly seen, emit + enqueue its targets
    ▶ Vec<String> in deterministic (sorted-neighbor) BFS order

has_cycle():
    for each node, for each target: traverse(target) contains node?
    ▶ bool
```

Because neighbours are stored in a `BTreeSet` and visited via a `VecDeque`,
`traverse` output is deterministic for a given edge set — important for
reproducible job ordering.

## Invariants

- **Node-id grammar** (`try_add_edge` only): non-empty, ASCII alphanumeric plus
  `:`, `.`, `_`, `-`. `add_edge` does **not** validate — use `try_add_edge` for
  untrusted input (e.g. `../account` is rejected as `InvalidNodeId`).
- `try_add_edge` rejects self-loops (`from == to`) with `SelfLoop`; `add_edge`
  would happily insert one.
- `traverse` never revisits a node (`seen` set), so it terminates even on cyclic
  graphs.
- `has_cycle` is O(V·(V+E)) — it re-traverses per edge. It is intended for the
  small DAGs the platform builds (job/lineage graphs), not large-scale graphs.
- All operations are pure; the graph holds no I/O or global state.
