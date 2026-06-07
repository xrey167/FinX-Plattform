# tdw-session

Durable session / permission / cost store for the FinX agent runtime.

## Purpose

Persists per-session runtime state: the session lifecycle record, its permission
rules, pending approval requests + decisions, and a cost ledger (tokens / bytes /
rows / backend). Ships:

- [`SqliteSessionStore`] — always available, the default. Backed by SQLite via
  `sqlx` (an in-process file or `sqlite::memory:`). Self-migrates on connect.
- [`PgSessionStore`] — Postgres-backed store behind the `postgres` feature, built
  on [`tdw-storage-postgres::PgEngine`](../tdw-storage-postgres).

## Store contract

Value types: [`SessionRecord`] / [`SessionStatus`], [`PendingApprovalRecord`],
[`CostLedgerEntry`]. Both stores expose the same async surface:

- `upsert_session` / `get_session`
- `save_permission_rules` / `load_permission_rules`
- `request_approval` / `resolve_approval` / `pending_approval`
- `append_cost` / `cost_entries`

`SqliteSessionStore` adds `connect(url)` / `from_pool(pool)` / `migrate()`.

## Default (SQLite) vs real backend

| | Type | Feature | Backend |
|---|---|---|---|
| Default | `SqliteSessionStore` | — (always built) | SQLite (sqlx) |
| Real (server) | `PgSessionStore` | `postgres` | sqlx via `PgEngine` |

The default is itself durable (SQLite file), so the split is *embedded vs server*
rather than stub vs real. The `postgres` feature pulls
`tdw-storage-postgres/postgres`. A further `g013-cross-store` feature enables the
durable cross-store integration test alongside the pg outbox/bus/snapshot stores.

## Connection / env vars

```rust
// embedded: in-memory or file-backed SQLite (auto-migrates)
let store = SqliteSessionStore::connect("sqlite::memory:").await?;
let store = SqliteSessionStore::connect("sqlite:///var/lib/tdw/session.db").await?;

// server: Postgres-backed, from a connected PgEngine
let engine = tdw_storage_postgres::PgEngine::connect(&url).await?;
let store = PgSessionStore::new(engine);
store.ensure_schema().await?;
```

The `PgEngine` URL comes from the caller (see
[`tdw-storage-postgres`](../tdw-storage-postgres) for live-profile resolution).

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the durable session store can be `PgSessionStore`
on the live `PgEngine`, sharing the same Postgres the daemon's other durable
stores use. `SqliteSessionStore` is the embedded default for the `service` profile
and local runs. The crate holds no profile switch; the binary selects the store.

## Quickstart (offline)

```rust
use tdw_protocol::SessionId;
use tdw_session::{SessionRecord, SessionStatus, SqliteSessionStore};

# async fn run() -> tdw_session::Result<()> {
let store = SqliteSessionStore::connect("sqlite::memory:").await?;
let id = SessionId::new("session-1").expect("session id");
store.upsert_session(&SessionRecord {
    session_id: id.as_str().to_string(),
    status: SessionStatus::Active,
    created_at: "2026-05-22T00:00:00Z".to_string(),
    updated_at: "2026-05-22T00:00:00Z".to_string(),
}).await?;
assert_eq!(store.get_session(&id).await?.unwrap().status, SessionStatus::Active);
# Ok(())
# }
```

```sh
cargo run -p tdw-session --example tdw-session-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). Durable backend rationale is in
`docs/quality/production-storage-transports.md` (G013).
