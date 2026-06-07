# tdw-table-format

Open table-format (Iceberg / Delta) **manifest** types for the FinX
data-warehouse — not a storage engine.

## Purpose

Models a versioned table snapshot in an open lakehouse format. [`TableManifest`]
records the `format`, `table`, monotonically increasing `version`, and the list of
[`TableFile`]s that make up that version, with a per-file checksum so the manifest
can be validated without touching the data files.

- [`TableFormat`] — `Iceberg` | `Delta`.
- [`TableManifest`] — `{ format, table, version, files }` with `validate()` and
  `verify_checksums()`.
- [`TableFile`] — `{ path, checksum }`.

## Engine trait

None. This crate implements **no** `tdw_core` engine trait — it is a pure value
type. It sits above [`tdw-storage-parquet`](../tdw-storage-parquet) (which models
the underlying Parquet files) and the blob engines that do the actual I/O.

## Default vs real backend

Not applicable — no backend, no feature flag, no network. Depends only on `serde`
+ `thiserror`.

## Connection / env vars

None.

## `TDW_PROFILE=live` behavior

None. The manifest type is profile-agnostic.

## Quickstart

```rust
use tdw_table_format::{simple_checksum, TableFile, TableFormat, TableManifest};

# fn run() -> tdw_table_format::Result<()> {
let path = "s3://stage/ohlcv.parquet";
let manifest = TableManifest {
    format: TableFormat::Iceberg,
    table: "raw.market_data_bar".to_string(),
    version: 1,
    files: vec![TableFile { path: path.to_string(), checksum: simple_checksum(path) }],
};

manifest.validate()?;            // shape + per-file checksum check
assert!(manifest.verify_checksums());
# Ok(())
# }
```

```sh
cargo run -p tdw-table-format --example tdw-table-format-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md).
