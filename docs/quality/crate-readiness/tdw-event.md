# tdw-event Readiness Worksheet

Owner tranche: G002-core-contracts-event-session-and-replay-crates - Core Contracts, Event, Session, and Replay Crates.

## Baseline Inventory

- Manifest: crates\tdw-event\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145; validator ^0.20.0 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-actor, tdw-bus, tdw-cdc, tdw-hooks, tdw-outbox, tdw-service-api, xtask
- Feature flags: none
- Test attributes detected: 3
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: package uses workspace lints, publish=false, edition 2024, and expected workspace dependencies.
- [x] Dependency direction reviewed: local dependencies are none; reverse dependencies are tdw-actor, tdw-bus, tdw-cdc, tdw-hooks, tdw-outbox, tdw-service-api, xtask.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed for the crate role.
- [x] Runtime behavior reviewed for in-memory, JSONL, SQLite, protocol, or schema responsibilities as applicable.
- [x] Tests and coverage evidence recorded: 3 test attributes detected plus focused and workspace verification commands.
- [x] Docs and examples reviewed: no per-crate README/examples required for this foundational crate when higher-level docs and schema artifacts cover the contract.
- [x] Surface wiring reviewed: service-api and xtask usage were checked where applicable via rg evidence.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test assertions, sample helpers, defaults with explicit policy, or tracked follow-ups; no bootstrap stubs found in this tranche.
- [x] Security and reliability risks reviewed for ID validation, retention loss, persistence corruption, and auditability boundaries.

## Findings

- Actor, origin, trace, event envelope, schema references, child causation, and schema export are typed and tested.
- No code change required in G002; sample helpers are explicit fixtures and not hidden fallbacks.
- Follow-up boundary: Production event ID generation policy can be tightened when persistent event stores own IDs.

## Verification

- Focused patched-crate check passed: cargo test -p tdw-bus -p tdw-outbox -p tdw-session -p tdw-actor.
- G002 focused tranche check: cargo test -p tdw-core -p tdw-domain -p tdw-protocol -p tdw-config -p tdw-event -p tdw-actor -p tdw-bus -p tdw-cdc -p tdw-outbox -p tdw-snapshot -p tdw-replay -p tdw-rollout -p tdw-session.
- Workspace gate passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G002 blocker remains; listed follow-ups are assigned to later tranche responsibilities or future production runtime/storage implementations.

