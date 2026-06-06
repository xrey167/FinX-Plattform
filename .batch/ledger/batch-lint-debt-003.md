---
batch: batch-lint-debt-003
items: lint:clippy::doc_markdown
outcome: done
---

# batch-lint-debt-003 — doc_markdown remainder + harness gate amendments

Completes the `lint:clippy::doc_markdown` family (batches 002+003): the 3
crates deferred by batch-002's sizing cap.

## Scope

- `cargo clippy --fix --all-targets -p tdw-sandbox -p tdw-service-api -p tdw-worker
  -- -W clippy::doc_markdown` — 3 machine-applicable fixes (one per crate).
- Harness amendments to `.claude/commands/batch.md` from batch-002 lessons:
  (a) revert `*.gen.rs` after `clippy --fix`; (b) run `schema-sync` +
  schema-checks when doc comments change in schema-bearing crates
  (tdw-agent/tdw-event/tdw-protocol/tdw-config) — CI diffs the generated
  schemas (PR #146 failure).

## Residuals

The non-machine-applicable doc_markdown sites and the reverted
`finance.gen.rs` warnings from batch-002 remain; the next discover scan
reopens the item with the residual count (`reopened-from: done`).

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p <3 touched> && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| wasmi combo | `cargo clippy -p tdw-udf-wasm --features wasmi --all-targets -- -D warnings` | pass |
| udf-wasm combo | `cargo clippy -p tdw-sandbox -p tdw-service-api --features tdw-sandbox/udf-wasm,tdw-service-api/udf-wasm --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
