# tdw-udf Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-udf\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-sandbox, tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: serde/thiserror match definition and error contracts.
- [x] Dependency direction reviewed: no local dependencies and expected sandbox/service reverse consumers.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: capability denial, invalid definition, source/input size, and unknown UDF errors are explicit.
- [x] Runtime behavior reviewed: UDF evaluation validates name, source, source size, input size, network, and filesystem flags before dispatching builtins.
- [x] Tests and coverage evidence recorded: tests cover allowed execution, denied network, invalid names/sources, and oversized input.
- [x] Docs and examples reviewed: worksheet records UDF core behavior; no README/examples required.
- [x] Surface wiring reviewed: consumed by sandbox and service API.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signal is a test-only panic helper.
- [x] Security and reliability risks reviewed: core UDF path is deny-by-capability and bounded by source/input limits.

## Findings

- The crate remains a deterministic bootstrap UDF dispatcher, not a language runtime.
- Validation now provides the shared guardrails used by LocalUdfSandbox.
- Follow-up boundary: runtime-specific interpreters/engines should call validate_definition before execution.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-udf; follow-ups are real runtime engines and resource metering.
