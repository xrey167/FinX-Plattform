---
batch: batch-test-gaps-001
items: test-gap:tdw-proto
outcome: done
---

# batch-test-gaps-001 — characterization tests for tdw-proto

First test-gaps batch. Chosen over the next lint item
(`missing_const_for_fn`, 16 crates) to stay additive-only while the parallel
v1.0.0 release train (#148, g001/g101/g108 worktrees) is churning main —
new test files cannot conflict with release/security diffs.

## Scope

- `crates/tdw-proto/tests/types.rs` — 21 characterization tests for the
  vendored prost market-data types: encode→decode round-trips with field
  preservation for OhlcvBar/Tick/OrderBookSnapshot/PriceLevel/
  MarketDataEnvelope/Payload/TradeSide, plus proto3 default-elision
  (empty-message ⇒ zero bytes) semantics.

## Skipped (correctly, not failures)

- `test-gap:tdw-cli` and `test-gap:tdw-bootstrap` — both are `main.rs`-only
  binary crates with no lib target; they cannot be tested without source
  changes (lib split), which violates this batch's additive-only rule.
  They remain `pending` in the backlog; a future batch may do the lib split
  as a deliberate refactor.

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p tdw-proto && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed; +21 new in tdw-proto) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

## PR

(link added on creation)
