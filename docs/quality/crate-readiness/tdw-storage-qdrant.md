# tdw-storage-qdrant Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-storage-qdrant\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core
- External dependencies: async-trait ^0.1.89
- Dev dependencies: serde_json ^1.0.145; tokio ^1.52.3 features=[macros, rt-multi-thread, sync]
- Reverse local dependencies: tdw-knowledge, tdw-service-api
- Feature flags: none
- Test attributes detected: 2
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
- [x] Tests and coverage evidence recorded: 2 test attributes detected plus focused tranche and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this bootstrap contract crate when the readiness worksheet records the role and follow-ups.
- [x] Surface wiring reviewed: service-api, xtask, and local reverse dependencies were checked where applicable.
- [x] Scaffold, dead-code, and fallback signals classified: 2 current scan signals, all test-only panic assertions or accepted recording/in-memory follow-up boundaries; 0 bootstrap stub signals remain.
- [x] Security and reliability risks reviewed for injection, traversal, checksum drift, silent data loss, bad dimensions, empty inputs, and adapter failure modes.

## Findings

- InMemoryVectorEngine now rejects empty collection names, empty point IDs, empty vectors, empty upserts, empty queries, zero top_k, and vector dimension mismatches.
- Async tests cover scoring order and mismatch rejection.
- Follow-up boundary: this remains an in-memory vector contract, not an external Qdrant client.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit.

## Verdict

Ready with follow-ups. No G003 blocker remains; remaining follow-ups are production adapter depth, orchestration, or durability work layered behind the validated contracts.

## Production Backend Evidence (G010)

`QdrantHttpEngine` (gated by `--features qdrant`) lives in
`crates/tdw-storage-qdrant/src/http_engine.rs` and implements
`tdw_core::VectorEngine` directly against Qdrant's REST API (port 6333
by default). No SDK crate is required — just `reqwest`.

Existing `InMemoryVectorEngine` remains the offline test stand-in;
`QdrantHttpEngine` is opt-in.

Public surface:
- `QdrantHttpEngine::new(endpoint, api_key)` — optional `api-key`
  header for managed deployments.
- `with_distance(distance)` — override the vector distance metric for
  auto-created collections (defaults to `Cosine`; valid values
  `Cosine`/`Dot`/`Euclid`).
- `VectorEngine::upsert` — lazily creates the collection on first call
  using the first point's vector dimension; subsequent upserts to the
  same collection skip the existence check via a per-instance cache.
- `VectorEngine::search_knn` — POST `/collections/{name}/points/search`
  with `with_payload: true`; parses `result` array into `ScoredPoint`.

Integration test at `crates/tdw-storage-qdrant/tests/http_engine.rs`
is double-gated: compiles only with `--features qdrant`; runs only
when `TDW_QDRANT_TEST_URL` is set. Exercises a 3-point upsert + kNN
search asserting the closest point is the query itself.

Point ID handling in this slice: Qdrant accepts unsigned-integer or
UUID strings; the engine forwards the caller's `id` verbatim. Mapping
arbitrary string IDs through deterministic UUIDs (e.g. UUID v5) is
a follow-up tracked in
`docs/quality/production-storage-transports.md`.

See `docs/quality/production-storage-transports.md` for the full G010
status table and remaining backends.
