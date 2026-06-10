# tdw-storage-graph

The graph-storage backends for the [`GraphEngine`](../tdw-core/src/graph.rs) contract
(knowledge-system overhaul A2).

## What lives here

- **`InMemoryGraphEngine`** — the deterministic, no-I/O reference implementation: the
  test/conformance/embedded backend, exactly as `InMemoryVectorEngine` is for the
  vector contract.
- **`tests/conformance.rs`** — ONE behavioral suite asserted identically against every
  backend: upsert/read round-trips, directional + rel/kind/as_of-filtered
  neighborhoods, hop-bounded expansion, shortest paths, and real merge semantics
  (alias union, edge rewiring, tombstone, audit edge).
- Slice A4 adds the **Bolt backend** (Memgraph primary, Neo4j-compatible by
  construction) as a feature-gated second conformance leg.

## Contract highlights

- **Temporal**: nodes/edges carry an optional half-open `[valid_from, valid_to)`
  window (RFC 3339, lexicographic); every read takes `as_of` (`None` = no temporal
  filtering).
- **Provenance is structured** (`Ingest`/`Rule`/`Agent`/`Manual`/`System`), never a
  free string.
- **Merges are real**: alias union, edge rewiring with provenance preserved,
  tombstone (`props.merged_into`), and a `merged_into` audit edge — not just an audit
  log entry.
- **Determinism**: all read orders are sorted (`(rel, neighbor, valid_from)` for
  hops; id-sorted subgraphs), so identical calls yield identical results on every
  backend.
