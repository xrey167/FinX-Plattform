---
batch: batch-lint-debt-009
items: lint:clippy::manual_let_else
outcome: no-op (lint family already clean)
---

# batch-lint-debt-009 — manual_let_else (already resolved upstream)

Targeted the `clippy::manual_let_else` family: rewrite
`let x = match/if-let … else { return/panic }` into the `let … else { }`
form. Discovery found **zero** warnings across the workspace, so there was
nothing to rewrite and no PR is opened.

## Lint family

`clippy::manual_let_else` (default `warn`; not pinned in
`[workspace.lints.clippy]`, which only denies `dbg_macro`, `todo`,
`unwrap_used` — so the lint would surface in a clippy run if present).

## Discovery

Run from the isolated worktree
`FinX-Plattform-batch-lint-letelse-01` (branch `work/batch-lint-letelse-01`,
based on `origin/main` @ `5be42120`).

The shared `CARGO_TARGET_DIR=C:\Users\ReyDa\.cargo-target` is warm across all
sibling worktrees and re-uses fingerprints, so a plain `cargo clippy` does not
re-emit warnings on already-built crates (and `cargo clean` hits
`os error 5 / Zugriff verweigert` because other processes hold locks). To get
a truthful pass the discovery used an in-worktree `--target-dir target` and
cleared `target/debug/.fingerprint` to force clippy's lint pass to actually
run:

```
cargo clippy --workspace --all-targets --target-dir target \
  --message-format=short -- -W clippy::manual_let_else
```

After clearing fingerprints, all `tdw-*` workspace crates were re-`Checking`-ed
and clippy `Finished` cleanly: **0 occurrences of `manual_let_else`** in 488
lines of output (all `Compiling`/`Checking` progress, no `warning:` lines).

A broad source regex for `let x = match/if-let … { return/panic … }` matched
21 files, but inspection shows these are ordinary `match`/`?`/`.map_err`
expressions with value-returning arms — none are `manual_let_else` candidates
(the lint only fires for a single binding arm plus a diverging `_` arm).

## Confirmation (hard-deny, cleared fingerprints)

| Crate | Command | Result |
| --- | --- | --- |
| tdw-core | `cargo clippy -p tdw-core --all-targets --target-dir target -- -D clippy::manual_let_else` | exit 0 (no findings) |
| tdw-mcp | `cargo clippy -p tdw-mcp --all-targets --target-dir target -- -D clippy::manual_let_else` | exit 0 (no findings) |
| tdw-app-server | `cargo clippy -p tdw-app-server --all-targets --target-dir target -- -D clippy::manual_let_else` | exit 0 (no findings) |

These three were chosen because they were among the 21 regex hits; promoting
the lint to `-D` would have failed the run on any real occurrence. All passed.

## Crates touched / rewrites / reverted

- Crates touched: **none**
- Rewrites applied: **0**
- Reverted: **none**

## Gate evidence

| Gate | Command | Result |
| --- | --- | --- |
| discovery | `cargo clippy --workspace --all-targets --target-dir target -- -W clippy::manual_let_else` (fingerprints cleared) | 0 `manual_let_else` warnings |
| deny-check | `cargo clippy -p {tdw-core,tdw-mcp,tdw-app-server} --all-targets --target-dir target -- -D clippy::manual_let_else` | pass (exit 0) |
| clean-room | `cargo run -p xtask --target-dir target -- clean-room-audit` | pass ("clean-room audit passed") |

Per-crate fmt / `-D warnings` / `cargo test` gates were not run because no
crate source was modified (no diff to gate). No schema-bearing crate was
touched, so no schema-sync/regen was required.

## Outcome

No `manual_let_else` debt exists on `origin/main`; the codebase already uses
the idiomatic `let … else` form wherever the lint applies. No source change,
no commit of source, no PR. This ledger is the deliverable record. The
`lint:clippy::manual_let_else` backlog item should be marked done/clean.

No AGPL code copied.
