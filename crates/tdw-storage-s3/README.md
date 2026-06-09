# tdw-storage-s3

S3 / object-store [`BlobEngine`](../tdw-core/src/lib.rs) for the FinX
data-warehouse storage layer.

## Purpose

Stores opaque object bodies keyed by a canonical relative path. Ships two
implementations of `tdw_core::BlobEngine`:

- [`InMemoryS3BlobEngine`] — always available, no network. Keeps objects in a
  `BTreeMap` for offline tests.
- [`S3Engine`] — real `aws-sdk-s3` backend behind the `s3` feature. Talks to AWS
  S3 or any S3-compatible service (MinIO, Cloudflare R2, …).

## Engine trait

Both types implement `BlobEngine`:

- `put_object(key, body, content_type) -> Result<()>`
- `get_object(key) -> Result<Bytes>`

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `InMemoryS3BlobEngine` | — (always built) | none |
| Real | `S3Engine` | `s3` | aws-sdk-s3 |

The default features list is empty, so `cargo test --workspace` stays offline
and never pulls the ~70-crate aws-sdk dependency set. Enable the real backend
with `--features s3`.

## Connection / env vars

`S3Engine` has two constructors:

```rust
// AWS production: credentials + region from the standard AWS env / config chain.
let engine = S3Engine::from_env("my-bucket").await?;

// S3-compatible endpoint (MinIO / R2): explicit endpoint + creds.
let engine = S3Engine::from_endpoint(
    "http://127.0.0.1:9000", "us-east-1", "minioadmin", "minioadmin", "my-bucket",
);
```

The env-gated integration test (`tests/aws_engine.rs`) reads
`TDW_S3_TEST_BUCKET` + `TDW_S3_TEST_ENDPOINT` (and AWS creds).

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the service blob engine is `S3Engine`, wired by
[`select_blob_engine`](../tdw-service-api/src/app_state.rs) from:

| Env var | Meaning |
|---|---|
| `TDW_S3_ENDPOINT` | S3-compatible endpoint URL (required) |
| `TDW_S3_BUCKET` | target bucket (required) |
| `TDW_S3_ACCESS_KEY` | access key (required) |
| `TDW_S3_SECRET_KEY` | secret key (required) |
| `TDW_S3_REGION` | region (optional, defaults to `us-east-1`) |

A missing required var fails the `live` boot closed. The real engine requires the
service-api `real-s3` feature; without it a `live` boot errors with an actionable
message instead of silently using the in-memory engine.

## Quickstart (offline)

```rust
use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_s3::InMemoryS3BlobEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = InMemoryS3BlobEngine::default();
engine.put_object("raw/ohlcv.parquet", Bytes::from_static(b"parquet"), "application/vnd.apache.parquet").await?;
let body = engine.get_object("raw/ohlcv.parquet").await?;
assert_eq!(body, Bytes::from_static(b"parquet"));
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-s3 --example tdw-storage-s3-basic
```

For the MinIO docker run recipe and key validation rules, see
[`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
