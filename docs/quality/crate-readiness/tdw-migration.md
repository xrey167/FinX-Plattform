# tdw-migration Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-migration\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: xtask
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package metadata, publish=false, edition 2024, workspace lints, and dependency declarations are intentional for this crate role.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies remain bounded by the matrix inventory.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: catalog validation returns typed errors for empty fields, non-create SQL, and duplicate versions per target.
- [x] Runtime behavior reviewed for filesystem, in-memory adapter, recording engine, checksum, migration, or generated-SQL boundaries as applicable.
- [x] Tests and coverage evidence recorded: 2 test attributes detected plus focused tranche and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this bootstrap contract crate when the readiness worksheet records the role and follow-ups.
- [x] Surface wiring reviewed: service-api, xtask, and local reverse dependencies were checked where applicable.
- [x] Scaffold, dead-code, and fallback signals classified: 0 current scan signals, all test-only panic assertions or accepted recording/in-memory follow-up boundaries; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- Postgres and ClickHouse migration catalogs use include_str! fixtures and preserve target/version/name/sql metadata.
- Added validate_migration_catalog and all_migrations so malformed or duplicate catalog entries fail before operator display.
- Migration coverage test checks required schema and system table boundaries for the current bootstrap data plane.
- Follow-up boundary: migration execution and locking policy remain xtask/deployment responsibilities.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit.

## Verdict

Ready with follow-ups. No G003 blocker remains; remaining follow-ups are production adapter depth, orchestration, or durability work layered behind the validated contracts.
