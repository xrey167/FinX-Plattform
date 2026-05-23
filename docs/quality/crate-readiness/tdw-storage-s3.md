# tdw-storage-s3 Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-storage-s3\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core
- External dependencies: async-trait ^0.1.89; bytes ^1.11.0
- Dev dependencies: tokio ^1.52.3 features=[macros, rt-multi-thread, sync]
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 3
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package metadata, publish=false, edition 2024, workspace lints, and dependency declarations are intentional for this crate role.
- [x] Dependency direction reviewed: local dependencies are tdw-core; reverse dependencies remain bounded by the matrix inventory.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for data, storage, SQL, pipeline, or manifest responsibilities.
- [x] Runtime behavior reviewed for filesystem, in-memory adapter, recording engine, checksum, migration, or generated-SQL boundaries as applicable.
- [x] Tests and coverage evidence recorded: 3 test attributes detected plus focused tranche and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this bootstrap contract crate when the readiness worksheet records the role and follow-ups.
- [x] Surface wiring reviewed: service-api, xtask, and local reverse dependencies were checked where applicable.
- [x] Scaffold, dead-code, and fallback signals classified: 2 current scan signals, all test-only panic assertions or accepted recording/in-memory follow-up boundaries; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- InMemoryS3BlobEngine now validates canonical relative object keys and rejects empty, parent, rooted, and backslash keys.
- Async tests cover put/get behavior and invalid key rejection.
- Follow-up boundary: this is an in-memory S3-shaped adapter, not an AWS client.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit.

## Verdict

Ready with follow-ups. No G003 blocker remains; remaining follow-ups are production adapter depth, orchestration, or durability work layered behind the validated contracts.

## Production Backend Evidence (G010)

`S3Engine` (gated by `--features s3`) lands in
`crates/tdw-storage-s3/src/aws_engine.rs` and implements
`tdw_core::BlobEngine` directly against `aws_sdk_s3::Client`. The default
`InMemoryS3BlobEngine` remains the offline test stand-in; `S3Engine` is
opt-in so the default workspace test set stays deterministic and
offline.

Two constructors are exposed:
- `S3Engine::from_env(bucket)` — for AWS production, resolves
  credentials and region from the standard chain (env vars, shared
  config files, instance profile).
- `S3Engine::from_endpoint(endpoint, region, access_key, secret_key, bucket)`
  — for MinIO and any S3-compatible service; uses path-style addressing.

Integration test at `crates/tdw-storage-s3/tests/aws_engine.rs` is
double-gated: compiles only with `--features s3` and runs only when
`TDW_S3_TEST_BUCKET` + `TDW_S3_TEST_ENDPOINT` are both set. CI workflows
that bring up a MinIO container should set these.

See `docs/quality/production-storage-transports.md` for the full G010
status table and remaining backends.
