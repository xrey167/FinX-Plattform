# Architecture — tdw-storage-clickhouse

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `ClickHouseRecordingEngine`, ingest helpers (`build_insert_jsoneachrow`, `ingest_dedup_token`, `batch_dedup_token`), identifier/string guards, unit tests |
| `src/http_engine.rs` | `clickhouse` | `ClickHouseHttpEngine` (reqwest HTTP client) |
| `tests/http_engine.rs` | `clickhouse` + env | double-gated integration test |

`src/lib.rs` re-exports `ClickHouseHttpEngine` under `#[cfg(feature = "clickhouse")]`.

## Trait contract & invariants

`tdw_core::OlapEngine`:

- **`execute(ddl)`** — validates non-empty SQL; recording engine records it, the
  HTTP engine `POST`s it.
- **`query_json(sql, params)`** — recording engine returns a synthetic JSON
  object; the HTTP engine runs the query with `FORMAT JSON` and parses the body.

`ClickHouseRecordingEngine` also implements `WriteSink<T>` (counter +
`Healthy`).

### Idempotent-ingest invariants

The ingest helpers encode the correctness rules for at-least-once delivery into
ClickHouse:

- `build_insert_jsoneachrow` emits **synchronous** inserts (never
  `async_insert`) with `deduplicate_blocks_in_dependent_materialized_views=1`
  and an `insert_deduplication_token`. ClickHouse 25.x rejects `async_insert`
  combined with the dependent-MV dedup (Code 344), and dependent-MV dedup
  correctness (no double-counted OHLC aggregates on a retried batch) takes
  priority.
- The table name is interpolated (ClickHouse has no identifier bind param), so it
  is validated as a plain `[db.]table` identifier rather than escaped — an
  injection-surface guard. The dedup token is escaped for a single-quoted SQL
  literal.
- `ingest_dedup_token` keys on `(session_id, sequence, table)` (retry-stable for
  the protocol path). `batch_dedup_token` hashes serialized row content for the
  streaming path that has no protocol sequence — a retried identical batch hashes
  the same (deduped); distinct batches hash differently (both kept).

## Real-vs-stub duality design

Same pattern as the sibling engine crates: the recording engine is always
compiled (offline default), the reqwest HTTP engine is opt-in behind the
`clickhouse` feature so the default workspace build pulls no HTTP stack. The
service layer selects the HTTP engine only on the `live` path.

## Env-gated integration test pattern

`tests/http_engine.rs` is **double-gated**: compiled only with `--features
clickhouse`, runs only when `TDW_CLICKHOUSE_TEST_URL` is set (else early-returns
with a stderr notice).

### Docker recipe

```powershell
docker compose --profile minimal up -d clickhouse
$env:TDW_CLICKHOUSE_TEST_URL = "http://127.0.0.1:8123"
cargo test -p tdw-storage-clickhouse --features clickhouse --test http_engine
docker compose --profile minimal down -v
```

## Migration story

The analytics schema (raw/silver tables, materialized views, dictionaries) is
owned by [`tdw-migration`](../tdw-migration)'s `clickhouse_migrations()` (20+
files under `migrations/clickhouse/`), applied via
`cargo run -p xtask -- migrate up`. This crate executes statements against an
already-migrated cluster; it carries no schema of its own.
