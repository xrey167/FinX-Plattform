# Architecture — tdw-storage-parquet

## Module map

| Path | Contents |
|---|---|
| `src/lib.rs` | `ParquetDatasetManifest`, `ParquetFile`, `ParquetManifestError`, FNV checksum helpers, unit tests |

Single-module, no features, no async.

## Type contract & invariants

### `ParquetFile`

`ParquetFile::new(path, row_count, content_length)` computes a deterministic FNV
checksum over `(path, row_count, content_length)` and validates the shape:

- non-empty path,
- `row_count > 0`,
- `content_length > 0`.

`verify_checksum()` recomputes the FNV hash and compares it, catching silent
tampering of any of the three fields.

### `ParquetDatasetManifest`

`new(table, files)` validates shape (non-empty table, ≥1 file, each file valid)
then computes a manifest-level checksum that folds each file's checksum with the
FNV prime. Invariants enforced by `verify_checksums()`:

- shape is still valid,
- every file's own checksum verifies,
- the manifest checksum matches the recomputed fold.

This gives a two-level integrity check: per-file plus whole-manifest, so a
dropped/added file or a mutated count is detected.

### Checksum design

A small inline FNV-1a (`FNV_OFFSET`/`FNV_PRIME`) keeps the crate
dependency-free and deterministic across platforms (uses `to_le_bytes` for the
integer fields). It is an integrity/consistency check, not a cryptographic
digest.

## Real-vs-stub duality

Not applicable — there is no engine, no network, and nothing to stub. The crate
is a pure value type; the same code runs in every profile.

## Env-gated integration test pattern

Not applicable. Behaviour is covered by the in-crate unit tests
(`manifest_records_totals_and_verifies_checksums`,
`rejects_empty_manifest_boundaries_and_checksum_drift`) under the default offline
workspace test set.

## Migration story

None. The manifest is metadata about Parquet files; it owns no schema. (The
warehouse's `system.table_manifest` SQL table lives in
[`tdw-migration`](../tdw-migration) and is a separate concern.)
