# tdw-storage-postgres

Postgres [`RelationalEngine`](../tdw-core/src/lib.rs) for the FinX
data-warehouse storage layer.

## Purpose

Executes SQL and returns rows as JSON for the relational tier. Ships:

- [`PostgresRecordingEngine`] — always available, no network. Validates and
  *records* statements (returns synthetic JSON describing the call) so offline
  tests can assert what SQL the pipeline would run. Also implements
  `WriteSink<T>` for any `DataModel`.
- [`PgEngine`] — real sqlx `PgPool` backend behind the `postgres` feature.

## Engine trait

`RelationalEngine`:

- `execute(sql, params) -> Result<u64>` (rows affected)
- `fetch_json(sql, params) -> Result<Vec<Value>>` (rows as JSON objects)

`PostgresRecordingEngine` additionally implements `WriteSink<T>`
(`name`/`write_batch`/`health_check`) so it can sit behind a
[`StorageRouter`](../tdw-storage-router).

## Default (recording) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `PostgresRecordingEngine` | — (always built) | none |
| Real | `PgEngine` | `postgres` | sqlx `PgPool` |

Default features list is empty; `cargo test --workspace` stays offline. Enable
the real driver with `--features postgres`.

## Connection / env vars

```rust
let engine = PgEngine::connect("postgres://user:pass@host/db").await?;
// or reuse an existing pool:
let engine = PgEngine::with_pool(my_pool);
```

- **Parameter binding:** `Value::Null` → no params; `Value::Array([..])` of
  primitives → bound as `$1, $2, …`; anything else → `Error::Storage`.
- **Result conversion:** `fetch_json` wraps the caller's SELECT in
  `SELECT row_to_json(t)::text FROM (… ) t` so Postgres handles per-column type
  conversion server-side.

The env-gated integration test (`tests/sqlx_engine.rs`) reads
`TDW_POSTGRES_TEST_URL`.

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the service relational engine is `PgEngine`,
connected by [`select_relational_engine`](../tdw-service-api/src/app_state.rs)
from the first of:

1. `TDW_POSTGRES_URL`
2. `TDW_DAEMON_PG_URL`
3. `DATABASE_URL`

A missing URL fails the `live` boot closed. `PgEngine` is also the foundation the
durable persistence crates ([`tdw-outbox`](../tdw-outbox),
[`tdw-bus`](../tdw-bus), [`tdw-session`](../tdw-session),
[`tdw-snapshot`](../tdw-snapshot), [`tdw-rollout`](../tdw-rollout)) build on under
their `postgres` features.

## Quickstart (offline)

```rust
use serde_json::json;
use tdw_core::RelationalEngine;
use tdw_storage_postgres::PostgresRecordingEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = PostgresRecordingEngine::default();
engine.execute("insert into raw.market_data_bar values (...)", json!([1])).await?;
let rows = engine.fetch_json("select * from raw.market_data_bar", json!([])).await?;
assert_eq!(rows[0]["engine"], "postgres-recording");
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-postgres --example tdw-storage-postgres-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
