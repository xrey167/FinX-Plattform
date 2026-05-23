# tdw-storage-clickhouse Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-storage-clickhouse\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core
- External dependencies: async-trait ^0.1.89; serde_json ^1.0.145
- Dev dependencies: schemars ^1.2.1; serde ^1.0.228 features=[derive]
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 7 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package metadata, publish=false, edition 2024, workspace lints, and dependency declarations are intentional for this crate role.
- [x] Dependency direction reviewed: local dependencies are tdw-core; reverse dependencies remain bounded by the matrix inventory.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for data, storage, SQL, pipeline, or manifest responsibilities.
- [x] Runtime behavior reviewed for filesystem, in-memory adapter, recording engine, checksum, migration, or generated-SQL boundaries as applicable.
- [x] Tests and coverage evidence recorded: 2 test attributes detected plus focused tranche and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this bootstrap contract crate when the readiness worksheet records the role and follow-ups.
- [x] Surface wiring reviewed: service-api, xtask, and local reverse dependencies were checked where applicable.
- [x] Scaffold, dead-code, and fallback signals classified: 7 current scan signals, all test-only panic assertions or accepted recording/in-memory follow-up boundaries; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- Recording OLAP/write sink maps mutex poison to storage errors and now rejects empty SQL/DDL.
- Tests cover statements, query echo, row receipts, and health status.
- Follow-up boundary: this is a recording engine, not a networked ClickHouse client.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit.

## Verdict

Ready with follow-ups. No G003 blocker remains; remaining follow-ups are production adapter depth, orchestration, or durability work layered behind the validated contracts.

## Production Backend Evidence (G010)

`ClickHouseHttpEngine` (gated by `--features clickhouse`) lives in
`crates/tdw-storage-clickhouse/src/http_engine.rs` and implements
`tdw_core::OlapEngine` directly against ClickHouse's native HTTP
interface (port 8123 by default). No SDK crate is required — just
`reqwest` via the newly-added workspace dep.

The existing `ClickHouseRecordingEngine` remains the offline test
stand-in; `ClickHouseHttpEngine` is opt-in so the default workspace
test set stays deterministic and offline.

Authentication uses HTTP basic auth via the constructor: pass
`Some(user)` / `Some(password)` if the ClickHouse instance is
configured for auth, or `None`/`None` for the default `default` user
with no password (the standard local-development setup).

Integration test at `crates/tdw-storage-clickhouse/tests/http_engine.rs`
is double-gated: compiles only with `--features clickhouse` and runs
only when `TDW_CLICKHOUSE_TEST_URL` is set.

Param binding is not yet supported on the HTTP interface (ClickHouse
exposes server-side params via `param_<name>` query string keys,
which is a different binding shape than sqlx-style positional `$N`).
The engine rejects non-null params with a clear error so production
callers extend the binding surface deliberately.

See `docs/quality/production-storage-transports.md` for the full G010
status table and remaining backends.
