# tdw-fn-string architecture

A single-module, dependency-free crate (`src/lib.rs`).

## Module map

| Item | Role |
|------|------|
| `CRATE_NAME: &str` | `"tdw-fn-string"`. |
| `StringFn` | The operation enum: `Trim`, `Uppercase`, `Lowercase`, `Replace { from, to }`. |
| `StringPipeline` | `{ name, steps: Vec<StringFn> }`. |
| `StringFnError` | `InvalidPipelineName` / `EmptyPipeline` / `EmptyPattern` / `UnsafePattern`. |
| `apply_pipeline(input, pipeline)` | Validate, then fold the steps over the input. |
| `validate_pipeline(pipeline)` | Validation only. |
| `is_identifier` / `contains_control_or_shell` (private) | The two safety guards. |

## Contract

The transform is a left fold over `steps`, but only after `validate_pipeline`
passes — so an invalid pipeline never partially mutates the input.

- `Trim` → `str::trim`.
- `Uppercase` / `Lowercase` → ASCII case folding (locale-independent,
  deterministic).
- `Replace { from, to }` → `str::replace`, after `from`/`to` clear the safety
  guard.

`validate_pipeline` enforces, in order: a valid identifier `name`, a non-empty
step list, and for each `Replace`: a non-empty `from` and no control/shell
characters (`;`, `|`, `` ` ``) in either `from` or `to`. The shell-metacharacter
guard exists because pipeline definitions can be authored as data; restricting
the replacement strings keeps a crafted pipeline from smuggling injection
payloads downstream.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Validate before transform.**
- **Deterministic & pure** — ASCII-only case folding, no allocation beyond the
  working string.
- **No control/shell injection** in `Replace` patterns.
- **No dependencies** — the crate stands alone.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
