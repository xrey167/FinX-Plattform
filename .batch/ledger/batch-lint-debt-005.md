---
batch: batch-lint-debt-005
items: lint:clippy::missing_const_for_fn
outcome: done
---

# batch-lint-debt-005 — missing_const_for_fn remainder (closes the family)

Completes `lint:clippy::missing_const_for_fn` (batches 004+005): the 6 crates
deferred by batch-004's ≤10-crate sizing cap.

## Scope

`cargo clippy --fix --all-targets -p tdw-provider-glassnode -p
tdw-provider-seeking-alpha -p tdw-provider-trading-economics -p
tdw-provider-velodata -p tdw-tool-exec -p tdw-worker -- -W
clippy::missing_const_for_fn` — 6 const-ifications (one per crate, 6
insertions/6 deletions). No generated files touched this time. Wrong
const-ness fails compilation, so the workspace gates double as semantic
verification.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p <6 touched> && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
