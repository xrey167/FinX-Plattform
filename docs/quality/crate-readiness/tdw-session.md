# tdw-session Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-session\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-hooks, tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; sqlx ^0.9.0 features=[runtime-tokio,tls-rustls,sqlite,sqlite-bundled,macros]; thiserror ^2.0.18; tokio ^1.52.3 kind=dev features=[macros,rt-multi-thread,sync]
- Dev dependencies: tokio
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 19 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are tdw-hooks, tdw-protocol; reverse dependencies are none.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 2 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- SQLite session store migrates session, permission, approval, and cost-ledger tables with typed corruption errors.
- Added cost_entries read path and test coverage so cost ledger entries are auditable, not append-only.
- Follow-up boundary: Multi-connection durability and migration versioning can be expanded when runtime ownership is introduced.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.

