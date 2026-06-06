---
batch: batch-lint-debt-001
items: lint:clippy::use_self
outcome: done
---

# batch-lint-debt-001 — pilot lint-debt batch

First batch executed through the batch improvement harness (`/batch run
lint-debt`, backlog scan of 2026-06-06 on rustc 1.95.0).

## Scope

`lint:clippy::use_self` — 242 warnings (121 distinct fix sites), all in
`tdw-agent`: `crates/tdw-agent/src/kind.rs` (96), `base.rs` (22), `mcp.rs` (2),
`registry.rs` (1). Applied via `cargo clippy --fix -p tdw-agent --all-targets
-- -W clippy::use_self` (machine-applicable `Self` substitutions only; no
behavior change, 114 insertions / 114 deletions).

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | pass |
| clippy | `cargo clean -p tdw-agent && cargo clippy --workspace --all-targets -- -D warnings` | pass |
| tests | `cargo test --workspace` | pass (0 failed) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | pass |

Re-check after fix: `cargo clippy -p tdw-agent --all-targets -- -W
clippy::use_self` reports zero `use_self` warnings.

## PR

(link added on creation)

## Notes

- Pilot run validating the harness loop end-to-end; friction findings feed
  back into `.claude/commands/batch.md`.
- Backlog untouched by this branch (single-writer rule) — verified via
  `git diff --name-only main...HEAD`.
