# tdw-bootstrap

One-shot binary that brings the TDW data backend live before any application
service starts. It connects to Postgres and applies every durable-persistence
schema (outbox, snapshot, bus, session), writes and reads back a marker object in
the configured S3/MinIO bucket, and — when their endpoints are set — creates a
baseline schema/collection/index in ClickHouse, Qdrant, and Meilisearch.

`#![forbid(unsafe_code)]`. It emits one structured JSON line per step (suitable
for `docker compose logs tdw-bootstrap`) and exits non-zero on the first failed
step. It is designed to run as a compose init container, not as a long-running
service.

## Binaries produced

- **`tdw-bootstrap`** — runs the steps in order and exits.

Exit codes: `0` success; `2` env; `3` postgres-connect; `4` postgres-schema;
`5` s3-marker; `6` s3-roundtrip; `7` clickhouse; `8` qdrant; `9` meilisearch.

## Feature flags

None. (Storage crates are pulled in with their backend features always on.)

## Key environment variables

(Full reference in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).)

Required:

- `TDW_POSTGRES_URL` — e.g. `postgres://tdw:tdw@postgres:5432/tdw`
- `TDW_S3_ENDPOINT` / `TDW_S3_BUCKET` / `TDW_S3_ACCESS_KEY` / `TDW_S3_SECRET_KEY`

Optional (each backend is skipped unless its `*_URL` is set):

- `TDW_S3_REGION` (default `us-east-1`)
- `TDW_CLICKHOUSE_URL` (+ `_USER` / `_PASSWORD`)
- `TDW_QDRANT_URL` (+ `_API_KEY` / `_VECTOR_SIZE`, default size 1536)
- `TDW_MEILI_URL` (+ `_API_KEY`)

## Quickstart (binary)

```bash
# Typically run as a compose init container; equivalent shell invocation:
TDW_POSTGRES_URL=postgres://tdw:tdw@localhost:5432/tdw \
TDW_S3_ENDPOINT=http://localhost:9000 TDW_S3_BUCKET=tdw-default \
TDW_S3_ACCESS_KEY=minio TDW_S3_SECRET_KEY=minio123 \
  cargo run -p tdw-bootstrap
```

Each step prints a JSON line like `{"step":"postgres-connect","status":"ok"}`.

> This binary requires live backends and is **not** offline. The bundled
> [`examples/basic.rs`](examples/basic.rs) is an offline config-inspection demo
> that reports which env vars are set/missing and the exit-code contract **without
> connecting to anything**: `cargo run -p tdw-bootstrap --example tdw_bootstrap_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — step sequence and exit-code contract.
- `tdw-storage-*` — the schema/marker helpers each step drives.
