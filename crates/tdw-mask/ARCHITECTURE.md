# tdw-mask architecture

A single-module crate (`src/lib.rs`) implementing fail-closed field masking and
its hook binding.

## Module map

| Item | Role |
|------|------|
| `MaskMode` | `Redact` / `Last4`. |
| `MaskRule` | `{ field: String, mode: MaskMode }`. |
| `MaskError` | `InvalidFieldName`. |
| `apply_masks(row, rules) -> BTreeMap` | Infallible, fail-closed entry point. |
| `try_apply_masks(row, rules) -> Result<BTreeMap, MaskError>` | Fallible form. |
| `validate_rules(rules) -> Result<(), MaskError>` | Field-name guard. |
| `masking_hook() -> HookSpec` | The `mask.sync_filter` hook. |
| `is_field_name` / `redact_all` (private) | Charset guard / total-redaction fallback. |

## Masking contract

`try_apply_masks` is the core:

1. `validate_rules` — every rule's `field` must be a dotted name whose segments
   are non-empty and all `[A-Za-z0-9_]`. Otherwise `Err(MaskError::InvalidFieldName)`.
2. clone the row, then for each rule whose field is present, replace the value:
   - `Redact` → `"***"`;
   - `Last4` → `"***"` followed by the last four characters of the value
     (computed `rev().take(4).rev()` so it is correct for multi-byte input and
     for values shorter than four characters).
3. fields not named by any rule are passed through unchanged.

## Fail-closed wrapper

`apply_masks` is the production entry point and is deliberately infallible:

```rust
try_apply_masks(row, rules).unwrap_or_else(|_| redact_all(row))
```

If the rule set is invalid, it does **not** return the original row (which would
leak the very fields the operator tried to mask) — it `redact_all`s, replacing
every value with `"***"`. The safe failure mode is "show nothing", never "show
everything".

## Hook binding

`masking_hook()` returns `HookSpec::new("mask.sync_filter", 5,
TransactionMode::InTransaction)` — order 5, running inside the transaction so the
masked projection is what the rest of the response pipeline sees. This is how the
crate plugs into `tdw-hooks` without the hook engine needing to know about
masking specifically.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Fail closed.** Invalid rules → total redaction, never pass-through.
- **Field-name safety.** Crafted field strings (separators, injection markers)
  are rejected by `is_field_name`.
- **Deterministic & pure.** Ordered `BTreeMap` I/O, no side effects.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy), and clean-room — no
  vendor-derived code or branding.
