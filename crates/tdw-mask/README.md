# tdw-mask

Response masking: deterministic, fail-closed redaction of fields in a result row
before it leaves the platform.

`tdw-mask` applies a list of `MaskRule`s to a `BTreeMap<String, String>` row,
either fully redacting a field or keeping only its last four characters. It is
exposed as a `mask.sync_filter` hook (`tdw-hooks::HookSpec`) so the masking step
slots into the response pipeline.

## What it provides

- `MaskMode` — `Redact` (`"***"`) or `Last4` (`"***" + last 4 chars`).
- `MaskRule` — `{ field, mode }`.
- `apply_masks(row, rules)` — infallible wrapper that **fails closed**: on any
  invalid rule it redacts *every* field.
- `try_apply_masks(row, rules)` — fallible form returning `Result<_, MaskError>`.
- `validate_rules(rules)` — field-name validation.
- `masking_hook()` — the `mask.sync_filter` `HookSpec` (order 5, in-transaction).

## Feature flags

None. Depends only on `serde` and `tdw-hooks`.

## Quickstart

```rust
use std::collections::BTreeMap;
use tdw_mask::{apply_masks, MaskMode, MaskRule};

let mut row = BTreeMap::new();
row.insert("account_id".to_string(), "ACC123456".to_string());
row.insert("symbol".to_string(), "AAPL".to_string());

let masked = apply_masks(&row, &[MaskRule {
    field: "account_id".to_string(),
    mode: MaskMode::Last4,
}]);

assert_eq!(masked.get("account_id"), Some(&"***3456".to_string()));
assert_eq!(masked.get("symbol"), Some(&"AAPL".to_string())); // untouched
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-mask --example tdw_mask_basic
```

## Fail-closed behavior

`apply_masks` never errors. If `validate_rules` rejects the rule set (a field
name that is not dotted `[A-Za-z0-9_]` segments — e.g. `account-id` or
`account_id;drop`), `apply_masks` falls back to redacting **every** field in the
row rather than returning the row unmasked. Use `try_apply_masks` when you want
the typed `MaskError` instead of the safe fallback.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Fail closed.** An invalid rule set masks everything; sensitive data is never
  emitted because a rule was malformed.
- **Field-name safety.** Only `[A-Za-z0-9_]` segments separated by `.` are valid
  field names, blocking injection via crafted field strings.
- **Deterministic.** Rows are `BTreeMap`s (ordered); masking is pure.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
