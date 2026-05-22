# tdw-define Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-define\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-hooks
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

- [x] Manifest correctness reviewed: serde plus tdw-hooks matches the DEFINE-to-hook contract.
- [x] Dependency direction reviewed: depends on hooks and feeds service API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: compile_hook compatibility remains and try_compile_hook validates event/table/hook parts.
- [x] Runtime behavior reviewed: DEFINE events now reject unsafe table names before hook compilation.
- [x] Tests and coverage evidence recorded: tests cover hook/idempotency generation and invalid table rejection.
- [x] Docs and examples reviewed: worksheet records the DEFINE boundary; no README/examples required.
- [x] Surface wiring reviewed: service API composes DEFINE events into hook names.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: event/table/hook identifiers are constrained before execution surfaces consume them.

## Findings

- DEFINE remains a small compiler from declarative event metadata to hook specs.
- The checked path now rejects path/query/control-like table identifiers.
- Follow-up boundary: service API should use try_compile_hook when user-authored DEFINE statements are accepted.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-define; follow-ups are checked path adoption by service-level DEFINE execution.
