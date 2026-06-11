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
- Test attributes detected: 8
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
- [x] Tests and coverage evidence recorded: 8 test attributes; all required test classes present (round-trip pass, corrupted-file detected, legacy-manifest distinct status, large-file streaming, raw-old-JSON deserialization, mixed legacy+sha256 manifest, metadata drift with proper variant names, shape-validation rejects empty fields).
- [x] Docs and examples reviewed: module-level doc states exactly what is and is not guaranteed; example updated for new API; legacy migration path documented as reachable via serde wire-struct detection.
- [x] Surface wiring reviewed: no reverse local dependencies; example updated.
- [x] Scaffold, dead-code, and fallback signals classified: 0 current scan signals; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- G008 PQ1 fixed (2026-06-11): `ParquetFile::checksum` (u64 FNV of `path+row_count+content_length`,
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
- G008 review fixes (2026-06-11):
  - Legacy migration path is now reachable: pre-G008 JSON `{"path":"...","row_count":1,"content_length":1,"checksum":2847}`
    deserializes correctly to `ChecksumKind::Legacy` via a `ParquetFileWire` struct that
    accepts both shapes. Detection rule: presence of `content_checksum` field → `Sha256`;
    absence → `Legacy`. `deny_unknown_fields` enforces no unrecognised keys slip through.
  - `verify_metadata` now returns `RowCountDrift { path, expected, actual }` and
    `ContentLengthDrift { path, expected, actual }` instead of reusing the generic
    `EmptyFileRows` / `EmptyFileBytes` error variants — callers can distinguish the
    exact drift dimension.
  - Two new raw-JSON deserialization tests and one mixed-manifest test added.

## Verification

- Workspace gate: `cargo check --workspace` → Finished.
- Clippy: `cargo clippy -p tdw-table-format -p tdw-storage-parquet -p tdw-service-api --all-targets --target-dir target -- -D warnings` → 0 warnings, Finished.
- Tests: `cargo test -p tdw-table-format -p tdw-storage-parquet --target-dir target` → 16 passed (8 per crate), 0 failed.
  - `tdw-storage-parquet`: `round_trip_pass`, `corrupted_file_detected`, `legacy_manifest_reports_distinct_unverified_status`, `raw_old_format_json_deserializes_to_legacy`, `mixed_manifest_legacy_and_sha256`, `large_file_streaming_no_full_read`, `shape_validation_rejects_empty_fields`, `verify_metadata_detects_drift_with_proper_variants`.

## Verdict

Ready with follow-ups. G008 PQ1 resolved — checksums now verify real parquet file content via SHA-256. Legacy migration path is reachable. Drift errors are properly typed. No G003 blocker remains.
