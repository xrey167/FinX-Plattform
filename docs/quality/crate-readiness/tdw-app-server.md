# tdw-app-server Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-app-server\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; tokio ^1.52.3 features=[macros,rt-multi-thread,sync]
- Dev dependencies: tdw-event; loom ^0.7 (under `[target.'cfg(loom)'.dependencies]`, loom build only)
- Reverse local dependencies: tdw-app-client
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: yes (`tests/loom_relay.rs`, loom-only model)
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 3 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- Added daemon endpoint validation for UDS and HTTP/SSE transports, rejecting empty, traversal, control-character, shell-control, and wrong-scheme addresses.
- Existing queue behavior is covered by `AgentLoop::run_once`, which consumes submitted protocol envelopes and emits `EventMsg::Started`.
- Scan signals are test `expect` calls; no stub, copied FinX-XR, OpenBB, or duplicate service implementation was found.

## Concurrency Model (loom, TEST-POLICY-003)

This crate hosts the workspace's first `loom` model. It targets the in-memory
outbox->bus relay (`spawn_inmemory_relay`), which is the most non-trivial
in-process lock interaction in the workspace: a check-then-act across three
separate lock acquisitions per record (read pending under the outbox lock,
publish under the bus lock, mark dispatched under the outbox lock), with the
outbox lock released between the read and the mark.

- Model file: `tests/loom_relay.rs`, gated behind `#![cfg(loom)]`. It never
  compiles into the default build, default `cargo test`, or the release profile.
- Dependency: `loom = "0.7"` under `[target.'cfg(loom)'.dependencies]` only.
- Invariant proven: with a relay drain cycle racing a concurrent producer
  append (two threads, bounded budget), every record the relay observes as
  pending is published to the bus exactly once (no double-publish) and marked
  `Dispatched` (no lost update where a shipped record stays `Pending`); the
  dispatched count equals the bus entry count (outbox/bus stay consistent). The
  model is falsifiability-checked: dropping a `mark_dispatched` makes loom find
  the violating interleaving and fail.
- Pivot note: ADR 0014 originally named `tdw-bus` as the first loom target;
  `tdw_bus::EventBus` is single-threaded (`&mut self`, no atomics/interior
  mutability), so it was non-modelable. See the ADR 0014 amendment (2026-05-29).
- Run command (from repo root):

  ```powershell
  $env:RUSTFLAGS = "--cfg loom"
  cargo test -p tdw-app-server --test loom_relay
  Remove-Item Env:\RUSTFLAGS
  ```

  If loom reports a too-large branch count, set `$env:LOOM_MAX_PREEMPTIONS = "2"`.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.
- Loom model passed under `--cfg loom` (`cargo test -p tdw-app-server --test loom_relay`); default `cargo test -p tdw-app-server` stays loom-free (the model reports `running 0 tests`).

## Verdict

Ready with follow-ups. The daemon sample has typed endpoint contracts and queue-event tests; durable production queueing remains a future service concern.
