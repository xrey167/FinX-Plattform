# tdw-app-server Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-app-server\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; tokio ^1.52.3 features=[macros,rt-multi-thread,sync]
- Dev dependencies: none
- Reverse local dependencies: tdw-app-client
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 3 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- Added daemon endpoint validation for UDS and HTTP/SSE transports, rejecting empty, traversal, control-character, shell-control, and wrong-scheme addresses.
- Existing queue behavior is covered by `AgentLoop::run_once`, which consumes submitted protocol envelopes and emits `EventMsg::Started`.
- Scan signals are test `expect` calls; no stub, copied FinX-XR, OpenBB, or duplicate service implementation was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. The daemon sample has typed endpoint contracts and queue-event tests; durable production queueing remains a future service concern.
