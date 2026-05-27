# tdw-session Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-session\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core (optional), tdw-hooks, tdw-protocol, tdw-storage-postgres (optional)
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; sqlx ^0.9.0 features=[runtime-tokio,tls-rustls,sqlite,sqlite-bundled,macros]; thiserror ^2.0.18; tokio ^1.52.3 optional/dev features=[macros,rt-multi-thread,sync]
- Dev dependencies: tdw-bus, tdw-event, tdw-outbox, tdw-rollout, tdw-snapshot, tokio
- Reverse local dependencies: none
- Feature flags: postgres, g013-cross-store
- Test attributes detected: 4
- tests/ directory: yes
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 19 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are tdw-core (optional), tdw-hooks, tdw-protocol, tdw-storage-postgres (optional); reverse dependencies are none.
- [x] Feature flags reviewed: postgres and g013-cross-store keep production and cross-store verification paths opt-in.
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 4 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- SQLite session store migrates session, permission, approval, and cost-ledger tables with typed corruption errors.
- Added cost_entries read path and test coverage so cost ledger entries are auditable, not append-only.
- Follow-up boundary: Multi-connection durability and migration versioning can be expanded when runtime ownership is introduced.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.


## Production Backend Evidence (G013)

`PgSessionStore` (gated by `--features postgres`) at
`crates/tdw-session/src/pg_session.rs`. Wraps
`tdw_storage_postgres::PgEngine` (G010) and coexists with the
existing `SqliteSessionStore`.

This slice covers **core session CRUD only** (upsert + get).
Permission rules, approvals, and cost-ledger persistence remain
sqlite-only; the follow-up PRs add Postgres mirrors behind the
same feature flag.

Schema:

```sql
CREATE TABLE tdw_sessions (
    session_id  TEXT PRIMARY KEY,
    status      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
```

Public surface: `new(engine)`, `with_table(name)`,
`ensure_schema()`, `upsert_session(record)`, `get_session(id)`.

Upsert uses `ON CONFLICT (session_id) DO UPDATE SET status,
updated_at` — created_at is preserved across re-upserts to match
the SqliteSessionStore contract.

Integration test at `crates/tdw-session/tests/pg_session.rs`
double-gated by `--features postgres` + `TDW_POSTGRES_TEST_URL`.
Asserts upsert + get roundtrip, conflict-update preserves
created_at, and `get_session` returns `None` for unknown ids.

## Production Backend Evidence (G013, full surface)

`PgSessionStore` (gated by `--features postgres`) at
`crates/tdw-session/src/pg_session.rs` is now a **full mirror of
`SqliteSessionStore`**: sessions + permission rules + approvals +
cost ledger.

Public surface (all on `PgSessionStore`):
- `new(engine)` / `with_table(base)` / `ensure_schema()`
- `upsert_session(record)` / `get_session(id)`
- `save_permission_rules(id, rules)` / `load_permission_rules(id)`
- `request_approval(id, perm_id, action, pattern)` /
  `resolve_approval(perm_id, decision)` / `pending_approval(perm_id)`
- `append_cost(entry)` / `cost_entries(id)`

Schema (four tables, prefixed by the `with_table` base, default
`tdw_sessions`):
- `<base>` — sessions
- `<base>_permission_state` — rules JSON keyed by session
- `<base>_pending_approvals` — pending + resolved approvals
- `<base>_cost_ledger` — append-only cost entries with BIGSERIAL id

Integration test at `crates/tdw-session/tests/pg_session.rs`
double-gated by `--features postgres` + `TDW_POSTGRES_TEST_URL`.
Hermetic per-process base table; exercises every method in the
full surface as one roundtrip.

## Cross-Store Evidence (G013)

`crates/tdw-session/tests/g013_durable_cross_store.rs` verifies the
G013 durable persistence set together. It uses one `PgEngine` for
`PgOutboxStore`, `PgEventBus`, `PgSessionStore`, and `PgSnapshotStore`,
then writes a locked/synced `JsonlRollout` archive on the filesystem.
The test is double-gated by `--features g013-cross-store` +
`TDW_POSTGRES_TEST_URL`; without the env var it compiles and reports a
skip, preserving offline workspace defaults.
