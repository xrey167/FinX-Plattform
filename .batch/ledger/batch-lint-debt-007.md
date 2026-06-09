---
batch: batch-lint-debt-007
items: lint:clippy::redundant_clone
outcome: done
---

# batch-lint-debt-007 — redundant_clone cleanup (tdw-event)

Fix ALL `clippy::redundant_clone` (nursery) warnings in the **tdw-event** crate.
Scope is strictly one lint family in one crate.

NOTE: a prior `batch-lint-debt-007.md` ledger (for the unrelated
`missing_const_for_fn` family, merged via #232) shared this filename on
origin/main. This branch re-purposes the 007 slot for the `redundant_clone`
backlog item and the ledger has been rewritten to document the actual work on
`work/batch-lint-debt-007`.

## Lint family

`clippy::redundant_clone` (nursery) — a `.clone()`/`.to_owned()` is unnecessary
because the source value is not used afterward and can be moved instead.

## Discovery

```
cargo clippy -p tdw-event --all-targets -- -W clippy::redundant_clone
warning: redundant clone
   --> crates\tdw-event\src\lib.rs:233:37
    |
233 |             Some(child.trace.span_id.clone())
    |                                     ^^^^^^^^ help: remove this
note: this value is dropped without further use
```

One warning, one crate.

## Fix applied (1)

| Crate | File:line | Change |
| --- | --- | --- |
| tdw-event | src/lib.rs:233 | `Some(child.trace.span_id.clone())` → `Some(child.trace.span_id)` |

This is inside the `grandchild_inherits_root_correlation_id` test. Line 233 is
the last use of `child`, so the `String` is moved out of `child.trace.span_id`
rather than cloned. Behavior is unchanged (the assertion compares the same
value). After removing the clone the expression fit on one line, so rustfmt
collapsed the `assert_eq!` to a single line.

## Schema-sync (schema-bearing crate safety check)

tdw-event is schema-bearing, so after the fix:

```
cargo run -p xtask -- schema-sync
schema-sync wrote 9 agent schemas to docs/schemas/agent
git status --porcelain docs/schemas/
(no output)
```

NO changes in `docs/schemas/` — the redundant_clone fix did not alter any
generated schema (as expected; schemas derive from type/doc structure, not
call-site clones). Nothing staged under docs/schemas/.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| clean | `cargo clean -p tdw-event` | Removed 1294 files, 158.9MiB |
| fmt | `cargo fmt -p tdw-event -- --check` | pass |
| redundant_clone | `cargo clippy -p tdw-event --all-targets -- -W clippy::redundant_clone` | 0 warnings |
| pedantic/nursery | `cargo clippy -p tdw-event --all-targets -- -W clippy::pedantic -W clippy::nursery` | 0 warnings (no regression) |
| workspace clippy | `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests (crate) | `cargo test -p tdw-event` | 14 passed, 0 failed |
| tests (workspace) | `cargo test --workspace` | pass, 0 failed |
| clean-room | `cargo run -p xtask -- clean-room-audit` | `clean-room audit passed` |

## Ratchet

`redundant_clone` in tdw-event = 0. No new pedantic/nursery warnings
introduced; workspace pedantic+nursery baseline (14) not regressed.

## Reverted

None.

## Clean-room

No AGPL code copied. No `finx-*`, no FinX-XR, no `tdw-provider-openbb`.

## PR

https://github.com/xrey167/FinX-Plattform/pull/261
