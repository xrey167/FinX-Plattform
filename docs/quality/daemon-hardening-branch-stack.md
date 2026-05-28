# Daemon Hardening Branch Stack Status

Generated for Ultragoal `G001-reconcile-branch-stack-and-governance`.

## Current checkout

- Worktree: primary checkout of `FinX-Plattform` (sibling-worktree migration is itself a tracked governance item below).
- Branch: `feat/p8-quality-gate` (tip of the stack at the time this note was last refreshed).
- HEAD: `a10ef3b` (`docs(quality): add UDF runtime hardening scope honesty note (P8)`).
- `main`: `a1677b6` (`test(ultraqa): salvage validated characterization coverage`).
- Local dirty state before this note:
  - `Cargo.lock` only, adding dependency resolution for `tdw-app-server` and `tokio-util`-related crates.
- Local dirty state after Ultragoal setup:
  - `.omx/ultragoal/brief.md`
  - `.omx/ultragoal/goals.json`
  - `.omx/ultragoal/ledger.jsonl`
  - `Cargo.lock`
  - `crates/tdw-sandbox/Cargo.toml`
  - `crates/tdw-sandbox/src/lib.rs`
  - `crates/tdw-service-api/Cargo.toml`
  - `crates/tdw-udf-wasm/src/lib.rs`
  - this status note

## Branch stack

The daemon work is not on `main` yet. It is a stacked PR series:

| PR | Head | Base | Status | Notes |
|---|---|---|---|---|
| #51 | `feat/p0-app-state-composition-root` | `main` | open | P0 AppState composition root |
| #52 | `feat/p1-async-dispatcher` | `feat/p0-app-state-composition-root` | open | P1 Dispatcher and AppState dispatch |
| #53 | `feat/p2-service-loop-cost-ledger` | `feat/p1-async-dispatcher` | open | P2 ServiceLoop, persistence, cost ledger |
| #54 | `feat/p3-relay-lifecycle` | `feat/p2-service-loop-cost-ledger` | open | P3 relay and lifecycle |
| #55 | `feat/p4-transports` | `feat/p3-relay-lifecycle` | open | P4 TCP, HTTP/SSE, UDS transports |
| #56 | `feat/p5-providers-engines` | `feat/p4-transports` | open | P5 fs `BlobEngine` + wasm UDF + adapter pattern |
| #57 | `feat/p6-real-binary-wiring` | `feat/p5-providers-engines` | open | P6 real `tdw-service` daemon + `tdw-cli` client |
| #58 | `feat/p7-e2e-real-backends` | `feat/p6-real-binary-wiring` | open | P7 real-backend e2e integration test |
| #59 | `feat/p8-quality-gate` | `feat/p7-e2e-real-backends` | open | P8 quality gate audit (this document) |

P5's UDF-adjacent code changes have been pushed and reviewed as part of #56 — `tdw-sandbox` gains an optional `udf-wasm` feature that routes `UdfRuntime::Wasm` through `tdw-udf-wasm`, `tdw-service-api` gains optional `storage-fs` and `udf-wasm` feature wiring, and `tdw-udf-wasm` ships a deterministic fixture runtime that validates WASM magic bytes and export dispatch.

## CI state

All stack PRs checked so far have the same blocking signal:

- `Lint, Schema, and Audit`: failing.
- Cause observed from PR #54 log: `cargo fmt --all -- --check` reports formatting diffs in:
  - `crates/tdw-app-server/src/lib.rs`
  - `crates/tdw-service-api/src/dispatcher.rs`
  - `crates/tdw-service-api/src/event_sink.rs`
- Other checks are passing or still running depending on PR recency:
  - Unit Linux/Windows
  - Coverage
  - Integration/Property/E2E subset
  - Windows Release
  - CodeQL
  - Analyze

## Governance findings

- The active checkout has been used for the full P0–P8 stack, but `AGENTS.md` says non-trivial work should happen in sibling worktrees. Continued implementation should move to a dedicated sibling worktree, or restore the primary checkout to `main` after preserving current changes.
- The old `.omx/ultragoal` run was complete and has been replaced for the daemon-hardening continuation. The old run remains available in git history.
- Main does not contain P0–P8 yet, so audits against `main` will look stale unless they explicitly include the branch stack.
- Do not merge or build later phases until the earlier stacked PRs are formatted and green, or intentionally collapse the stack into a fresh branch/PR with equivalent review evidence.

## Safe landing path

1. Preserve current `Cargo.lock` intent before changing branches.
2. Run `cargo fmt --all` on the branch stack, then rerun `cargo fmt --all -- --check`.
3. Push updated commits to the affected stack branches or collapse the stack into a new clean branch if stack maintenance becomes more expensive than review value.
4. Merge P0 → P8 in order after CI is green.
5. Continue follow-up Ultragoal stories only after the base branch is unambiguous.
