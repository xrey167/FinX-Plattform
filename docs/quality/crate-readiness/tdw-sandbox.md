# tdw-sandbox Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-sandbox\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-udf
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 3
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: tdw-udf plus serde/thiserror matches sandbox request/response contracts.
- [x] Dependency direction reviewed: depends only on tdw-udf and feeds service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: capability denied, invalid request, and UDF failure paths are explicit.
- [x] Runtime behavior reviewed: local sandbox validates request name/source/input size before evaluating UDF definitions.
- [x] Tests and coverage evidence recorded: tests cover successful local execution, denied network capability, and invalid empty source rejection.
- [x] Docs and examples reviewed: worksheet records sandbox behavior; no README/examples required.
- [x] Surface wiring reviewed: service API calls LocalUdfSandbox through SandboxRuntime.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signal is a test-only panic helper.
- [x] Security and reliability risks reviewed: sandbox request validation preserves UDF capability denial and input-size limits.

## Findings

- LocalUdfSandbox is a bootstrap dispatcher over tdw-udf, not a process/VM isolation boundary.
- Request validation now catches malformed definitions before dispatch.
- Follow-up boundary: OS/process/Wasm isolation, CPU/memory metering, and runtime-specific adapters belong in later sandbox runtime work.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-sandbox; follow-ups are hardened runtime isolation.

## Policy Binding Evidence (G015)

`tdw-service-api::secure_udf_run` now dispatches UDF work through
`SandboxRuntime` only after ingress JWT validation and role authorization.
Focused tests prove a network-capability UDF request is denied through the
secure service path.
