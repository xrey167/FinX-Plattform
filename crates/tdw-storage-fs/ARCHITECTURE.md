# Architecture — tdw-storage-fs

## Module map

| Path | Contents |
|---|---|
| `src/lib.rs` | `LocalBlobEngine` struct, `resolve_key` key validation, `BlobEngine` impl, unit tests |

A single-module crate. No feature-gated modules.

## Trait contract & invariants

Implements `tdw_core::BlobEngine`:

- **`put_object`** creates any missing parent directories under the root, then
  writes the body with `std::fs::write` (a full overwrite — last write wins).
- **`get_object`** reads the whole object into memory and returns it as `Bytes`.
  A missing file surfaces as `Error::Storage` (the underlying `io::Error`
  stringified).

### Key-containment invariant (the security boundary)

Every operation routes through `resolve_key`, which guarantees the resolved path
stays inside `root`:

1. Reject empty/whitespace keys and any key containing `\`.
2. Walk `Path::components()` and reject if any component is `CurDir` (`.`),
   `ParentDir` (`..`), `RootDir` (leading `/`), or `Prefix` (a Windows drive
   prefix). Only `Component::Normal` segments are allowed.
3. Join the validated relative key onto `root`.

This is intentionally identical to the key rules in
`tdw-storage-s3::InMemoryS3BlobEngine`/`S3Engine`, so the same key set is valid
across both engines and a caller cannot traverse out of the configured root.

### Durability

`put_object` relies on `std::fs::write` semantics: the OS buffers the write and
returns once the data is handed to the kernel. This crate does **not** call
`fsync`/`sync_all` (contrast with [`tdw-rollout`](../tdw-rollout), which fsyncs
each appended frame). It targets bulk blob storage where the object store /
filesystem durability policy governs, not per-write fsync.

## Real-vs-stub duality

There is no stub. The crate is the local-disk realisation of `BlobEngine`. The
"duality" lives one layer up: the service-api layer selects between this
local-disk engine, the in-memory engine, and the real S3 engine by profile (see
`tdw-service-api::app_state::select_blob_engine`).

## Env-gated integration test pattern

Not applicable — there is no network backend, so there is no double-gated
`tests/<engine>.rs`. The crate's behaviour is fully covered by the in-crate unit
tests (`rejects_path_traversal_keys`, `writes_and_reads_relative_keys`), which
run under the default offline `cargo test --workspace`.

## Migration story

None. The filesystem needs no schema; directories are created lazily on write.
