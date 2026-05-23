# Production Storage Transports (G010)

Tracks the conversion of each `tdw-storage-*` crate from its in-memory
default engine to a real network/disk backend behind an opt-in feature
flag.

## Status

| Crate | Trait | In-memory default | Production backend | Feature flag | Env-gated test |
|---|---|---|---|---|---|
| `tdw-storage-fs` | `BlobEngine` | (n/a — real disk-backed by default) | `LocalBlobEngine` | — | — |
| `tdw-storage-postgres` | `RelationalEngine` | `PostgresRecordingEngine` | `PgEngine` (sqlx 0.9 `PgPool`) | `postgres` | `TDW_POSTGRES_TEST_URL` |
| `tdw-storage-s3` | `BlobEngine` | `InMemoryS3BlobEngine` | (pending) | — | — |
| `tdw-storage-clickhouse` | `RelationalEngine` | (in-memory) | (pending) | — | — |
| `tdw-storage-qdrant` | `VectorEngine` | `InMemoryVectorEngine` | (pending) | — | — |
| `tdw-storage-meilisearch` | `LexicalEngine` | `InMemoryLexicalEngine` | (pending) | — | — |
| `tdw-storage-parquet` | — | (utility, not an engine) | — | — | — |

## Postgres (`tdw-storage-postgres`)

Lives at `crates/tdw-storage-postgres/src/sqlx_engine.rs`. Built on
sqlx 0.9's `PgPool` with the `postgres` driver feature. The crate has
two engines side by side: the existing `PostgresRecordingEngine` (always
available, offline) and `PgEngine` (only with `--features postgres`).

### Public surface

```rust
use tdw_core::RelationalEngine;
use tdw_storage_postgres::PgEngine;

let engine = PgEngine::connect("postgres://user:pass@host/db").await?;
// or: let engine = PgEngine::with_pool(my_pool);

engine.execute("CREATE TABLE x (id BIGSERIAL PRIMARY KEY)", Value::Null).await?;
engine.execute(
    "INSERT INTO x DEFAULT VALUES; INSERT INTO x DEFAULT VALUES",
    Value::Null,
).await?;
let rows = engine.fetch_json("SELECT id FROM x ORDER BY id", Value::Null).await?;
// rows: Vec<serde_json::Value>, one per row, e.g. [{"id": 1}, {"id": 2}]
```

### Parameter binding

This slice supports a narrow but useful subset:
- `Value::Null` → no parameters bound
- `Value::Array([...])` of primitives (`null`, `bool`, integer, float, string) → bound as `$1, $2, …` in order
- Anything else returns a clear `Error::Storage` so callers know to extend the binding surface deliberately

Nested arrays, objects, and bare scalars are rejected on purpose to keep
the boundary explicit. Extending the binding surface (binary, decimal,
timestamps, JSONB inputs) is follow-up work tracked in this doc.

### Result conversion

`fetch_json` wraps the caller's SELECT in `SELECT row_to_json(t)::text
FROM (… caller …) t` so Postgres handles per-column type conversion.
This works for any SELECT shape sqlx + postgres can produce without
requiring per-type decode plumbing in this crate. Trade-off: an extra
server-side projection per query; revisit when a benchmark shows it
matters.

### How to run the integration test

```powershell
# Bring up a postgres
docker run --rm -d --name tdw-pg-smoke `
    -e POSTGRES_PASSWORD=tdw -p 55432:5432 `
    postgres:17-alpine

# Point the test at it
$env:TDW_POSTGRES_TEST_URL = "postgres://postgres:tdw@127.0.0.1:55432/postgres"

# Run feature-gated integration test
cargo test -p tdw-storage-postgres --features postgres --test sqlx_engine

# Tear down
docker stop tdw-pg-smoke
```

Without `TDW_POSTGRES_TEST_URL` set, the integration tests early-return
with a stderr notice and do not require a running database — keeps the
default `cargo test --workspace` deterministic and offline.

CI workflow integration (bringing up the postgres container in the
`Integration, Property, and E2E Subset` job and setting the env var) is
a follow-up tracked under this same goal.

## Why the `postgres` feature is opt-in

The sqlx `postgres` driver pulls a non-trivial transitive dep set
(stringprep, hkdf, hmac, etc.). Gating it behind a feature flag keeps
the default workspace build lean and lets crates that consume
`tdw-storage-postgres` decide whether they pay for the real driver.

## Future tranches in this goal

1. **S3** — real `aws-sdk-s3` client behind `s3` feature; integration
   test against a `minio` container.
2. **ClickHouse** — `clickhouse-rs` (or HTTP via reqwest) behind
   `clickhouse` feature; testcontainer for `clickhouse/clickhouse-server`.
3. **Qdrant** — `qdrant-client` behind `qdrant` feature; `qdrant/qdrant`
   testcontainer.
4. **Meilisearch** — `meilisearch-sdk` behind `meilisearch` feature;
   `getmeili/meilisearch` testcontainer.
5. **CI wiring** — extend `.github/workflows/ci.yml` Integration job to
   start each container and set the respective `TDW_*_TEST_URL` env so
   the gated tests actually run in CI.
