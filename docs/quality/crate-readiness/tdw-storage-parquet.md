# tdw-storage-parquet Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-storage-parquet\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; sha2 ^0.11; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 6
- tests/ directory: no
- README: no
- Examples directory: yes (tdw-storage-parquet-basic)
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package metadata, publish=false, edition 2024, workspace lints, and dependency declarations are intentional for this crate role.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies remain bounded by the matrix inventory.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for data, storage, SQL, pipeline, or manifest responsibilities.
- [x] Runtime behavior reviewed for filesystem, in-memory adapter, recording engine, checksum, migration, or generated-SQL boundaries as applicable.
- [x] Tests and coverage evidence recorded: 6 test attributes; all four required test classes present (round-trip pass, corrupted-file detected, legacy-manifest distinct status, large-file streaming) plus metadata validation test.
- [x] Docs and examples reviewed: module-level doc states exactly what is and is not guaranteed; example updated for new API.
- [x] Surface wiring reviewed: no reverse local dependencies; example updated.
- [x] Scaffold, dead-code, and fallback signals classified: 0 current scan signals; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- G008 PQ1 fixed (2026-06-11): `ParquetFile::checksum` (u64 FNV of path+row_count+length,
  no content read) replaced by `ParquetFile::content_checksum` (lower-case hex SHA-256 of
  file bytes) + `content_checksum_kind` tag (`sha256` | `legacy`).
- `ParquetFile::from_reader` computes a real SHA-256 content hash via a streaming 64 KiB
  reader loop — the file is never fully loaded into memory.
- `ParquetDatasetManifest::verify_checksums` accepts an `open` callback and re-hashes each
  file; corrupted parquet files are detected. Legacy entries return
  `VerifyOutcome::LegacyUnverified` — a loud, distinct status, neither pass nor fail.
- The `(path, row_count, content_length)` triple is retained as cheap metadata validation
  via `verify_metadata` (no file I/O required).
- Old per-manifest FNV checksum field and `calculate_checksum` / `verify_checksums() -> Result<()>`
  API removed; replaced by per-file SHA-256 content hash.
- Follow-up boundary: production ingest paths can plug real object-store readers into
  `verify_checksums`.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate: cargo check --workspace; cargo clippy -p tdw-table-format -p tdw-storage-parquet -p tdw-service-api --all-targets -- -D warnings (0 warnings); cargo test -p tdw-table-format -p tdw-storage-parquet (12 passed, 0 failed).

## Verdict

Ready with follow-ups. G008 PQ1 resolved — checksums now verify real parquet file content via SHA-256. No G003 blocker remains.
