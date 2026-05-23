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
| `tdw-storage-clickhouse` | `OlapEngine` | `ClickHouseRecordingEngine` | `ClickHouseHttpEngine` (reqwest HTTP) | `clickhouse` | `TDW_CLICKHOUSE_TEST_URL` |
| `tdw-storage-qdrant` | `VectorEngine` | `InMemoryVectorEngine` | `QdrantHttpEngine` (reqwest HTTP) | `qdrant` | `TDW_QDRANT_TEST_URL` |
| `tdw-storage-meilisearch` | `LexicalEngine` | `InMemoryLexicalEngine` | `MeilisearchHttpEngine` (reqwest HTTP) | `meilisearch` | `TDW_MEILISEARCH_TEST_URL` |
| `tdw-storage-parquet` | — | (utility, not an engine) | — | — | — |

**G010 storage backends: 5 of 5 real.**

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

## ClickHouse (`tdw-storage-clickhouse`)

Lives at `crates/tdw-storage-clickhouse/src/http_engine.rs`. Direct
reqwest HTTP against ClickHouse's native HTTP interface (port 8123).
No SDK; `POST /?query=...` for execute, append `FORMAT JSON` for
parseable SELECT responses. Auth optional (HTTP basic).

```rust
use tdw_core::OlapEngine;
use tdw_storage_clickhouse::ClickHouseHttpEngine;

let engine = ClickHouseHttpEngine::new("http://127.0.0.1:8123", None, None)?;
engine.execute("CREATE TABLE x (...) ENGINE = Memory").await?;
let payload = engine.query_json("SELECT * FROM x FORMAT JSON", Value::Null).await?;
```

Param binding deferred (ClickHouse uses `param_<name>` query keys, a
different shape from sqlx-style positional binding).

### Docker run recipe (ClickHouse)

```powershell
docker compose --profile minimal up -d clickhouse
$env:TDW_CLICKHOUSE_TEST_URL = "http://127.0.0.1:8123"
cargo test -p tdw-storage-clickhouse --features clickhouse --test http_engine
docker compose --profile minimal down -v
```

## Qdrant (`tdw-storage-qdrant`)

Lives at `crates/tdw-storage-qdrant/src/http_engine.rs`. Direct
reqwest HTTP against Qdrant's REST API (port 6333). Lazy collection
auto-create on first upsert using the first point's vector dimension.

```rust
use tdw_core::{VectorEngine, VectorPoint, VectorQuery};
use tdw_storage_qdrant::QdrantHttpEngine;

let engine = QdrantHttpEngine::new("http://127.0.0.1:6333", None)?;
engine.upsert("my-collection", vec![VectorPoint { id: "1".into(), vector: vec![0.1,0.2,0.3,0.4], payload: json!({}) }]).await?;
let hits = engine.search_knn("my-collection", VectorQuery { vector: vec![0.1,0.2,0.3,0.4], top_k: 5 }).await?;
```

Point IDs in this slice must be unsigned integers or UUIDs (Qdrant
constraint); arbitrary string ID normalization via UUID v5 is a
follow-up.

### Docker run recipe (Qdrant)

```powershell
docker compose --profile full up -d qdrant
$env:TDW_QDRANT_TEST_URL = "http://127.0.0.1:6333"
cargo test -p tdw-storage-qdrant --features qdrant --test http_engine
docker compose --profile full down -v
```

## Meilisearch (`tdw-storage-meilisearch`)

Lives at `crates/tdw-storage-meilisearch/src/http_engine.rs`. Direct
reqwest HTTP against Meilisearch's REST API (port 7700). `index`
polls `/tasks/{uid}` until succeeded so callers can immediately
follow with `search_text` without flakiness.

```rust
use tdw_core::{LexicalEngine, LexicalDoc, TextQuery};
use tdw_storage_meilisearch::MeilisearchHttpEngine;

let engine = MeilisearchHttpEngine::new("http://127.0.0.1:7700", None)?;
engine.index("my-index", vec![LexicalDoc { id: "1".into(), body: "alpha beta".into(), fields: json!({}) }]).await?;
let hits = engine.search_text("my-index", TextQuery { text: "alpha".into(), top_k: 5 }).await?;
```

`showRankingScore: true` populates `ScoredDoc.score`; the
`_rankingScore` field is stripped from returned doc fields.

### Docker run recipe (Meilisearch)

```powershell
docker compose --profile full up -d meilisearch
$env:TDW_MEILISEARCH_TEST_URL = "http://127.0.0.1:7700"
cargo test -p tdw-storage-meilisearch --features meilisearch --test http_engine
docker compose --profile full down -v
```

## Why each feature is opt-in

The real driver crates pull non-trivial transitive dep sets:

- `sqlx postgres` → stringprep, hkdf, hmac, etc.
- `aws-sdk-s3` → `aws-config`, `aws-smithy-*`, `hyper-rustls`, `rustls`, etc. (~70 crates)
- `reqwest` → `hyper`, `hyper-rustls`, `rustls`, `tower-http`, `url`, etc. (~50 crates)

Gating each behind a feature flag keeps the default workspace build
lean and lets consuming crates pick whether they pay for the real
driver.

## CI integration

The `Integration, Property, and E2E Subset` job in
`.github/workflows/ci.yml` brings up the storage containers from
`docker-compose.yaml`, waits for health, and runs the feature-gated
integration tests with the appropriate `TDW_*_TEST_*` env vars set.
Containers covered:

- postgres (`TDW_POSTGRES_TEST_URL`)
- minio (`TDW_S3_TEST_BUCKET` + `TDW_S3_TEST_ENDPOINT` + creds)
- clickhouse (`TDW_CLICKHOUSE_TEST_URL`)
- qdrant (`TDW_QDRANT_TEST_URL`)
- meilisearch (`TDW_MEILISEARCH_TEST_URL`)

Default `cargo test --workspace` remains offline because the integration
tests early-return when their env vars are unset; only the CI
integration job pays the container startup cost.

## G010 status: complete

All five storage backend slices have shipped and CI validates them on
every PR. Next goals build on this foundation:

- **G011** — Production provider transports (Yahoo / FRED / Alpaca /
  Binance / Polygon / HuggingFace). Pattern in
  `docs/quality/production-transport-status.md`.
- **G013** — Durable persistence backends (outbox / session / bus /
  snapshot / rollout) built on top of `PgEngine` + `S3Engine`.
