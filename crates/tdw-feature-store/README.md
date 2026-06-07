# tdw-feature-store

Point-in-time feature snapshots stamped with the entity's active tags. It
materializes a named bag of numeric features for an entity as of a date and
records the tags that were active at that moment.

## Purpose

`FeatureStore` turns "the features for this entity, as of this date" into a stored
`FeatureSnapshot`:

- `entity_id`, `as_of` — who and when;
- `features` — a `BTreeMap<String, f64>` of feature name → value;
- `tags` — the entity's active tags pulled from a `tdw_tags::TagStore` at `as_of`.

`materialize` is the infallible primitive; `try_materialize` validates the request
first (entity-id grammar, date shape, feature-name grammar, finite values).
`latest(entity)` returns the most recently materialized snapshot for an entity.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `FeatureSnapshot` (de)serialization.
- `tdw-tags` — `TagStore` consulted for active tags at `as_of`.

## Quickstart

```rust
use std::collections::BTreeMap;
use tdw_feature_store::FeatureStore;
use tdw_tags::TagStore;

let tags = TagStore::default();
let mut features = BTreeMap::new();
features.insert("return_1d".to_string(), 0.01);

let mut store = FeatureStore::default();
let snapshot = store.materialize("instrument:AAPL", "2026-05-21", features, &tags);

assert_eq!(snapshot.as_of, "2026-05-21");
assert_eq!(store.latest("instrument:AAPL").map(|s| s.as_of.clone()), Some("2026-05-21".to_string()));
```

Run the worked example:

```text
cargo run -p tdw-feature-store --example tdw-feature-store-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — snapshot model, tag stamping, validation.
- `tdw-tags` — the source of the active tags stamped onto each snapshot.
