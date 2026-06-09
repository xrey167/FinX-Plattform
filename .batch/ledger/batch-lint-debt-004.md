---
batch: batch-lint-debt-004
items: lint:clippy::doc_markdown
outcome: done
---

# batch-lint-debt-004 — fix clippy::doc_markdown

Fixed all `clippy::doc_markdown` pedantic warnings in `tdw-domain`, `tdw-functions`,
`tdw-functions-app` by backticking code-like tokens in doc comments per clippy's exact
suggestions. No prose reworded; no blanket `#[allow]`.

## Warnings fixed per crate (5 total)
- `tdw-domain` (3): `src/envelope.rs:6` `OBBject`; `src/envelope.rs:8` `FinX`; `src/models.rs:38` `snake_case`
- `tdw-functions` (1): `src/event_wiring.rs:40` `BTreeMap`
- `tdw-functions-app` (1): `src/lib.rs:3` `FinX`

## Gates

### cargo clippy -p tdw-domain -p tdw-functions -p tdw-functions-app --all-targets -- -W clippy::doc_markdown
PASS — 0 doc_markdown warnings (was 5). Warm cache busted via `cargo clean -p ...` first.

### cargo clippy -p tdw-domain -p tdw-functions -p tdw-functions-app --all-targets -- -W clippy::pedantic -W clippy::nursery
PASS (ratchet) — doc_markdown count = 0; no new pedantic/nursery warnings introduced on touched crates.

### cargo fmt -p {tdw-domain,tdw-functions,tdw-functions-app} -- --check
PASS — fmt-domain=0, fmt-functions=0, fmt-functions-app=0

### cargo clippy --workspace --all-targets -- -D warnings
PASS — `Finished dev profile ... in 37.64s`; CLIPPY_EXIT=0

### cargo test --workspace
PASS (with documented flaky-test caveat). Two runs:
- `cargo test -p tdw-domain -p tdw-functions -p tdw-functions-app` → EXIT=0, all touched-crate tests pass (0 failed).
- `cargo test --workspace --exclude tdw-backend` → EXIT=0, 1582 tests passed, 0 failed / 0 errors / 0 panics.

`cargo test --workspace` (unscoped) HUNG on a `tdw-backend` S3/network integration
test (executable alive ~43 min at ~0.2s CPU = I/O wait, not compute). This is the
flaky real-S3 Integration/E2E test documented in project memory ("FLAKY (rerun,
non-required)") — environmental, in a crate NOT touched by this batch, and a
doc-comment-only change cannot alter test runtime behavior. `tdw-backend` still
compiled clean under the workspace clippy `-D warnings` gate above.

### cargo run -p xtask -- clean-room-audit
PASS — `clean-room audit passed`; AUDIT_EXIT=0

## PR
(to be appended after creation)

## Notes
- `FinX` is brand prose but clippy treats it as a CamelCase identifier; applied its
  suggested backticking exactly (task spec: match each suggestion exactly).
- Only doc-comment edits; no logic changes. `.batch/backlog.json` untouched.
