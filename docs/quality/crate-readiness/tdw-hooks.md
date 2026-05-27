# tdw-hooks Readiness Worksheet

Owner tranche: G005-agent-auth-hooks-tools-and-udf-crates - Agent, Auth, Hooks, Tools, and UDF Crates.

## Baseline Inventory

- Manifest: crates\tdw-hooks\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-event, tdw-protocol
- External dependencies: reqwest ^0.12.24 features=[blocking]; serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-define, tdw-mask, tdw-service-api, tdw-session, tdw-tools
- Feature flags: none
- Test attributes detected: 11
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: event/protocol dependencies match hook execution and deferred approval contracts.
- [x] Dependency direction reviewed: lower-level hook crate feeds define, mask, session, tools, and service surfaces.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: invalid hook specs, recursion guard, depth guard, permission evaluation, and deferred approvals are explicit.
- [x] Runtime behavior reviewed: execution rejects unsafe commands, non-HTTPS HTTP handlers, invalid MCP/agent IDs, bad prompt paths, invalid context URIs, and invalid permission actions.
- [x] Tests and coverage evidence recorded: tests cover deterministic ordering, disabled hooks, recursion, runtime outcomes, permission precedence, deferred approvals, prompt asset, and unsafe hook rejection.
- [x] Docs and examples reviewed: worksheet and tool prompt asset cover the runtime contract; no README/examples required.
- [x] Surface wiring reviewed: service/session/tools consumers receive validated runtime outcomes and permission decisions.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are generated permission-id/test helper panics.
- [x] Security and reliability risks reviewed: hook handlers are shape-validated before execution outcomes are emitted.

## Findings

- Hooks now have a policy-gated execution API in addition to declarative runtime outcomes.
- `SystemHookHandlerBackend` executes command hooks without a shell, calls HTTPS hooks through bounded authenticated POST requests, and routes MCP hooks through registered local client handlers.
- Follow-up boundary: service transports still need to bind `execute_handlers` into their request entrypoints with deployment-specific permission rules and MCP clients.

## Verification

- Focused G005 crate check passed: cargo test -p tdw-agent -p tdw-agent-store -p tdw-auth -p tdw-auth-oidc -p tdw-define -p tdw-hooks -p tdw-mask -p tdw-tools -p tdw-sandbox -p tdw-udf -p tdw-udf-external -p tdw-udf-js -p tdw-udf-python -p tdw-udf-wasm.
- Final workspace gate for G005 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G005 blocker remains inside tdw-hooks; follow-ups are executor integration and approval persistence.

## Policy Binding Evidence (G015)

`tdw-service-api` now executes registered hook outcomes on the secure request
path and treats `should_stop` as a hard request veto. The hook crate remains
responsible for validated hook metadata.

The second G015 slice adds `HookRegistry::execute_handlers`, `HookExecutionPolicy`,
`HookExecutionOutcome`, and `SystemHookHandlerBackend`. Handler execution now
requires explicit `PermissionRules` allow entries before any backend is called.
Command hooks run via `std::process::Command` with no shell. HTTP hooks require
HTTPS metadata validation, an explicit bearer token, a bounded timeout, a
response-size cap, and cannot veto unless the policy enables handler vetoes.
MCP hooks call registered client handlers and fail closed when a server/tool is
not registered.

Verification: `cargo +stable test -p tdw-hooks -- --nocapture` passed 11/11
tests, including allowed command execution, registered MCP execution,
permission denial before backend execution, and HTTP veto denial.
