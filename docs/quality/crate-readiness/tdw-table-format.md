# tdw-table-format Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-table-format\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; sha2 ^0.11; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 6
- tests/ directory: no
- README: no
- Examples directory: yes (tdw-table-format-basic)
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package metadata, publish=false, edition 2024, workspace lints, and dependency declarations are intentional for this crate role.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies remain bounded by the matrix inventory.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for data, storage, SQL, pipeline, or manifest responsibilities.
- [x] Runtime behavior reviewed for filesystem, in-memory adapter, recording engine, checksum, migration, or generated-SQL boundaries as applicable.
- [x] Tests and coverage evidence recorded: 6 test attributes; all four required test classes present (round-trip pass, corrupted-file detected, legacy-manifest distinct status, large-file streaming).
- [x] Docs and examples reviewed: module-level doc states exactly what is and is not guaranteed; example updated for new API.
- [x] Surface wiring reviewed: service-api and local reverse dependencies checked and updated for new API.
- [x] Scaffold, dead-code, and fallback signals classified: 0 current scan signals; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- G008 TF1 fixed (2026-06-11): `TableFile::checksum` (u64 path-byte-sum, no content read)
  replaced by `TableFile::content_checksum` (lower-case hex SHA-256 of file bytes) +
  `checksum_kind` tag (`sha256` | `legacy`).
- `TableFile::from_reader` computes a real SHA-256 content hash via a streaming 64 KiB
  reader loop — the file is never fully loaded into memory.
- `TableManifest::verify_checksums` accepts an `open` callback and re-hashes each file;
  corrupted files are detected. Legacy entries (`checksum_kind = legacy`) return
  `VerifyOutcome::LegacyUnverified` — a loud, distinct status that is neither pass nor
  fail, rather than silently passing.
- Old `simple_checksum` function and `validate` / `verify_checksums() -> bool` API removed;
  callers (tdw-service-api) updated.
- Follow-up boundary: production manifest readers can plug real object-store readers
  into `verify_checksums`.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate: cargo check --workspace; cargo clippy -p tdw-table-format -p tdw-storage-parquet -p tdw-service-api --all-targets -- -D warnings (0 warnings); cargo test -p tdw-table-format -p tdw-storage-parquet (12 passed, 0 failed).

## Verdict

Ready with follow-ups. G008 TF1 resolved — checksums now verify real file content via SHA-256. No G003 blocker remains.
