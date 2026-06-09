# tdw-feature-store — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `FeatureSnapshot` | `entity_id`, `as_of`, `features: BTreeMap<String, f64>`, `tags: Vec<String>`. |
| `FeatureError` | `InvalidEntityId`, `InvalidAsOf`, `InvalidFeatureName`, `NonFiniteFeatureValue`. |
| `FeatureStore` | Append-only `Vec<FeatureSnapshot>`. |
| `materialize` / `try_materialize` | Stamp a snapshot (infallible / validated). |
| `latest` | Most recent snapshot for an entity. |
| `validate_feature_request` | Public request validator. |
| `is_entity_id` / `is_feature_name` / `is_date` (private) | Grammar checks. |

## Key types and traits

- `FeatureSnapshot` derives `Clone, Debug, PartialEq, Serialize, Deserialize`
  (`PartialEq` not `Eq` — it holds `f64`).
- `FeatureError` is a `Copy` enum (no `serde`).
- `FeatureStore` derives `Clone, Debug, Default`; the snapshot `Vec` is private and
  append-only.

## Snapshot / tag-stamping model

```
materialize(entity_id, as_of, features, &tags):
    snapshot.tags = tags.active_tags(entity_id, as_of)   // point-in-time tag stamp
    push snapshot; return clone

try_materialize(...):
    validate_feature_request(entity_id, as_of, &features)?  // grammar + finiteness
    materialize(...)

latest(entity_id):
    iterate snapshots in reverse, return first with matching entity_id
```

The key idea is **temporal consistency**: a snapshot's tags are exactly those the
`TagStore` considered active at the snapshot's `as_of`, so a feature row and its
tags always agree on the point in time.

## Invariants

- **Entity-id grammar**: non-empty ASCII alphanumeric plus `:`, `.`, `_`, `-`.
- **`as_of` / date**: exactly `YYYY-MM-DD`.
- **Feature-name grammar**: non-empty ASCII alphanumeric plus `_`, `-`.
- **Feature values must be finite** — `NaN`/`±inf` are rejected by
  `try_materialize` (`NonFiniteFeatureValue`); `materialize` does not check.
- `features` is a `BTreeMap`, so feature ordering within a snapshot is
  deterministic (sorted by name).
- `latest` reflects insertion order (last materialized wins), not `as_of` ordering;
  the store is an append log, not a sorted index.
- Pure and deterministic: no I/O, no clock — the caller supplies `as_of`.
