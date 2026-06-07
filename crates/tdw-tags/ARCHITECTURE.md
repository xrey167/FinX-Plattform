# tdw-tags — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `TagDefinition` | Taxonomy node: `tag_id`, optional `parent`, optional `ttl_days`. |
| `TagAssignment` | Entity↔tag binding with `assigned_at`/`expires_at`/`provenance`. |
| `TagStore` | Holds definitions (`BTreeMap`) + assignments (`Vec`). |
| `TagError` | `Cycle`, `UnknownTag`, `UnknownParent`, `InvalidTagId`, `InvalidAssignment`. |
| `define` / `assign` | Mutators (validated). |
| `active_tags` / `taxonomy_stats` / `assignments` | Queries. |
| `has_cycle` (private) | Parent-chain cycle walk. |
| `validate_tag_id` / `validate_assignment` / `is_tag_id` / `is_entity_id` / `is_date` | Grammar checks. |

## Key types and traits

- `TagDefinition` and `TagAssignment` derive `Clone, Debug, PartialEq, Eq,
  Serialize, Deserialize`.
- `TagStore` derives `Clone, Debug, Default`; its fields are private, so the
  taxonomy can only change through the validated `define`/`assign` paths.
- `TagError` uses `thiserror`.

## Taxonomy + validity-window model

```
define(tag):
    validate tag_id grammar
    if parent: validate grammar + parent must already exist (UnknownParent)
    reject ttl_days == 0
    insert; then walk parent chain — if it revisits a node, roll back (Cycle)

assign(assignment):
    validate assignment (id grammar, date shape, provenance, window ordering)
    tag_id must already be defined (UnknownTag)
    push onto assignments

active_tags(entity, as_of):
    assignments where entity matches
        AND assigned_at <= as_of
        AND (expires_at is None OR expires_at > as_of)
    ▶ Vec<tag_id>
```

The validity window is half-open: `[assigned_at, expires_at)`. Dates are compared
as strings, which is correct because the enforced format is fixed-width
`YYYY-MM-DD` (lexicographic order == chronological order).

## Invariants

- **Tag-id grammar**: non-empty, must contain `:`, otherwise ASCII alphanumeric
  plus `:`, `_`, `-` (e.g. `asset:equity`). `asset/equity` is rejected.
- **Entity-id grammar**: non-empty, ASCII alphanumeric plus `:`, `.`, `_`, `-`.
- **Dates**: exactly `YYYY-MM-DD` with `-` at positions 4 and 7, digits elsewhere.
- **Assignment validity**: `expires_at`, when present, must be a valid date strictly
  greater than `assigned_at`; provenance must be non-empty and control-char-free.
  Any violation → `InvalidAssignment`.
- **Acyclic taxonomy**: defining a tag that would create a parent cycle is rolled
  back and returns `Cycle`.
- `ttl_days == Some(0)` is rejected (a zero TTL is meaningless).
- Pure and deterministic: no I/O, no clock — "now" is always supplied by the caller
  as `as_of`.
