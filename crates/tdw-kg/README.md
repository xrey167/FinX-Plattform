# tdw-kg

In-memory knowledge graph: entities, relationships, neighbor queries, and an
audited manual-merge trail.

## Purpose

`tdw-kg` is a small, dependency-light graph store used by `tdw-knowledge` to back
the entity/relationship side of document indexing. It holds typed entities keyed
by id, directed relationships, and an append-only merge-audit log, with checked
mutation paths that reject malformed ids and dangling edges.

Core surface:

- [`Entity`] — `{ entity_id, kind, label, aliases }` with [`EntityKind`]
  (`Instrument` / `Account` / `Strategy` / `Agent` / `Dataset`).
- [`Relationship`] — `{ from, to, rel_type, provenance }`.
- [`KnowledgeGraph`] — `upsert_entity`, `try_upsert_entity`, `add_relationship`,
  `try_add_relationship`, `entity`, `neighbors`, `manual_merge`, `merge_audit`.
- Validators: [`validate_entity`], [`validate_relationship`].

## Feature flags

None. The only dependency is `serde`.

## Environment variables

None.

## Quickstart

```rust
use tdw_kg::{Entity, EntityKind, KnowledgeGraph, Relationship};

let mut kg = KnowledgeGraph::default();
kg.upsert_entity(Entity {
    entity_id: "instrument:AAPL".to_string(),
    kind: EntityKind::Instrument,
    label: "Apple".to_string(),
    aliases: vec!["AAPL".to_string()],
});
kg.upsert_entity(Entity {
    entity_id: "dataset:ohlcv".to_string(),
    kind: EntityKind::Dataset,
    label: "OHLCV".to_string(),
    aliases: Vec::new(),
});
kg.add_relationship(Relationship {
    from: "instrument:AAPL".to_string(),
    to: "dataset:ohlcv".to_string(),
    rel_type: "has_prices".to_string(),
    provenance: "fixture".to_string(),
});
assert_eq!(kg.neighbors("instrument:AAPL")[0].entity_id, "dataset:ohlcv");
```

Prefer `try_upsert_entity` / `try_add_relationship` for the checked path: ids must
be `[A-Za-z0-9:._-]`, and an edge whose endpoints are not both present is rejected
with `MissingEndpoint`.

## Example

```text
cargo run --example tdw_kg_basic -p tdw-kg
```

`examples/basic.rs` builds an in-memory graph, queries neighbors, records an
audited manual merge, and shows the checked-path rejections — all in-process.
