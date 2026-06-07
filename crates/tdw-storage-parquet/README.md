# tdw-storage-parquet

Parquet dataset **manifest** types for the FinX data-warehouse — not a storage
engine.

## Purpose

`tdw-storage-parquet` describes a set of Parquet files that make up a logical
table, with integrity checksums. It is a pure, dependency-light value type used to
record and verify what a staged Parquet dataset contains (file paths, row counts,
byte sizes) without reading the files themselves.

- [`ParquetDatasetManifest`] — `{ table, files, checksum }` with `total_rows()`,
  `total_bytes()`, and `verify_checksums()`.
- [`ParquetFile`] — `{ path, row_count, content_length, checksum }`.
- [`ParquetManifestError`] — typed validation/checksum errors.

## Engine trait

None. This crate implements **no** `tdw_core` engine trait. The storage-transports
status table lists it explicitly as "utility, not an engine". The actual blob I/O
is done by [`tdw-storage-s3`](../tdw-storage-s3) /
[`tdw-storage-fs`](../tdw-storage-fs); this crate only models the manifest.

## Default vs real backend

Not applicable — no backend, no feature flag, no network. The crate depends only
on `serde` + `thiserror`.

## Connection / env vars

None.

## `TDW_PROFILE=live` behavior

None. The manifest type is profile-agnostic; it is constructed and verified the
same way regardless of profile.

## Quickstart

```rust
use tdw_storage_parquet::{ParquetDatasetManifest, ParquetFile};

# fn run() -> tdw_storage_parquet::Result<()> {
let file = ParquetFile::new("s3://bucket/raw/ohlcv.parquet", 42, 4096)?;
let manifest = ParquetDatasetManifest::new("raw.market_data_bar", vec![file])?;

assert_eq!(manifest.total_rows(), 42);
assert_eq!(manifest.total_bytes(), 4096);
manifest.verify_checksums()?; // recomputes file + manifest FNV checksums
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-parquet --example tdw-storage-parquet-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). The companion
[`tdw-table-format`](../tdw-table-format) crate models Iceberg/Delta table
manifests on top of Parquet files.
