# tdw-snapshot Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-snapshot\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies are tdw-service-api.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 1 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- Snapshot store assigns per-table monotonic versions and supports latest and as-of-version lookup.
- No code change required in G002; existing test covers time-travel lookup and latest-version selection.
- Follow-up boundary: Persistence, pruning, and large-table manifests belong to storage tranches.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.


## Production Backend Evidence (G013)

`PgSnapshotStore` (gated by `--features postgres`) lives in
`crates/tdw-snapshot/src/pg_snapshot.rs`. Wraps
`tdw_storage_postgres::PgEngine` (G010).

Schema:

```sql
CREATE TABLE tdw_snapshot (
    id          BIGSERIAL PRIMARY KEY,
    table_name  TEXT NOT NULL,
    version     BIGINT NOT NULL,
    created_at  TEXT NOT NULL,
    row_ids     JSONB NOT NULL,
    UNIQUE (table_name, version)
);
```

Public surface:
- `PgSnapshotStore::new(engine)` / `with_table(name)`.
- `ensure_schema()` — idempotent CREATE TABLE.
- `commit(table, created_at, row_ids)` — computes the next version
  server-side as `COALESCE(MAX(version), 0) + 1` for the table via
  CTE-wrapped INSERT ... SELECT ... RETURNING. The `(table_name,
  version)` UNIQUE constraint serialises concurrent writers; under
  contention the loser of a race surfaces as `Error::Storage` and
  callers may retry.
- `as_of_version(table, version)` / `latest(table)` — both return
  `Option<Snapshot>`.

Per-table version numbering matches the in-memory `SnapshotStore`
(starts at 1, monotonic).

Integration test at `crates/tdw-snapshot/tests/pg_snapshot.rs` is
double-gated by `--features postgres` + `TDW_POSTGRES_TEST_URL`.
Hermetic per-process table; exercises commit/latest/as_of_version
across two tables to verify per-table versioning.

See `docs/quality/production-transport-status.md` for the broader
G013 punch list.
