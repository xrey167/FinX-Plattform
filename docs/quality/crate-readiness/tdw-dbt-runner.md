# tdw-dbt-runner Readiness Worksheet

Owner tranche: G003-data-storage-pipeline-and-sql-crates - Data, Storage, Pipeline, and SQL Crates.

## Baseline Inventory

- Manifest: crates\tdw-dbt-runner\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are none and no reverse local crates currently depend on this adapter.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: dbt command construction now returns typed validation errors for empty paths/selectors and selector control characters.
- [x] Runtime behavior reviewed: crate remains an offline command/result contract and does not execute dbt directly.
- [x] Tests and coverage evidence recorded: 2 test attributes cover JSON result parsing and command validation.
- [x] Docs and examples reviewed: no per-crate README/examples required while this crate exposes a small typed helper API.
- [x] Surface wiring reviewed: no current service or xtask caller invokes the dbt command builder.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: selector/path validation prevents malformed command payloads from being silently accepted.

## Findings

- Added `DbtCommandError` and fallible builders so invalid project/profile/selector inputs are rejected before a runner shell boundary.
- `parse_run_results` keeps serde_json parse errors explicit and unchanged.
- Follow-up boundary: process execution, dbt binary discovery, and subprocess sandboxing belong to a later runtime/operator integration crate.

## Verification

- Focused G003 crate check passed: cargo test -p tdw-dbt-runner -p tdw-migration -p tdw-pipe -p tdw-pipeline -p tdw-sql-codegen -p tdw-stage -p tdw-table-format -p tdw-storage-clickhouse -p tdw-storage-fs -p tdw-storage-meilisearch -p tdw-storage-parquet -p tdw-storage-postgres -p tdw-storage-qdrant -p tdw-storage-router -p tdw-storage-s3.
- Final workspace gate for G003: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G003 blocker remains; listed follow-ups are runtime integration boundaries rather than crate-readiness blockers.
