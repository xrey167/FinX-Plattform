# tdw-auth Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-auth\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: serde-only auth contract remains minimal.
- [x] Dependency direction reviewed: no local dependencies; consumed by service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: bool authorize compatibility is retained and authorize_with_decision exposes deny reasons.
- [x] Runtime behavior reviewed: empty subjects, unsafe table names, empty required roles, invalid roles, and missing roles deny explicitly.
- [x] Tests and coverage evidence recorded: tests cover allowed/denied paths and adversarial subject/table/role inputs.
- [x] Docs and examples reviewed: worksheet records the policy contract; no README/examples required.
- [x] Surface wiring reviewed: service API can keep bool authorize while richer decisions are available.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: policy table and role strings are validated before authorization succeeds.

## Findings

- Authorization remains role-based and intentionally small for bootstrap.
- Deny reasons now make auth failures auditable without changing existing service callers.
- Follow-up boundary: tenant/row-filter enforcement belongs in the query/service layer.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-auth; follow-ups are service-layer row and tenant policy enforcement.

## Policy Binding Evidence (G015)

`tdw-service-api` now uses `authorize_with_decision` in its secure request-path
wrapper. Missing roles deny before endpoint execution, and deny reasons are
included in the service error path for auditability.
