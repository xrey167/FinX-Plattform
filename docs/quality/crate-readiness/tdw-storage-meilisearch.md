# tdw-storage-meilisearch Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-storage-meilisearch\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core
- External dependencies: async-trait ^0.1.89
- Dev dependencies: serde_json ^1.0.145; tokio ^1.52.3 features=[macros, rt-multi-thread, sync]
- Reverse local dependencies: tdw-service-api
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

- InMemoryLexicalEngine now validates index names, document IDs, non-empty query text, and positive top_k.
- Async tests cover index/search behavior plus empty-query and zero-top_k rejection.
- Follow-up boundary: this remains an in-memory lexical contract, not an external Meilisearch client.

## Verification

- Focused G003 tranche check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3 -p tdw-service-api.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit.

## Verdict

Ready with follow-ups. No G003 blocker remains; remaining follow-ups are production adapter depth, orchestration, or durability work layered behind the validated contracts.

## Production Backend Evidence (G010)

`MeilisearchHttpEngine` (gated by `--features meilisearch`) lives in
`crates/tdw-storage-meilisearch/src/http_engine.rs` and implements
`tdw_core::LexicalEngine` directly against Meilisearch's REST API
(port 7700 by default). No SDK crate is required — just `reqwest`.

The existing `InMemoryLexicalEngine` remains the offline test
stand-in; `MeilisearchHttpEngine` is opt-in.

Public surface:
- `MeilisearchHttpEngine::new(endpoint, api_key)` — optional bearer
  token via `Authorization: Bearer ...` header for managed
  deployments or self-hosted instances configured with a master key.
- `LexicalEngine::index` POSTs documents with `primaryKey=id`, then
  blocks on `/tasks/{uid}` until the task reaches `succeeded`
  (Meilisearch indexing is async; without the wait, callers would
  hit empty search results on the next call). Polling: up to ~12s
  total (60 attempts × 200ms).
- `LexicalEngine::search_text` POSTs `/indexes/{name}/search` with
  `showRankingScore: true`; per-hit `_rankingScore` becomes the
  returned `ScoredDoc.score` and is stripped from the doc fields
  before being returned to the caller.

Integration test at
`crates/tdw-storage-meilisearch/tests/http_engine.rs` is double-gated:
compiles only with `--features meilisearch`; runs only when
`TDW_MEILISEARCH_TEST_URL` is set. Exercises a 3-doc index + text
search asserting that "rocket" surfaces both rocket-related docs.

See `docs/quality/production-storage-transports.md` for the full
G010 status table.
