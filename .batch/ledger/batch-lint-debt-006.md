---
batch: batch-lint-debt-006
items: lint:clippy::too_long_first_doc_paragraph
outcome: done
---

# batch-lint-debt-006 — too_long_first_doc_paragraph (closes the family)

First batch requiring MANUAL judgment fixes (the lint is not
machine-applicable — `clippy --fix` applied zero changes). Fixes were
delegated to an executor agent per the harness design and independently
verified: 0 warnings remain.

## Scope

16 doc-comment sites split (short summary sentence + blank `///` line +
remainder) across 8 files in tdw-agent, tdw-agent-store, tdw-backend,
tdw-eval-runner. tdw-agent is schema-bearing: `cargo run -p xtask --
schema-sync` regenerated `docs/schemas/agent/storage_mapping.schema.json`
(committed here, per the batch.md rule from #146).

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p <4 touched> && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| quality-gate | `cargo run -p xtask -- quality-gate check` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |
| lint re-check | `cargo clippy --all-targets -p <4 crates> -- -W clippy::too_long_first_doc_paragraph` | 0 warnings |

## PR

(link added on creation)
