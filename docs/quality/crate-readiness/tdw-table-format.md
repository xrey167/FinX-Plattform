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
- Test attributes detected: 8
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
- [x] Tests and coverage evidence recorded: 8 test attributes; all required test classes present (round-trip pass, corrupted-file detected, legacy-manifest distinct status, large-file streaming, raw-old-JSON deserialization, mixed legacy+sha256 manifest, iceberg+delta format parity, shape-validation rejects invalid manifests).
- [x] Docs and examples reviewed: module-level doc states exactly what is and is not guaranteed; example updated for new API; legacy migration path documented as reachable via serde wire-struct detection.
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
- G008 review fixes (2026-06-11):
  - Legacy migration path is now reachable: pre-G008 JSON `{"path":"...","checksum":2847}`
    deserializes correctly to `ChecksumKind::Legacy` via a `TableFileWire` struct that
    accepts both shapes. Detection rule: presence of `content_checksum` field → `Sha256`;
    absence (old `checksum: u64` field present or absent) → `Legacy`. `deny_unknown_fields`
    enforces no unrecognised keys slip through.
  - `VerifyOutcome::Mismatch` variant removed — it was dead (never produced). Content
    corruption is signalled via `Err(TableManifestError::ChecksumMismatch)` which callers
    cannot accidentally ignore.
  - `tdw-service-api` now surfaces the tri-state per-file outcome
    (`verified=N legacy_unverified=N failed=0`) rather than collapsing to a bool.
  - Two new raw-JSON deserialization tests and one mixed-manifest test added.

## Verification

- Workspace gate: `cargo check --workspace` → Finished.
- Clippy: `cargo clippy -p tdw-table-format -p tdw-storage-parquet -p tdw-service-api --all-targets --target-dir target -- -D warnings` → 0 warnings, Finished.
- Tests: `cargo test -p tdw-table-format -p tdw-storage-parquet --target-dir target` → 16 passed (8 per crate), 0 failed.
  - `tdw-table-format`: `round_trip_pass`, `corrupted_file_detected`, `legacy_manifest_reports_distinct_unverified_status`, `raw_old_format_json_deserializes_to_legacy`, `mixed_manifest_legacy_and_sha256`, `large_file_streaming_no_full_read`, `shape_validation_rejects_invalid_manifests`, `iceberg_and_delta_manifests_both_verify`.
- `tdw-service-api`: `cargo test -p tdw-service-api --target-dir target` → 144 passed, 0 failed (includes `parity_layer_sample_wires_layer_c_features` which asserts `verified=1 legacy_unverified=0 failed=0`).

## Verdict

Ready with follow-ups. G008 TF1 resolved — checksums now verify real file content via SHA-256. Legacy migration path is reachable. No G003 blocker remains.
