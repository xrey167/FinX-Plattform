---
batch: batch-lint-debt-002
items: lint:clippy::doc_markdown
outcome: done
---

# batch-lint-debt-002 — doc_markdown (first 10 crates)

Second lint-debt batch (backlog seed of 2026-06-06, PR #145; rustc 1.95.0).

## Scope

`lint:clippy::doc_markdown` (74 warnings across 13 crates + 1 test-target
alias) — capped at 10 crates per the batch sizing rule: tdw-agent,
tdw-app-server, tdw-backend, tdw-knowledge, tdw-proto,
tdw-provider-{akshare,ccdata,coingecko,geckoterminal,seeking-alpha}.
Applied via `cargo clippy --fix --all-targets -p <10 crates> -- -W
clippy::doc_markdown` (35 machine-applicable fixes, 12 files).

## Deliberate exclusions / residuals (8 warnings remain in scope)

- `crates/tdw-proto/src/finance.gen.rs` (3) — GENERATED file (vendored
  protobuf bindings); hand-edits would be silently lost on regeneration.
  Reverted after --fix. Fix belongs in the proto doc comments upstream.
- ~5 non-machine-applicable sites (incl. tdw-app-server) — need manual
  doc rewording, not worth blocking this mechanical batch.
- Out of batch scope (sizing cap): tdw-sandbox, tdw-service-api, tdw-worker
  → next lint-debt batch.

The next `/batch discover` will re-detect the residuals and reopen the item
with the reduced count (`reopened-from: done`) — by design.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p <8 touched> && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
