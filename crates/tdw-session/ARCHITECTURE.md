# Architecture — tdw-session

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `SessionError`, value types, `SqliteSessionStore`, `SQLITE_MIGRATION`, row mappers, unit tests |
| `src/pg_session.rs` | `postgres` | `PgSessionStore` (built on `PgEngine`), `ensure_schema` (4 tables) |
| `tests/durability.rs` | always | SQLite durability test |
| `tests/g013_durable_cross_store.rs` | `g013-cross-store` + env | cross-store integration (pg session + bus + outbox + snapshot) |
| `tests/pg_session.rs` | `postgres` + env | double-gated Postgres integration test |

`src/lib.rs` re-exports `PgSessionStore` under `#[cfg(feature = "postgres")]`.

## Store contract & invariants

The store owns four logical tables: `sessions`, `permission_state`,
`pending_approvals`, `cost_ledger` (see `SQLITE_MIGRATION` for the SQLite DDL and
`PgSessionStore::ensure_schema` for the Postgres DDL — the schemas are
intentionally parallel).

### Invariants

- **Self-migrating embedded store.** `SqliteSessionStore::connect` runs
  `migrate()` (splitting `SQLITE_MIGRATION` on `;`) so a fresh DB is immediately
  usable; `from_pool` lets a caller share an existing pool (and is `max_connections(1)`
  for SQLite single-writer safety).
- **Upsert semantics.** `upsert_session` and `save_permission_rules` use
  `ON CONFLICT … DO UPDATE`, so re-writing a session/rules is idempotent.
- **Typed enum round-trip is validated.** Persisted `status` /
  `decision` strings are parsed back to `SessionStatus` / `ApprovalDecision`; an
  unrecognized value surfaces as `SessionError::InvalidSessionStatus` /
  `InvalidApprovalDecision` rather than being silently coerced (covered by
  `sqlite_store_rejects_invalid_persisted_enums`).
- **Approvals are request→resolve.** `request_approval` inserts a row with a
  `null` decision; `resolve_approval` sets the decision and returns the resolved
  record.
- **Cost ledger is append-only**, ordered by insertion id.

### Durability

`SqliteSessionStore` persists to a SQLite database (file or in-memory for tests).
`PgSessionStore` persists the same four-table model to Postgres via `PgEngine`,
with `ensure_schema()` creating all four tables lazily. Durability is the
respective engine's (SQLite / Postgres WAL).

## Real-vs-stub duality design

Both backends are durable; the split is **embedded SQLite (default) vs server
Postgres (opt-in)**. The SQLite store is always compiled and is the canonical
schema/behaviour reference; `PgSessionStore` mirrors it for the server profile
behind `postgres`. The default workspace build links only sqlx (already a regular
dep), not the Postgres driver path.

## Env-gated integration test pattern

`tests/pg_session.rs` and `tests/g013_durable_cross_store.rs` are **double-gated**:
compiled only with their feature (`postgres` / `g013-cross-store`) and run only
when the Postgres test URL is set. `tests/durability.rs` covers the SQLite store
under the default offline workspace test set.

## Migration story

Two parallel, self-applied schemas:

- SQLite: `SQLITE_MIGRATION` const, applied by `migrate()` on connect.
- Postgres: `PgSessionStore::ensure_schema()` issues four
  `CREATE TABLE IF NOT EXISTS tdw_sessions* …` statements.

Both are independent of the main [`tdw-migration`](../tdw-migration) warehouse
catalog — the session store provisions its own runtime tables.
