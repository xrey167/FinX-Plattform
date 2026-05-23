# tdw-core Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-core\Cargo.toml
- Target kinds: lib, test
- Local dependencies: none
- External dependencies: async-trait ^0.1.89; bytes ^1.11.0; futures-core ^0.3.31; inventory ^0.3.21; schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-knowledge, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-runtime, tdw-service-api, tdw-storage-clickhouse, tdw-storage-fs, tdw-storage-meilisearch, tdw-storage-postgres, tdw-storage-qdrant, tdw-storage-router, tdw-storage-s3
- Feature flags: default; inventory-registration=[dep:inventory]
- Test attributes detected: 7
- tests/ directory: yes
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 9 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies are tdw-knowledge, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-runtime, tdw-service-api, tdw-storage-clickhouse, tdw-storage-fs, tdw-storage-meilisearch, tdw-storage-postgres, tdw-storage-qdrant, tdw-storage-router, tdw-storage-s3.
- [x] Feature flags reviewed: default; inventory-registration=[dep:inventory].
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 7 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- Provider, stream, storage, registry, progress, and envelope contracts are typed and covered by unit and golden tests.
- No code change required in G002; existing duplicate-registration, inventory-feature, and OBBject serialization tests were sufficient for this contract layer.
- Follow-up boundary: Provider-specific and storage-specific behavior remains owned by G003 and G004.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.


## Smoke Evidence (G009)

Participates in the [end-to-end functional smoke](../end-to-end-smoke.md). The smoke composition is exercised by:

- `tdw-test-utils::smoke::run_end_to_end_smoke` (library entry)
- `crates/tdw-test-utils/tests/end_to_end_smoke.rs` (integration tests)
- `tdw-service` and `tdw-cli` binaries (programmatic harness output)

Verified with `cargo test -p tdw-test-utils --test end_to_end_smoke` — green.
