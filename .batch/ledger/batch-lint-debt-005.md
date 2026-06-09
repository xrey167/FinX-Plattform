---
batch: batch-lint-debt-005
items: lint:clippy::match_same_arms
outcome: done
---

# batch-lint-debt-005 — clippy::match_same_arms in tdw-acp

> Note: the `batch-lint-debt-005` id was previously used for a `missing_const_for_fn`
> remainder ledger (merged via PR #160). This branch/run reuses the id per the assigned
> task spec to fix `clippy::match_same_arms` in `tdw-acp`; this file is overwritten
> accordingly.

## Summary
Fixed all `clippy::match_same_arms` warnings in `tdw-acp` (2 distinct source warnings,
reported as 4 with lib + lib-test build duplicates). Both fixed by merging identical
match arms with `|` patterns in `crates/tdw-acp/src/lib.rs` (the `validate` op-dispatch
match). No `#[allow]` was needed — both merges preserve identical behavior and
readability.

- `Op::DeleteAlert { id } | Op::SetAlertActive { id, .. } => validate_token("id", id)`
  (merged two arms with the same `validate_token("id", id)` body)
- `Op::ListAlerts {} | Op::Cancel { .. } | Op::Shutdown => Ok(())`
  (folded the `ListAlerts` no-op arm into the existing `Cancel | Shutdown => Ok(())` arm)

## Gate results

### cargo clean -p tdw-acp
Removed cached artifacts (warm cache busted before fmt/clippy).

### cargo fmt -p tdw-acp -- --check
FMT_EXIT=0 (pass)

### cargo clippy --workspace --all-targets -- -D warnings
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 21.69s
CLIPPY_EXIT=0
```
pass

### cargo test --workspace
```
DONE_TEST_EXIT=0
```
pass — 210 `test result: ok` lines, 0 failed / 0 panicked / 0 errors across the whole
workspace (lib + integration + doctests).

Note: under the detached background runner, stdin is an open pipe (never EOF), which
hangs the `tdw-backend` `run_both_surface_wires_daemon_and_runs_mcp_loop_to_completion`
test (its embedded stdio MCP loop blocks on a stdin read that the test relies on being
at EOF). Re-ran the gate with stdin redirected from `NUL` (`cargo test --workspace < NUL`)
so EOF is immediate; that test then passed along with the full suite. Pure environment
artifact — unrelated to the `tdw-acp` match-arm change.

### cargo run -p xtask -- clean-room-audit
```
clean-room audit passed
AUDIT_EXIT=0
```
pass

### Ratchet: cargo clippy -p tdw-acp --all-targets -- -W clippy::pedantic -W clippy::nursery
pedantic/nursery warning count in tdw-acp: 0 (no new warnings introduced)

### Confirm match_same_arms == 0 in tdw-acp
```
cargo clippy -p tdw-acp --all-targets -- -W clippy::match_same_arms
"match arms have identical" count: 0
```
pass

## Diff scope (git diff --name-only origin/main...HEAD)
Only the ledger + `crates/tdw-acp/src/lib.rs`; `.batch/backlog.json` untouched.

## Notes
- 2 match_same_arms warnings fixed, both by `|`-merge. No allow added.
- tdw-acp is not schema-bearing; no xtask schema-sync run.

## PR
https://github.com/xrey167/FinX-Plattform/pull/259

Pushed as `work/batch-lint-debt-005-match-same-arms` (the `work/batch-lint-debt-005`
remote ref already exists from the merged `missing_const_for_fn` PR #160 — batch-id
reuse). Avoided a force-push per the stop conditions.
