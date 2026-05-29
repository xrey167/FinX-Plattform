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

## Amendment (2026-05-29): First loom target pivots from `tdw-bus` to `tdw-app-server` relay

The Decision named `tdw-bus` as the first loom target. Implementation of
`TEST-POLICY-003` showed that this would be near-tautological: the in-memory
`tdw_bus::EventBus` is single-threaded by construction. Every mutator takes
`&mut self`, there is no interior mutability, no atomics, and no `Arc<Mutex<_>>`
wrapping inside the type itself, so loom (which only instruments shared-memory
concurrency primitives) would have no interleavings to explore. A model there
would assert ordering that the compiler's `&mut` exclusivity already guarantees.

The first loom model therefore targets the genuinely concurrent in-process
component with the most non-trivial lock interaction: the in-memory outbox to
bus relay in `tdw_app_server::spawn_inmemory_relay`. The relay performs a
check-then-act across three separate lock acquisitions per record (read pending
under the outbox lock, publish under the bus lock, mark dispatched under the
outbox lock), with the outbox lock released between the read and the mark. A
concurrent producer that appends to the outbox can interleave at those release
points, which is exactly the interleaving sensitivity loom exists to verify.

The bounded model (`crates/tdw-app-server/tests/loom_relay.rs`, two threads, one
relay drain cycle racing one producer append) proves: every record the relay
observes as pending is published to the bus exactly once (no double-publish) and
marked `Dispatched` (no lost update where a record stays `Pending` after being
shipped), and the dispatched count stays equal to the bus entry count (outbox
and bus remain consistent). The model was falsifiability-checked: deliberately
skipping a `mark_dispatched` makes loom find the violating interleaving and fail.

Run command (loom is gated behind `--cfg loom` and never enters the default
build or `cargo test`):

```powershell
$env:RUSTFLAGS = "--cfg loom"
cargo test -p tdw-app-server --test loom_relay
Remove-Item Env:\RUSTFLAGS
```

The order in the original Decision (`tdw-bus` first, then outbox/relay and
daemon cancellation) is superseded: outbox/relay is the first model; `tdw-bus`
is dropped from the loom queue as a non-candidate; daemon cancellation remains a
deferred follow-on if a bounded model design emerges. Status remains Accepted.

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
