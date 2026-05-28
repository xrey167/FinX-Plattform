# ADR 0014: Test Policy Backlog Decisions

Status: Accepted

Date: 2026-05-28

## Context

The bootstrap test strategy in `.plans/2026-05-21-test-strategy.md` left three
policy questions open:

- O24: whether mutation testing should run on every PR for changed crates.
- O25: when to introduce `loom` beyond the first concurrency model.
- O26: which parsers and wire formats should receive fuzz targets first.

The repository already has a nightly mutation canary and a generated
phase-exit gate contract. Default `cargo test` must remain offline,
deterministic, and short enough for normal development.

## Decision

Mutation testing is not a required PR gate for v0.1. Keep the nightly
`cargo-mutants` smoke as the only CI-enforced mutation signal until baseline
runtime and survivor data exist. Changed-crate mutation runs are recommended
before high-risk merges, and release candidates should run a scoped mutation
sweep over changed foundational, protocol, daemon, and storage crates. Mutation
score floors remain policy targets until `TEST-POLICY-001` and
`TEST-POLICY-002` in `docs/quality/test-policy-backlog.md` are closed.

`loom` will not be introduced broadly for v0.1. Add it only where the
concurrency contract is small enough to model without state-space blowups:
`tdw-bus` first, then outbox/relay and daemon cancellation paths only after a
bounded model design exists. PR gating applies only to crates with committed
loom models; broader nightly budgets wait until the first model is stable.

Fuzzing starts with wire-format and parser boundaries, not business logic. The
initial target list is:

- `tdw-protocol` JSON decode for `OpEnvelope`, `EventMsg`, and replay frames.
- `tdw-app-client` daemon length-delimited event frames and HTTP/SSE events.
- `tdw-mcp` JSON-RPC and Streamable HTTP request parsing.
- `tdw-config` TOML layer parsing and merge boundaries.
- `tdw-exec` SQL guard parsing for multi-statement and unsafe-token inputs.

Enforcement is intentionally deferred to the task ledger in
`docs/quality/test-policy-backlog.md`. Do not hand-edit
`docs/quality/phase-exit-gates.json`; add or promote gates through
`xtask/src/main.rs` and regenerate the JSON.

## Consequences

Default local and PR testing stays offline and deterministic. Heavy mutation,
loom, and fuzz work has explicit owners and acceptance criteria instead of
being implied by the broader quality strategy. Release readiness can still
mention these items, but it must distinguish currently enforced gates from
deferred policy tasks.

## Alternatives Considered

- Require mutation testing on every PR for changed crates. Rejected for v0.1
  because runtime and survivor-noise baselines do not exist yet.
- Add `loom` to all async/concurrent crates immediately. Rejected because the
  state space would be unbounded without per-crate model designs.
- Fuzz every parser-like boundary at once. Rejected in favor of a small
  wire-format-first target list with stable corpora.
