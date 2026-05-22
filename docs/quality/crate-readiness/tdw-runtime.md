# tdw-runtime Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-runtime\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core
- External dependencies: async-trait ^0.1.89 kind=dev; bytes ^1.11.0 kind=dev; futures-core ^0.3.31; schemars ^1.2.1 kind=dev; serde ^1.0.228 kind=dev features=[derive]; serde_json ^1.0.145
- Dev dependencies: async-trait, bytes, schemars, serde
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

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

- `CommandRunner` remains the shared provider runtime adapter, preserving `tdw-core` registry and credential boundaries.
- Streaming wrapper emits deterministic start/progress/done events around terminal fetch results and is covered by focused tests.
- Scan signals are test fixture/mock names and test panic assertions; no stub, copied FinX-XR, OpenBB, or duplicate provider runtime was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. Runtime orchestration is delegated to provider traits and streaming wrappers; production scheduling belongs above this crate.
