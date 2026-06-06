---
description: Batch improvement mode - run discover/run/status cycles against the .batch/ backlog
argument-hint: <discover|run|status> [bucket] [n]
allowed-tools: Bash, Read, Edit, Write, Glob, Grep, Task
---

You are driving the FinX-Plattform **batch improvement mode** defined in
`docs/batch-improvement.md`. That doc and `AGENTS.md` are authoritative; this
command is the execution loop. The deterministic scanner is
`cargo run -p xtask -- improve-scan`; you only execute what the backlog ranks.

Mode and arguments: **$ARGUMENTS** (mode = `$1`: one of `discover`, `run`, `status`).

Configuration (override only when the user says so):
- `MAX_BATCH_WORKTREES = 2` — hard cap on live `work/batch-*` worktrees
  (each worktree's `target/` is multi-GB on this 114-crate workspace; the host
  has a documented disk-crisis history; `new-worktree.ps1` has no count guard,
  so THIS loop enforces it).
- Batch sizing caps (one PR each): lint-debt = one lint family x <=10 crates;
  test-gaps = 1-3 related crates; provider-wiring = <=5 providers.

## Hard rails (apply to every mode)

- Never commit to `main`; never force-push; rebase onto `main`, don't merge it back.
- Never `git add -A` — stage only the files this batch touched.
- Commit + push immediately after the first green gate (parallel sessions have
  wiped uncommitted sibling worktrees before).
- Never reuse an old worktree for a new batch. CAUTION: this host sets a global
  `CARGO_TARGET_DIR` (`~/.cargo-target`), so even a fresh worktree shares a WARM
  cache — and warm caches have hidden clippy warnings before. Always
  `cargo clean -p <crate>` every touched crate before the clippy gate.
- `backlog.json` is SINGLE-WRITER: only `discover` mutates it. A batch worktree
  must never touch `.batch/backlog.json` (verify with
  `git diff --name-only main...HEAD` before opening the PR).
- Evidence blobs in `.batch/` must never contain fetched external content
  (clean-room-audit does not scan `.batch/`; the boundary is on you here).

## Mode: discover

1. Pre-flight: run on a clean checkout of latest `main` (or a fresh
   `chore/batch-scan-<yyyy-mm-dd>` worktree). CI cost notice: a discover PR pays
   the FULL pipeline (~30+ min incl. image builds) — run discover per WAVE
   (before starting a bucket's batches), not per batch.
2. Run `cargo run -p xtask -- improve-scan`. It folds completed-batch ledger
   outcomes (`.batch/ledger/*.md` front-matter) into the backlog automatically
   and preserves `blocked`/`in-review` statuses by id.
3. Review the `.batch/backlog.json` diff; summarize new/reopened/resolved items
   for the user. A large diff right after a toolchain bump is expected (the
   `toolchain` header stamp identifies it).
4. Commit the backlog diff on `chore/batch-scan-<date>`, push, open a PR
   (human-reviewed; not auto-merged).

## Mode: run  (`/batch run [bucket] [n=1]`)

1. **Pre-flight — refuse to start if any check fails:**
   - An OmX ultragoal is active on a competing goal (`.omx/ultragoal/goals.json`
     status not `complete`) or another self-improve harness owns the topic
     (check `.remember/` / project memory) → stand down and report.
   - The primary checkout is dirty → report, don't touch it.
   - `git worktree list` already shows >= MAX_BATCH_WORKTREES `batch-*` entries
     → refuse with the list; teardown merged ones first.
2. Select the next `n` ranked `pending` items of `$2` (bucket) from
   `.batch/backlog.json`. Skip `needs-design`, `blocked`, `in-review` items.
   Respect the batch sizing caps above — split rather than exceed.
3. Determine the next batch number `NNN` for the bucket from `.batch/ledger/`
   filenames, then create the worktree:
   ```powershell
   .\scripts\git\new-worktree.ps1 -Name batch-<bucket>-<NNN>
   ```
4. Execute the items in the worktree — delegate per item type:
   - lint-debt: fix the lint family across the listed crates; no blanket
     `#[allow]` without justification comments.
   - test-gaps: characterization/unit tests beside code or under `tests/`,
     following `docs/development-workflow.md` TDD guidance.
   - provider-wiring: follow the established pattern — provider crate `http`
     feature + `http_fetcher.rs`, `provider-<name>` feature key in
     `crates/tdw-service-api/Cargo.toml` wired into `default_registry()`, and
     membership in `all-http-providers` (see PR #140/#141 for the shape).
5. Gates (in the worktree; all four must be green). First bust the shared warm
   cache for every crate this batch touched, then run the gate:
   ```powershell
   cargo clean -p <each-touched-crate>
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo run -p xtask -- clean-room-audit
   ```
   If you touched `tdw-udf-wasm`, `tdw-sandbox`, or `tdw-service-api`, ALSO
   build the `wasmi`/`udf-wasm` feature combos (CI default matrix skips them).
6. Write the batch ledger file `.batch/ledger/batch-<bucket>-<NNN>.md` in the
   SAME branch. It must start with the fold-back front-matter:
   ```text
   ---
   batch: batch-<bucket>-<NNN>
   items: <comma-separated item ids>
   outcome: done
   ---
   ```
   followed by: gate evidence (the four commands + results), PR link (added
   after creation), and notes.
7. Commit (Conventional Commits, scoped adds), push, open the PR with the
   template body (Summary / Verification / Clean-Room Checklist). Confirm
   `git diff --name-only main...HEAD` lists the ledger file but NOT
   `.batch/backlog.json`.
8. **Stop conditions:** if a gate fails twice for an item after genuine fix
   attempts, set the ledger `outcome: blocked`, record the failing command +
   output as evidence, drop the item from the batch, and continue with the
   rest. Never bypass hooks or gates; never force-land.
9. After merge (CI green, branch up to date, squash-and-merge): teardown
   ```powershell
   .\scripts\git\remove-worktree.ps1 -Path ..\FinX-Plattform-batch-<bucket>-<NNN> -RemoveBranch
   ```
   The next `discover` run folds the ledger outcome into the backlog.

## Mode: status

Read-only: render a summary table of `.batch/backlog.json` (items by bucket and
status, top pending per bucket) and list `.batch/ledger/*.md` outcomes. Write
no files.

Report progress after each numbered step. A failed gate is a blocker, not a
footnote.
