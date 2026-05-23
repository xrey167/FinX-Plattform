# Production Storage Transports (G010)

Tracks the conversion of each `tdw-storage-*` crate from its in-memory
default engine to a real network/disk backend behind an opt-in feature
flag.

## Status

| Crate | Trait | In-memory default | Production backend | Feature flag | Env-gated test |
|---|---|---|---|---|---|
| `tdw-storage-fs` | `BlobEngine` | (n/a — disk-backed by default) | `LocalBlobEngine` | — | — |
| `tdw-storage-postgres` | `RelationalEngine` | `PostgresRecordingEngine` | `PgEngine` (sqlx 0.9 `PgPool`) | `postgres` | `TDW_POSTGRES_TEST_URL` |
| `tdw-storage-s3` | `BlobEngine` | `InMemoryS3BlobEngine` | `S3Engine` (aws-sdk-s3) | `s3` | `TDW_S3_TEST_BUCKET` + `TDW_S3_TEST_ENDPOINT` |
| `tdw-storage-clickhouse` | `RelationalEngine` | (in-memory) | (pending) | — | — |
| `tdw-storage-qdrant` | `VectorEngine` | `InMemoryVectorEngine` | (pending) | — | — |
| `tdw-storage-meilisearch` | `LexicalEngine` | `InMemoryLexicalEngine` | (pending) | — | — |
| `tdw-storage-parquet` | — | (utility, not an engine) | — | — | — |

## Common pattern

Each storage crate gets:

1. **A feature flag** (`postgres`, `s3`, `clickhouse`, `qdrant`, `meilisearch`) opting in to the real client. Default features list stays empty so `cargo test --workspace` is offline.
2. **One new module** behind the feature (e.g. `src/sqlx_engine.rs`, `src/aws_engine.rs`) implementing the relevant `tdw_core` engine trait against the real client.
3. **The existing `InMemory*Engine` preserved** as the offline test stand-in.
4. **An integration test** at `tests/<engine>.rs` that is *double-gated*: compiled only with the feature, and runs only when the per-backend env vars are set. With env vars unset, the test early-returns with a stderr notice. Default `cargo test --workspace` exercises only the in-memory engine, so the workspace stays deterministic and offline.

## Postgres (`tdw-storage-postgres`)

Lives at `crates/tdw-storage-postgres/src/sqlx_engine.rs`. Built on
sqlx 0.9's `PgPool` with the `postgres` driver feature.

### Public surface

```rust
use tdw_core::RelationalEngine;
use tdw_storage_postgres::PgEngine;

let engine = PgEngine::connect("postgres://user:pass@host/db").await?;
// or: let engine = PgEngine::with_pool(my_pool);

engine.execute("CREATE TABLE x (id BIGSERIAL PRIMARY KEY)", Value::Null).await?;
let rows = engine.fetch_json("SELECT id FROM x ORDER BY id", Value::Null).await?;
```

### Parameter binding

- `Value::Null` → no parameters
- `Value::Array([...])` of primitives (null, bool, i64, f64, string) → bound as `$1, $2, …` in order
- Anything else returns a clear `Error::Storage`

### Result conversion

`fetch_json` wraps the caller's SELECT in
`SELECT row_to_json(t)::text FROM (… caller …) t` so Postgres handles
per-column type conversion server-side.

### Docker run recipe

```powershell
docker run --rm -d --name tdw-pg-smoke `
    -e POSTGRES_PASSWORD=tdw -p 55432:5432 `
    postgres:17-alpine

$env:TDW_POSTGRES_TEST_URL = "postgres://postgres:tdw@127.0.0.1:55432/postgres"
cargo test -p tdw-storage-postgres --features postgres --test sqlx_engine

docker stop tdw-pg-smoke
```

## S3 (`tdw-storage-s3`)

Lives at `crates/tdw-storage-s3/src/aws_engine.rs`. Built on
`aws-sdk-s3` 1.x with the `rt-tokio` + `rustls` features.

### Public surface

```rust
use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_s3::S3Engine;

// AWS production
let engine = S3Engine::from_env("my-bucket").await?;

// MinIO / R2 / any S3-compatible service
let engine = S3Engine::from_endpoint(
    "http://127.0.0.1:9000",
    "us-east-1",
    "minioadmin",
    "minioadmin",
    "my-bucket",
);

engine.put_object("k.json", Bytes::from_static(b"..."), "application/json").await?;
let body = engine.get_object("k.json").await?;
```

### Key validation

The same rules as `InMemoryS3BlobEngine` apply: keys must be non-empty,
must not contain `\\`, and must be relative + canonical (no `..`, `./`,
leading `/`).

### Docker run recipe (MinIO)

```powershell
docker run --rm -d --name tdw-minio-smoke `
    -p 9000:9000 -p 9001:9001 `
    -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin `
    quay.io/minio/minio server /data --console-address ":9001"

# Create the bucket (one-shot)
docker run --rm --network host minio/mc `
    sh -c "mc alias set local http://127.0.0.1:9000 minioadmin minioadmin && mc mb local/tdw-smoke"

$env:TDW_S3_TEST_BUCKET = "tdw-smoke"
$env:TDW_S3_TEST_ENDPOINT = "http://127.0.0.1:9000"
cargo test -p tdw-storage-s3 --features s3 --test aws_engine

docker stop tdw-minio-smoke
```

## Why each feature is opt-in

The real driver crates pull non-trivial transitive dep sets:

- `sqlx postgres` → stringprep, hkdf, hmac, etc.
- `aws-sdk-s3` → `aws-config`, `aws-smithy-*`, `hyper-rustls`, `rustls`, etc. (~70 crates)

Gating each behind a feature flag keeps the default workspace build
lean and lets consuming crates pick whether they pay for the real
driver.

## Follow-ups in this goal

1. **ClickHouse** (`clickhouse-rs` or HTTP via reqwest) behind
   `clickhouse` feature; testcontainer for
   `clickhouse/clickhouse-server`.
2. **Qdrant** (`qdrant-client`) behind `qdrant` feature; `qdrant/qdrant`
   testcontainer.
3. **Meilisearch** (`meilisearch-sdk`) behind `meilisearch` feature;
   `getmeili/meilisearch` testcontainer.
4. **CI wiring** — extend `.github/workflows/ci.yml` `Integration,
   Property, and E2E Subset` job to start each container (postgres +
   minio + clickhouse + qdrant + meilisearch) and set the respective
   `TDW_*_TEST_URL` / `TDW_*_TEST_BUCKET` env vars so the gated tests
   actually run in CI.
