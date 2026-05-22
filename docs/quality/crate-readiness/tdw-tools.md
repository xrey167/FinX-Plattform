# tdw-tools Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-tools\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-hooks, tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 4
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 8 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: hooks/protocol dependencies match tool permission and call-id contracts.
- [x] Dependency direction reviewed: depends on hooks/protocol and feeds service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: duplicate, unknown, invalid definition, permission-denied, ask, and allow paths are explicit.
- [x] Runtime behavior reviewed: tool registration validates names, descriptions, and permission patterns before routing.
- [x] Tests and coverage evidence recorded: tests cover duplicate rejection, allowed execution, ask deferral, and unsafe definition rejection.
- [x] Docs and examples reviewed: worksheet records the orchestration boundary; no README/examples required.
- [x] Surface wiring reviewed: service API uses echo tool sample and orchestrator.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are test-only panic/expect helpers and generated permission-id assert.
- [x] Security and reliability risks reviewed: invalid tool names and permission patterns cannot enter the registry.

## Findings

- The crate is a synchronous in-process tool registry/orchestrator, not a remote execution engine.
- Permission evaluation remains delegated to tdw-hooks and now combines with registry validation.
- Follow-up boundary: durable permission IDs and approval lifecycle integration belong in service/session runtime.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-tools; follow-ups are durable permission orchestration.
