---
batch: batch-lint-debt-004
items: lint:clippy::missing_const_for_fn
outcome: done
---

# batch-lint-debt-004 — missing_const_for_fn (first 10 crates, wave 2)

First batch of wave 2 (backlog refreshed via #148, 30 items; rustc 1.95.0).
Started once the v1.0.0 release-train merge queue drained.

## Scope

`lint:clippy::missing_const_for_fn` (62 warnings across 16 crates) — capped
at 10 crates per the sizing rule: tdw-agent, tdw-backend, tdw-proto,
tdw-provider-{adanos,alpha-vantage,benzinga,coingecko,deribit,eia,finra}.
Applied via `cargo clippy --fix --all-targets -p <10 crates> -- -W
clippy::missing_const_for_fn` — 25 const-ifications across 12 files (after
the generated-file revert below). Wrong const-ness would fail compilation,
so the workspace clippy/test gates double as semantic verification.

## Deliberate exclusions / residuals

- `crates/tdw-proto/src/finance.gen.rs` — GENERATED (vendored protobuf);
  reverted after `--fix` per the batch.md rule (lesson batch-002).
- Out of batch scope (sizing cap): tdw-provider-{glassnode,seeking-alpha,
  trading-economics,velodata}, tdw-tool-exec, tdw-worker → batch-005.

The next `/batch discover` reopens the item with the residual count
(`reopened-from: done`) — by design.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p <9 touched> && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
