# tdw-agent Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-agent\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18; validator ^0.20.0 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-agent-store, tdw-eval-runner, tdw-service-api, tdw-workflow-engine, xtask
- Feature flags: none
- Test attributes detected: 6
- tests/ directory: yes
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 7 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: schema/serde/validator/thiserror dependencies match the agent contract surface.
- [x] Dependency direction reviewed: foundation crate with no local dependencies and expected reverse consumers.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: manifest parsing, slash command parsing, agent-card validation, workflow validation, and storage mapping contracts are explicit.
- [x] Runtime behavior reviewed: parser and validator paths reject unsafe identifiers and unsupported endpoint URI schemes before runtime use.
- [x] Tests and coverage evidence recorded: tests cover schema bundle, A2A card round trip, manifest parsing, slash parsing, invalid endpoint/command rejection, and DAG validation.
- [x] Docs and examples reviewed: worksheet plus golden agent-card fixture cover the crate contract; no README/examples required yet.
- [x] Surface wiring reviewed: consumed by agent store, eval runner, service API, workflow engine, and xtask schema export.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are test-only panic helpers or schema serialization asserts.
- [x] Security and reliability risks reviewed: identifier, URI, endpoint, and workflow-DAG validation now guard the agent boundary.

## Findings

- Agent schemas and golden fixture are stable enough for bootstrap service wiring.
- New contract validators reject path-like slash commands, invalid agent/workflow identifiers, unsafe content refs, and non-MCP/HTTPS endpoints.
- Follow-up boundary: service/runtime crates should use the checked validators on all external request paths.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-agent; follow-ups are runtime adoption of checked validation paths.
