# tdw-exec Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-exec\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
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

- Added `try_run_headless` and SQL validation so checked headless execution rejects empty, multi-statement, mutating, or unsafe-token query ops.
- Existing unchecked `run_headless` remains available for deterministic sample construction, while service integration now exercises the checked path.
- Scan signals are test `expect` calls and SQL helper code; no stub, copied FinX-XR, OpenBB, or duplicate runtime engine was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. Headless exec emits protocol events and has a checked read-only boundary; full SQL planning remains outside this crate.
