# Architecture — tdw-storage-postgres

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `PostgresRecordingEngine`, `validate_sql`, `RelationalEngine` + `WriteSink` impls, unit tests |
| `src/sqlx_engine.rs` | `postgres` | `PgEngine` (sqlx `PgPool`), param binding, `row_to_json` result conversion |
| `tests/sqlx_engine.rs` | `postgres` + env | double-gated integration test |

`src/lib.rs` re-exports `PgEngine` under `#[cfg(feature = "postgres")]`.

## Trait contract & invariants

`tdw_core::RelationalEngine`:

- **`execute`** — validates the SQL is non-empty; the recording engine pushes it
  onto a `Mutex<Vec<String>>` and returns `1`. `PgEngine` runs it on the pool and
  returns the affected-row count.
- **`fetch_json`** — recording engine returns a single synthetic JSON row echoing
  `engine`/`sql`/`params`. `PgEngine` wraps the caller SELECT in
  `SELECT row_to_json(t)::text FROM (… ) t` and parses each text row to `Value`.

### Parameter binding invariant

`PgEngine` accepts `params` as either `Value::Null` (no params) or a
`Value::Array` of primitive scalars (null / bool / i64 / f64 / string), bound
positionally to `$1, $2, …`. A non-array, non-null value is rejected with
`Error::Storage` rather than silently dropped — keeping the bind contract
explicit and injection-safe (values never interpolate into the SQL text).

### `WriteSink` contract

`PostgresRecordingEngine` implements `WriteSink<T>`: `write_batch` adds
`batch.rows.len()` to a counter and returns a `WriteReceipt`; `health_check`
returns `Healthy`. This lets the recording engine fan in behind
`StorageRouter`.

## Real-vs-stub duality design

`PostgresRecordingEngine` (always built) is the offline stand-in; `PgEngine`
(feature `postgres`) is the real driver. The default feature list is empty so the
workspace neither compiles nor links sqlx by default. The service layer chooses
`PgEngine` only in the `live` engine path.

## Env-gated integration test pattern

`tests/sqlx_engine.rs` is **double-gated**: compiled only with `--features
postgres`, and runs only when `TDW_POSTGRES_TEST_URL` is set (otherwise it
early-returns with a stderr notice). Default `cargo test --workspace` exercises
only the recording engine.

### Docker recipe

```powershell
docker run --rm -d --name tdw-pg-smoke -e POSTGRES_PASSWORD=tdw -p 55432:5432 postgres:17-alpine
$env:TDW_POSTGRES_TEST_URL = "postgres://postgres:tdw@127.0.0.1:55432/postgres"
cargo test -p tdw-storage-postgres --features postgres --test sqlx_engine
docker stop tdw-pg-smoke
```

## Migration story

This crate runs whatever SQL the caller passes; it does not own a schema. The
durable warehouse schema is defined in [`tdw-migration`](../tdw-migration)
(`postgres_migrations()`), applied via `cargo run -p xtask -- migrate up`. The
persistence crates that build on `PgEngine` create their own small tables lazily
via `ensure_schema()` (`CREATE TABLE IF NOT EXISTS`), independent of the main
migration catalog.
