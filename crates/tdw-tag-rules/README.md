# tdw-tag-rules

A hot-reloadable rule engine that turns predicates over an entity's label into
tag assignments written back to a `tdw-tags` `TagStore`. This is the automation
layer on top of the tag taxonomy.

## Purpose

A `TagRule` says "if this predicate matches, tag the entity with `tag_id`". The
`RuleEngine`:

- holds a versioned rule set and supports `hot_reload` (atomic swap, version bump);
- validates every rule on load — including rejecting unsafe SQL predicates and
  malformed JSON paths — before it can ever fire;
- `apply`s the rule set to an `(entity_id, label)` pair, writing matched
  assignments into a `TagStore` with `provenance = "rule:<rule_id>"`.

Three predicate kinds are supported: `SqlContains`, `JsonPathEquals`,
`LabelContains` (the current matcher evaluates all three against the supplied
`label`).

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `TagRule` / `RulePredicate` (de)serialization.
- `tdw-tags` — `TagStore` / `TagAssignment` the engine writes into.
- `thiserror` — `RuleError` variants.

## Quickstart

```rust
use tdw_tag_rules::{RuleEngine, RulePredicate, TagRule};
use tdw_tags::{TagDefinition, TagStore};

let mut tags = TagStore::default();
tags.define(TagDefinition { tag_id: "asset:equity".to_string(), parent: None, ttl_days: None })?;

let mut engine = RuleEngine::default();
engine.hot_reload(vec![TagRule {
    rule_id: "equity-symbol".to_string(),
    tag_id: "asset:equity".to_string(),
    predicate: RulePredicate::LabelContains { label: "AAPL".to_string() },
}])?;

let assigned = engine.apply("instrument:AAPL", "AAPL", &mut tags)?;
assert_eq!(assigned[0].provenance, "rule:equity-symbol");
assert_eq!(engine.version(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Run the worked example:

```text
cargo run -p tdw-tag-rules --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — predicate model, validation, hot reload.
- `tdw-tags` — the taxonomy/assignment store rules write into.
