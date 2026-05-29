# Test Policy Backlog

Date: 2026-05-28

Source: `.plans/2026-05-21-test-strategy.md` open questions O24, O25, and O26.

Decision record: `docs/adr/0014-test-policy-backlog.md`.

This file separates enforced gates from deferred policy work. A task listed
here is not a phase-exit blocker until it is wired into `xtask/src/main.rs`,
`docs/quality/phase-exit-gates.json` is regenerated, and CI runs it.

## Decisions

| Backlog item | Decision | Current enforcement | Deferred task |
| --- | --- | --- | --- |
| O24 mutation cadence | Do not require mutation testing on every PR for v0.1. Keep nightly canary coverage and run scoped changed-crate mutation before risky merges or release candidates. | `.github/workflows/nightly.yml` `mutation-smoke` runs `cargo mutants` over `tdw-core`, `tdw-protocol`, `tdw-app-client`, `tdw-mcp`, and `tdw-worker`, aggregates `cargo run -p xtask -- mutation report` into a `mutation-summary` artifact, and exposes `cargo run -p xtask -- mutation changed`; `phase-exit-gates.json` lists `mutation-smoke` and `mutation-summary` as nightly, not phase-exit. Score floors remain unenforced. | `TEST-POLICY-001`, `TEST-POLICY-002` (report-only; score-floor enforcement deferred) |
| O25 loom expansion | Do not add broad loom coverage for v0.1. The first bounded model is the `tdw-app-server` outbox->bus relay (pivoted from `tdw-bus`, which is single-threaded by construction; see ADR 0014 amendment 2026-05-29). Expand only where a compact state machine exists. | First model committed in `tdw-app-server` (`--cfg loom`, not a PR gate yet). | `TEST-POLICY-003` (implemented) |
| O26 fuzz target list | Start with parser and wire-format boundaries: protocol JSON, daemon frames/SSE, MCP JSON-RPC/HTTP, config TOML, and SQL guard parsing. | No fuzz gate yet. | `TEST-POLICY-004`, `TEST-POLICY-005` |

## Deferred Tasks

### TEST-POLICY-001: Baseline Mutation Score Reporting

Status: implemented (report-only; awaiting seven-run baseline before score
floors are considered).

- Scope: `tdw-core`, `tdw-protocol`, `tdw-app-client`, `tdw-mcp`, and
  `tdw-worker`.
- Implementation: the nightly `mutation-smoke` job runs `cargo mutants` for each
  scoped crate into `mutants.out/<crate>` (each step is `continue-on-error` so a
  survivor never fails the run yet), then `cargo run -p xtask -- mutation report
  mutants.out` aggregates every per-crate `outcomes.json` into a single
  `mutation-summary.json`. Both the summary and the raw `outcomes.json` files are
  uploaded as the `mutation-summary` CI artifact via `actions/upload-artifact@v4`.
  A `mutation-summary` quality gate (nightly, not phase-exit) is recorded in
  `phase-exit-gates.json`.
- Summary contents: per crate `runtimeSecs`, `total`, `killed`, `survivors`,
  `timeouts`, and `other`; top-level `scoredFloorEnforced: false`.
- Acceptance: CI artifact includes runtime, killed mutants, survivors, and
  timeout count per crate for at least seven consecutive nightly runs.
- Deferred: score-floor enforcement stays off until the seven-run baseline
  exists.

### TEST-POLICY-002: Promote Scoped Mutation Gates

Status: command implemented (still outside PR phase-exit gates per ADR 0014).

- Scope: crates changed in a release candidate plus foundational crates touched
  by protocol, daemon, storage, or worker changes.
- Implementation: `cargo run -p xtask -- mutation changed` (also `just
  mutation-changed`) computes crates changed vs `origin/main` from `git diff
  --name-only origin/main...HEAD`, unions them with the baseline set
  (`tdw-core`, `tdw-protocol`, `tdw-app-client`, `tdw-mcp`, `tdw-worker`,
  `tdw-storage-router`), and prints the scoped `cargo mutants -p <crate>`
  invocation plan. It is plan-only by default (always exits `Ok`, no dependency
  on `git` or `cargo-mutants`), so it stays offline-friendly and deterministic.
  Pass `--run` (`cargo run -p xtask -- mutation changed --run`) to execute the
  sweep; that path requires `cargo-mutants` and returns a non-zero exit on
  unclassified survivors. Skip / `MUTANT-EQUIV` survivors are honored through
  cargo-mutants' in-source `// mutants::skip` / `cargo-mutants: skip` markers.
- Acceptance: the command fails on unclassified survivors, supports
  `MUTANT-EQUIV`/skip annotations, and remains outside normal PR phase-exit
  gates until runtime is predictable.
- Deferred: promotion to a PR phase-exit gate stays out until runtime is
  predictable from the TEST-POLICY-001 baseline.

### TEST-POLICY-003: Add First Loom Model

Status: implemented.

- Component: the in-memory outbox->bus relay in
  `tdw_app_server::spawn_inmemory_relay`. This pivots from the `tdw-bus`
  candidate named in ADR 0014: `tdw_bus::EventBus` is single-threaded by
  construction (all mutators take `&mut self`, no interior mutability, no
  atomics), so a loom model there would be tautological. See the ADR 0014
  amendment dated 2026-05-29 for the rationale. The relay is genuinely
  concurrent: it performs a check-then-act across three separate lock
  acquisitions per record (outbox read, bus publish, outbox mark-dispatched),
  with the outbox lock released between read and mark.
- Invariant proven: with a relay drain cycle racing a concurrent producer
  append, every record the relay observes as pending is published to the bus
  exactly once (no double-publish) and marked `Dispatched` (no lost update where
  a shipped record stays `Pending`); the dispatched count equals the bus entry
  count (outbox/bus stay consistent). Falsifiability-checked: removing a
  `mark_dispatched` makes loom find the violating interleaving and fail.
- Implementation: `loom = "0.7"` is a `[target.'cfg(loom)'.dependencies]`
  dev-only dependency in `crates/tdw-app-server/Cargo.toml`, so it never enters
  the default build, default `cargo test`, or the release profile. The model
  lives in `crates/tdw-app-server/tests/loom_relay.rs` behind `#![cfg(loom)]`
  (two threads, one drain cycle racing one append, bounded permutation budget).
  The `unexpected_cfgs` workspace lint declares `check-cfg = ['cfg(loom)']`.
- Run command (from repo root):

  ```powershell
  $env:RUSTFLAGS = "--cfg loom"
  cargo test -p tdw-app-server --test loom_relay
  Remove-Item Env:\RUSTFLAGS
  ```

  If loom reports a too-large branch count, set `$env:LOOM_MAX_PREEMPTIONS = "2"`.
- Acceptance: met. The model proves the no-lost-update / no-double-publish
  invariant with a bounded permutation budget and is documented in
  `docs/quality/crate-readiness/tdw-app-server.md`. PR gating remains deferred
  per ADR 0014 until the first model is stable across nightly budgets.

### TEST-POLICY-004: Add Initial Fuzz Harnesses

Status: implemented (both halves). The stable corpus-replay harnesses run in
default `cargo test`; the nightly cargo-fuzz layout (`fuzz/` crate + `fuzz-smoke`
nightly CI job) reuses the same shims under `cargo +nightly fuzz`.

- Stable implementation (this PR): each scoped crate exposes a
  `#[doc(hidden)] pub fn __fuzz_<name>(&[u8])` shim that runs the real parse and
  discards the `Result`, asserting graceful error handling instead of panics.
  A per-crate `tests/fuzz_replay.rs` loads every committed seed under
  `tests/corpus/<name>/` and replays it through the shim, asserting no panic.
  Seed corpora include valid, malformed, empty, boundary/oversized, and
  non-UTF8 inputs. The six surfaces and shims are:
  - `tdw-protocol::__fuzz_protocol_json` (`protocol_json` corpus) — `OpEnvelope`,
    `EventMsg`, `ReplayFrame` JSON.
  - `tdw-config::__fuzz_config_toml` (`config_toml` corpus) — `ConfigLayer::from_toml`.
  - `tdw-mcp::__fuzz_mcp_jsonrpc` (`mcp_jsonrpc` corpus) — `handle_json_rpc_line`.
  - `tdw-mcp::__fuzz_mcp_http` (`mcp_http` corpus) — Streamable HTTP request body.
  - `tdw-app-client::__fuzz_daemon_frame` (`daemon_frame` corpus) — length-delimited
    daemon event frame reader.
  - `tdw-exec::__fuzz_sql_guard` (`sql_guard` corpus) — read-only SQL guard.
  The shims are designed so a future nightly cargo-fuzz target (the follow-up
  half of this task) can call the same entry points.
- Nightly implementation (follow-up PR): the `fuzz/` cargo-fuzz crate
  (`tdw-fuzz`, `publish = false`, `[package.metadata] cargo-fuzz = true`) defines
  one `libfuzzer-sys` target per surface that reuses these `__fuzz_*` shims:
  `protocol_json`, `config_toml`, `mcp_jsonrpc`, `mcp_http`, `daemon_frame`, and
  `sql_guard`. The crate is excluded from the stable workspace
  (`[workspace] exclude = ["fuzz"]`) so the default build and `cargo test` never
  pull in a nightly toolchain. Small seed corpora live under
  `fuzz/corpus/<target>/`; runtime crash reproducers and the generated corpus are
  git-ignored. Build/run a single target with `cargo +nightly fuzz run <target>`
  from the `fuzz/` directory. The `fuzz-smoke` job in
  `.github/workflows/nightly.yml` installs nightly + cargo-fuzz and runs each
  target for a bounded smoke budget (`-runs=10000 -max_total_time=30`), uploading
  `fuzz/artifacts/**` on failure. It is non-blocking and is not a required PR
  check (not wired into `ci.yml`).

- Scope:
  - `protocol_op_event_json` for `tdw-protocol` `OpEnvelope`, `EventMsg`, and
    replay frame JSON.
  - `app_client_daemon_frame` for daemon length-delimited event frames and
    HTTP/SSE event payloads.
  - `mcp_streamable_http_request` for MCP JSON-RPC and Streamable HTTP request
    parsing.
  - `config_layer_toml` for config TOML layer parsing and merge boundaries.
  - `exec_sql_guard` for SQL guard parsing of multi-statement and unsafe-token
    inputs.
- Implementation: create a `fuzz/` workspace or equivalent cargo-fuzz layout
  with committed seed corpora.
- Acceptance: each harness runs for a short smoke budget locally and records
  crash reproducers as artifacts instead of panicking in default `cargo test`.

### TEST-POLICY-005: Add Pre-Release Fuzz And Loom Recipe

- Scope: release candidates only.
- Implementation: add a documented pre-release command that runs the fuzz
  smoke targets and any stable loom models; keep long-duration fuzzing as a
  scheduled or manually triggered CI workflow.
- Acceptance: `docs/release.md` names the command and the expected artifacts;
  release readiness cannot claim fuzz/loom evidence without the command output.
