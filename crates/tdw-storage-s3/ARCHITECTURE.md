# Architecture — tdw-storage-s3

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `InMemoryS3BlobEngine`, `StoredObject`, `validate_key`, unit tests |
| `src/aws_engine.rs` | `s3` | `S3Engine` (aws-sdk-s3 client) |
| `tests/aws_engine.rs` | `s3` + env | double-gated integration test |

`src/lib.rs` re-exports `S3Engine` under `#[cfg(feature = "s3")]`.

## Trait contract & invariants

Both engines implement `tdw_core::BlobEngine`:

- **`put_object`** stores the body under `key`. In-memory it inserts into a
  `Mutex<BTreeMap<String, StoredObject>>` (recording the `content_type`); the real
  engine issues an S3 `PutObject`.
- **`get_object`** returns the stored body or `Error::Storage` if the key is
  absent.

### Key validation (shared boundary)

`validate_key` (used by `InMemoryS3BlobEngine`, and the same rules applied by
`S3Engine`) requires every key to be:

- non-empty / non-blank,
- free of `\` (forces `/` separators),
- composed only of `Component::Normal` segments — no `..`, no `./`, no leading
  `/`, no platform prefix.

This is the same contract as [`tdw-storage-fs`](../tdw-storage-fs), so a key that
is valid for local disk is valid for S3 and vice versa.

### Durability

Durability is delegated to the object store. The in-memory engine is volatile (a
test fixture). `S3Engine` inherits S3 PUT durability semantics.

## Real-vs-stub duality design

The recording/real split mirrors every other `tdw-storage-*` engine crate: the
in-memory type is always compiled and is the offline default; the real client is
an opt-in module behind a cargo feature so the default workspace build neither
compiles nor depends on the aws-sdk stack. Selection between the two (and the
local-disk fs engine) happens in `tdw-service-api::app_state` by profile.

### TLS / dependency note

`aws-config` and `aws-sdk-s3` are pinned with `default-features = false` plus
`behavior-version-latest`, `rt-tokio`, and `default-https-client` so the modern
hyper-rustls 0.27 / rustls 0.23 stack is selected (avoiding the advisories tied
to the legacy `rustls` feature path). See the dependency comments in
`Cargo.toml`.

## Env-gated integration test pattern

`tests/aws_engine.rs` is **double-gated**:

1. Compiled only with `--features s3`.
2. Runs only when `TDW_S3_TEST_BUCKET` and `TDW_S3_TEST_ENDPOINT` are set; with
   them unset it early-returns with a stderr notice.

So default `cargo test --workspace` exercises only `InMemoryS3BlobEngine` and
stays deterministic/offline; CI's integration job brings up MinIO and sets the
vars.

### MinIO docker recipe

```powershell
docker run --rm -d --name tdw-minio-smoke -p 9000:9000 `
    -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin `
    quay.io/minio/minio server /data
$env:TDW_S3_TEST_BUCKET = "tdw-smoke"
$env:TDW_S3_TEST_ENDPOINT = "http://127.0.0.1:9000"
cargo test -p tdw-storage-s3 --features s3 --test aws_engine
docker stop tdw-minio-smoke
```

(Full recipe incl. bucket creation: `docs/quality/production-storage-transports.md`.)

## Migration story

None. Object storage is schemaless; buckets are provisioned out of band (CI's
MinIO `mc mb`, or the deployment's S3 account).
