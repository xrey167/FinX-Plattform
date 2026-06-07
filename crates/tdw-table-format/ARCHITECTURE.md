# Architecture — tdw-table-format

## Module map

| Path | Contents |
|---|---|
| `src/lib.rs` | `TableFormat`, `TableFile`, `TableManifest`, `TableManifestError`, `simple_checksum`, unit tests |

Single-module, no features, no async.

## Type contract & invariants

`TableManifest::validate()` enforces:

- non-empty `table`,
- `version > 0` (versions are 1-based; `0` is rejected),
- at least one file,
- each file path non-empty,
- each file `checksum == simple_checksum(path)`.

`verify_checksums()` is a boolean convenience wrapper over `validate()`.

### Checksum design

`simple_checksum` sums the path bytes (`u64`). It is a lightweight
path-consistency check that detects a swapped or mistyped file path within a
manifest; like [`tdw-storage-parquet`](../tdw-storage-parquet) it is an integrity
aid, not a cryptographic digest. The two crates use different checksum schemes
deliberately — parquet manifests fold an FNV hash over `(path, rows, bytes)`,
whereas table manifests only key on path — because a table-format file entry here
carries just `{ path, checksum }`.

### Versioning invariant

`version` is the open-table-format snapshot version. The type does not itself
allocate versions (callers supply them, as Iceberg/Delta metadata would); it only
guarantees a stored version is non-zero. Compare with
[`tdw-snapshot`](../tdw-snapshot), which *does* allocate monotonic per-table
versions for its parity-layer snapshots.

## Real-vs-stub duality

Not applicable — pure value type, no engine to stub, identical across profiles.

## Env-gated integration test pattern

Not applicable. Covered by the in-crate unit tests
(`verifies_iceberg_and_delta_manifest_checksums`,
`rejects_invalid_manifest_shape_and_checksum_drift`) under the default offline
workspace test set.

## Migration story

None. The crate owns no schema. (The warehouse's `system.table_manifest` SQL
table is defined in [`tdw-migration`](../tdw-migration).)
