---
batch: batch-lint-debt-006
items: lint:clippy::missing_const_for_fn
outcome: done
---

# batch-lint-debt-006 — missing_const_for_fn in tdw-domain

Fix ALL `clippy::missing_const_for_fn` (nursery) warnings in crate
**tdw-domain**. Only `const` was added to fn signatures where the body is
const-evaluable and compiles cleanly. No `#[allow]` added.

NOTE: a prior ledger at this path on origin/main covered a different,
already-shipped item (`too_long_first_doc_paragraph`); this batch reuses the
`batch-lint-debt-006` id for the `missing_const_for_fn` item.

## Scope

2 functions const-ified in `crates/tdw-domain/src/envelope.rs`:

- `ResultEnvelope::len(&self) -> usize` (line 137) — body is `self.results.len()` (`Vec::len` is const) → `pub const fn`.
- `ResultEnvelope::is_empty(&self) -> bool` (line 143) — body is `self.results.is_empty()` (const) → `pub const fn`.

clippy reported 2 lib warnings + 2 lib-test duplicates = 4 total occurrences,
resolved by the 2 source edits above. All were const-able and compiled cleanly.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| missing_const recheck | `cargo clippy -p tdw-domain --all-targets -- -W clippy::missing_const_for_fn` | 0 warnings |
| clean | `cargo clean -p tdw-domain` | pass (removed 1283 files) |
| fmt | `cargo fmt -p tdw-domain -- --check` | pass (exit 0) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | pass (exit 0) |
| tests | `cargo test --workspace` | pass (exit 0, 0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass ("clean-room audit passed") |
| ratchet | `cargo clippy -p tdw-domain --all-targets -- -W clippy::pedantic -W clippy::nursery` | 0 warnings (no regression) |

## PR

https://github.com/xrey167/FinX-Plattform/pull/260
(branch `work/batch-lint-debt-006-const` — the `work/batch-lint-debt-006`
remote branch name was already taken by a prior shipped batch.)
