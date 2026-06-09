# tdw-storage-fs

Local-filesystem [`BlobEngine`](../tdw-core/src/lib.rs) for the FinX data-warehouse
storage layer.

## Purpose

`tdw-storage-fs` provides [`LocalBlobEngine`], a disk-backed implementation of
`tdw_core::BlobEngine`. It writes and reads opaque object bodies (`bytes::Bytes`)
under a configured root directory, validating every key so a caller can never
escape that root. It is the local-disk counterpart to the S3 blob engine in
[`tdw-storage-s3`](../tdw-storage-s3): same trait, no network.

## Engine trait

Implements `BlobEngine`:

- `put_object(key, body, content_type) -> Result<()>`
- `get_object(key) -> Result<Bytes>`

The `content_type` argument is accepted for trait parity but not persisted (the
filesystem has no per-object metadata store here).

## Default vs real backend

Unlike the other storage crates, this crate has **no in-memory stand-in and no
feature flag** — it is disk-backed by default. `LocalBlobEngine` is the real
backend; it simply targets the local filesystem instead of a remote object
store. There is nothing to gate behind a feature because it pulls no network
driver (only `std::fs` + `bytes`).

## Connection / configuration

There are no connection env vars. The engine is constructed with a root path:

```rust
let engine = LocalBlobEngine::new("/var/lib/tdw/blobs");
```

### Key rules

`resolve_key` rejects a key that is empty/blank, contains a `\` separator, or
contains any non-`Normal` path component (`..`, `.`, a leading `/`, or a Windows
prefix). This matches the canonical-relative-key contract enforced by
`InMemoryS3BlobEngine` so the two engines are interchangeable.

## `TDW_PROFILE=live` behavior

In the `live` profile the service blob engine is the real S3 backend
(`tdw-storage-s3::S3Engine`), wired from the `TDW_S3_*` env vars — see
[`select_blob_engine`](../tdw-service-api/src/app_state.rs). `LocalBlobEngine` is
selected by the **`service`** profile (rooted at the configured `data_dir`), and
the in-memory engine is the offline fallback. So `tdw-storage-fs` is the
local-disk path, not the `live` path.

## Quickstart

```rust
use bytes::Bytes;
use tdw_core::BlobEngine;
use tdw_storage_fs::LocalBlobEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = LocalBlobEngine::new(std::env::temp_dir().join("tdw-blobs"));
engine
    .put_object("raw/ohlcv.bin", Bytes::from_static(b"payload"), "application/octet-stream")
    .await?;
let body = engine.get_object("raw/ohlcv.bin").await?;
assert_eq!(body, Bytes::from_static(b"payload"));
# Ok(())
# }
```

Run the offline example:

```sh
cargo run -p tdw-storage-fs --example tdw-storage-fs-basic
```

See also [`ARCHITECTURE.md`](ARCHITECTURE.md) and the storage-transports
reference at `docs/quality/production-storage-transports.md`.
