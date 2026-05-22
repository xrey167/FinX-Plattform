# tdw-udf-external Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-udf-external\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: no dependencies are required for the external command contract.
- [x] Dependency direction reviewed: standalone adapter contract with no reverse consumers yet.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: command name, arguments, timeout, and runtime constants are explicit.
- [x] Runtime behavior reviewed: validator rejects path commands, shell-control arguments, and unbounded timeouts.
- [x] Tests and coverage evidence recorded: tests cover safe command shape and unsafe path/shell/timeout rejection.
- [x] Docs and examples reviewed: worksheet records adapter contract; no README/examples required.
- [x] Surface wiring reviewed: no runtime consumer yet.
- [x] Scaffold, dead-code, and fallback signals classified: bootstrap stub signal removed.
- [x] Security and reliability risks reviewed: external UDF execution must be allowlisted and timeout bounded.

## Findings

- The crate now defines an external-command validation contract instead of only exposing a crate name.
- It does not execute commands; that remains a sandbox/runtime responsibility.
- Follow-up boundary: integrate with a broker that enforces executable allowlists and process isolation.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-udf-external; follow-ups are isolated process execution.
