# tdw-tags

A hierarchical tag taxonomy with time-bounded, provenance-tracked assignments.
Entities (instruments, accounts, datasets) are labelled with tags drawn from a
parent/child taxonomy; assignments carry validity windows so "active" tags can be
queried as of any date.

## Purpose

`TagStore` is the in-memory home of two things:

- **definitions** — a DAG of `TagDefinition`s (`tag_id`, optional `parent`,
  optional `ttl_days`). Defining a tag validates its id grammar, checks the parent
  exists, and rejects cycles;
- **assignments** — `TagAssignment`s binding an `entity_id` to a `tag_id` over a
  `[assigned_at, expires_at)` window with a `provenance` string.

It answers point-in-time questions (`active_tags(entity, as_of)`) and reports
usage (`taxonomy_stats`). All inputs are validated against safe id/date grammars.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `TagDefinition` / `TagAssignment` (de)serialization.
- `thiserror` — `TagError` variants.

## Quickstart

```rust
use tdw_tags::{TagAssignment, TagDefinition, TagStore};

let mut store = TagStore::default();
store.define(TagDefinition {
    tag_id: "asset:equity".to_string(),
    parent: None,
    ttl_days: None,
})?;
store.assign(TagAssignment {
    entity_id: "instrument:AAPL".to_string(),
    tag_id: "asset:equity".to_string(),
    assigned_at: "2026-05-21".to_string(),
    expires_at: Some("2026-06-20".to_string()),
    provenance: "manual".to_string(),
})?;

// Active as of a date inside the window; empty after it expires.
assert_eq!(store.active_tags("instrument:AAPL", "2026-05-22"), vec!["asset:equity"]);
assert!(store.active_tags("instrument:AAPL", "2026-07-01").is_empty());
# Ok::<(), tdw_tags::TagError>(())
```

Run the worked example:

```text
cargo run -p tdw-tags --example tdw-tags-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — taxonomy DAG, validity-window model.
- `tdw-tag-rules` — declarative rules that produce assignments into a `TagStore`.
- `tdw-feature-store` — stamps a snapshot's active tags from a `TagStore`.
