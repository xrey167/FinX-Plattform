# tdw-outbox Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-outbox\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-event
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-cdc, tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are tdw-event; reverse dependencies are tdw-cdc, tdw-service-api.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 1 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- In-memory outbox records pending/dispatched state and pending-after recovery; dispatch marking now reports misses.
- Changed mark_dispatched to return bool and added missing-sequence regression coverage.
- Follow-up boundary: Persistent outbox implementation remains a later storage/runtime concern.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.


## Production Backend Evidence (G013)

`PgOutboxStore` (gated by `--features postgres`) lives in
`crates/tdw-outbox/src/pg_outbox.rs`. Wraps
`tdw_storage_postgres::PgEngine` (G010) — no direct sqlx use in
this crate.

Schema:

```sql
CREATE TABLE tdw_outbox (
    sequence       BIGSERIAL PRIMARY KEY,
    envelope_json  JSONB NOT NULL,
    status         TEXT NOT NULL DEFAULT 'Pending',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Public surface:
- `PgOutboxStore::new(engine)` — wraps a `PgEngine` instance.
- `with_table(name)` — override default `tdw_outbox` table.
- `ensure_schema()` — idempotent `CREATE TABLE IF NOT EXISTS`.
- `append(envelope)` — uses CTE `WITH inserted AS (INSERT ...
  RETURNING sequence) SELECT sequence FROM inserted` so
  `PgEngine::fetch_json`'s row_to_json projection can see the
  RETURNING rows. Returns the auto-assigned sequence (BIGSERIAL).
- `mark_dispatched(sequence)` — `UPDATE ... WHERE sequence = $1 AND
  status = 'Pending'`; returns `true` only when a row transitioned.
- `pending_after(sequence)` — `SELECT ... WHERE sequence > $1 AND
  status = 'Pending' ORDER BY sequence ASC`, decodes
  `envelope_json::text` back into `EventEnvelope<Value>`.

The semantics match `InMemoryOutbox` exactly so callers can swap
backends.

Integration test at `crates/tdw-outbox/tests/pg_outbox.rs` is
double-gated by `--features postgres` + `TDW_POSTGRES_TEST_URL`.
Hermetic table per test process (`tdw_outbox_test_<pid>`) — drops
table before and after the run.

See `docs/quality/production-transport-status.md` for the broader
G013 punch list.
