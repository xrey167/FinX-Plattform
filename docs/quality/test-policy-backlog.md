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
| O24 mutation cadence | Do not require mutation testing on every PR for v0.1. Keep nightly canary coverage and run scoped changed-crate mutation before risky merges or release candidates. | `.github/workflows/nightly.yml` runs `cargo mutants -p tdw-core --features inventory-registration --timeout 120`; `phase-exit-gates.json` lists `mutation-smoke` as nightly, not phase-exit. | `TEST-POLICY-001`, `TEST-POLICY-002` |
| O25 loom expansion | Do not add broad loom coverage for v0.1. Start with a bounded `tdw-bus` model, then expand only where a compact state machine exists. | No loom gate yet. | `TEST-POLICY-003` |
| O26 fuzz target list | Start with parser and wire-format boundaries: protocol JSON, daemon frames/SSE, MCP JSON-RPC/HTTP, config TOML, and SQL guard parsing. | No fuzz gate yet. | `TEST-POLICY-004`, `TEST-POLICY-005` |

## Deferred Tasks

### TEST-POLICY-001: Baseline Mutation Score Reporting

- Scope: `tdw-core`, `tdw-protocol`, `tdw-app-client`, `tdw-mcp`, and
  `tdw-worker`.
- Implementation: extend the nightly mutation job or `xtask` to emit a
  machine-readable mutation summary without failing on score floors yet.
- Acceptance: CI artifact includes runtime, killed mutants, survivors, and
  timeout count per crate for at least seven consecutive nightly runs.

### TEST-POLICY-002: Promote Scoped Mutation Gates

- Scope: crates changed in a release candidate plus foundational crates touched
  by protocol, daemon, storage, or worker changes.
- Implementation: add an explicit command such as `just mutation-changed` or
  `cargo run -p xtask -- mutation changed` after the baseline report is stable.
- Acceptance: the command fails on unclassified survivors, supports
  `MUTANT-EQUIV`/skip annotations, and remains outside normal PR phase-exit
  gates until runtime is predictable.

### TEST-POLICY-003: Add First Loom Model

- Scope: `tdw-bus` first; outbox relay and daemon cancellation only after the
  first model lands without state-space blowups.
- Implementation: add loom as a dev-only dependency in the modeled crate and
  gate model tests behind an explicit feature or test target.
- Acceptance: the first model proves ordering or cancellation invariants with a
  bounded permutation budget and is documented in the crate readiness worksheet.

### TEST-POLICY-004: Add Initial Fuzz Harnesses

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
