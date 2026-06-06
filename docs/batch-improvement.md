# Batch Improvement Mode

A repeatable harness that improves the workspace via batch runs: deterministic
discovery builds a ranked backlog; agentic execution lands each batch as
worktree -> fixes -> gates -> PR. Machines enumerate, agents judge.

- Scanner: `cargo run -p xtask -- improve-scan` (source: `xtask/src/improve_scan.rs`)
- Execution loop: `/batch` command (`.claude/commands/batch.md`)
- State: `.batch/` (git-tracked)

## State layout and file ownership

```text
.batch/
  backlog.json                 # SINGLE-WRITER: only `improve-scan` (via /batch discover) writes it
  ledger/
    batch-<bucket>-<NNN>.md    # one file PER batch; written only by that batch's branch
```

The ownership split is the conflict-freedom guarantee: a batch branch adds its
own ledger file and never touches `backlog.json`, so two batches from different
sessions cannot collide on `.batch/` files. Mechanical check before every batch
PR: `git diff --name-only main...HEAD` must list the ledger file and must NOT
list `.batch/backlog.json`.

`backlog.json` is the only writable status surface (there is deliberately no
generated markdown mirror); `/batch status` renders a human view on demand.

## Buckets

| Bucket | What it measures | Item id scheme |
| --- | --- | --- |
| provider-wiring | `crates/tdw-provider-*` dirs vs `provider-*` feature keys in `crates/tdw-service-api/Cargo.toml`, normalized | `provider:<name>` |
| lint-debt | clippy pedantic/nursery warnings (JSON), grouped per lint family | `lint:<clippy-code>` |
| test-gaps | crates with no `tests/` dir and no `#[test]`/`#[tokio::test]`, ranked by workspace reverse-deps | `test-gap:<crate>` |
| hygiene | `cargo deny check advisories` failures, TODO/FIXME counts | `hygiene:<topic>` |

Ids are deterministic and stable across runs — they are the merge key. The
fixture tests in `xtask/src/improve_scan.rs` pin the scheme; changing it breaks
status preservation, so treat it as a contract.

### Provider-wiring normalization

A naive set-difference produces false positives. The scanner therefore applies:

- A dir `tdw-provider-<name>` counts as **wired** if the feature key
  `provider-<name>` OR `provider-<name>-http` exists (`binance` is wired via
  `provider-binance-http`).
- Unwired dirs whose own manifest has an `http` feature are `unwired-http` —
  the actionable target (e.g. `yahoo` at the time of writing).
- Unwired dirs without an `http` feature are `needs-design` (e.g. `fileset`,
  `ws`) — they need a wiring design, not the standard fetcher pattern.
- `ws-mock` (test double) is excluded entirely.

## Merge semantics (status lifecycle)

Statuses: `pending` -> `in-review` -> `done` | `blocked`, plus scanner-managed
`resolved` and `needs-design`.

- `blocked` / `in-review` / `needs-design` survive re-scans; evidence refreshes
  in place.
- **Reappearing evidence reopens items**: a `done`/`resolved` item that is
  re-detected returns to `pending` with a `reopenedFrom` marker. Evidence wins
  over status — regressions never stay silently `done`.
- Vanished evidence marks an item `resolved`; `resolved` items are pruned on
  the **next** run (one cycle of visibility). Ledger files are the permanent
  history; an item that reappears after pruning is a fresh item with no
  lineage.
- A bucket that was skipped this run (tool unavailable, compile error) never
  resolves its items — no evidence signal is not vanished evidence.
- Ledger fold-back: the scanner parses `.batch/ledger/*.md` front-matter
  (`items:` + `outcome: done|blocked`) and applies those outcomes before
  merging, so `/batch discover` is the single point where batch results enter
  the backlog.

## Idempotency and toolchain drift

Re-running `improve-scan` on the same toolchain with unchanged debt writes
nothing (the `generatedAt` stamp survives). The pedantic/nursery warning set is
owned by the rustc/clippy version, so a toolchain bump legitimately produces a
large backlog diff — the `toolchain` header stamp identifies the cause. That
diff is expected maintenance, not rot; it lands via the reviewed discover PR.

## CI cost

`ci.yml` has no `paths:` filter: a discover PR pays the full pipeline (~30+
minutes including image builds and the Docker integration job). Run discover
**per wave** — once before starting a bucket's batches — not per batch. A
`paths-ignore` exemption for `.batch/`-only diffs is a possible follow-up but
interacts with the required-checks merge gating, so it needs its own PR.

## Boundary vs `quality-gate`

`xtask quality-gate` is a static declaration of which gates exist (a manifest,
spawns nothing). `improve-scan` is a dynamic measurement of current debt. They
are orthogonal; do not merge them: one defines "what must pass", the other
measures "what should improve".

`improve-scan` is NOT a gate and is not part of the quality-gate manifest.

## Adding a bucket

1. Add a scan function in `xtask/src/improve_scan.rs` returning `Vec<Item>`
   with a new id prefix; degrade-don't-fail on tool errors.
2. Add the bucket name to `BUCKET_ORDER` (determines ranking section order).
3. Pin the id scheme with a fixture test.
4. Add sizing caps + execution guidance to `.claude/commands/batch.md`.
5. Document the bucket in the table above.

## Operational hazards (encoded in `/batch`, repeated here)

- This host sets a global `CARGO_TARGET_DIR`, so even fresh worktrees share a
  warm cache; `cargo clean -p <touched-crate>` before the clippy gate.
- Commit + push immediately after the first green gate (parallel sessions have
  wiped uncommitted sibling worktrees).
- Hard cap of 2 live `work/batch-*` worktrees (multi-GB `target/` each).
- clean-room-audit scans `Cargo.toml` + `crates/**` only — it does NOT cover
  `.batch/`; never record fetched external content in evidence blobs.
