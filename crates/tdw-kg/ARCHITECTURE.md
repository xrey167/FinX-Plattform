# tdw-kg — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Role |
| --- | --- |
| `EntityKind` | The closed set of entity kinds. |
| `Entity` / `Relationship` | The graph node + edge DTOs. |
| `KnowledgeGraphError` | The validation error enum. |
| `KnowledgeGraph` | The in-memory store. |
| `validate_entity` / `validate_relationship` | Field hygiene checks. |

## Storage model

`KnowledgeGraph` holds three collections:

- `entities: BTreeMap<String, Entity>` — keyed by `entity_id`; upsert replaces.
- `edges: Vec<Relationship>` — directed, append-only.
- `merge_audit: Vec<String>` — append-only human-readable merge log.

`BTreeMap` (not a hash map) gives a deterministic iteration/neighbor order, which
keeps downstream snapshots and tests stable.

## Mutation contract

Two tiers per mutation:

| Unchecked | Checked |
| --- | --- |
| `upsert_entity` | `try_upsert_entity` → `validate_entity` first |
| `add_relationship` | `try_add_relationship` → `validate_relationship` + both endpoints must already exist (else `MissingEndpoint`) |

`validate_entity`: id must be a graph id (`is_graph_id`: non-empty,
`[A-Za-z0-9:._-]`), label non-empty, every alias non-empty and control-free.

`validate_relationship`: `from`/`to`/`rel_type` must all be graph ids, and
`provenance` must be non-empty and control-free.

## Queries

- `entity(id)` — borrow by id.
- `neighbors(id)` — the de-duplicated (`BTreeSet`) set of entities reachable by an
  outgoing edge from `id`, returned in id order.
- `manual_merge(source, target, approved_by)` — records a `source->target
  approved_by=…` audit entry; returns `false` (no-op) if either endpoint is
  missing or `approved_by` is empty/control-bearing. Approval is mandatory and
  audited, never silent.
- `merge_audit()` — the audit slice.

## Offline test design

Pure in-memory unit tests: build a graph, assert neighbor order, exercise the
audited merge, and confirm the checked paths reject a traversal-style id and a
dangling edge. No async, no I/O, no network.
