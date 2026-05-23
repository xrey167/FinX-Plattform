# tdw-acp Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-acp\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 4
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 6 total, 0 stub-related

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

- Added explicit ACP boundary validation for client initialization, submit-op payloads, approval resolution IDs, approval decisions, and read-only single-statement query ops.
- Public responses still wrap `tdw-protocol::EventMsg`; no duplicated event schema or alternate protocol model was introduced.
- Scan signals are test `expect` calls and query helper code; no stub, copied FinX-XR, OpenBB, or scaffold-only runtime path was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. ACP is a typed protocol edge with validation and event wrapping tests; production transport/session negotiation remains a future integration concern.
