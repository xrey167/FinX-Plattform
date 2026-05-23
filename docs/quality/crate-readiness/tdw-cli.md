# tdw-cli Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-cli\Cargo.toml
- Target kinds: bin
- Local dependencies: tdw-service-api
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 0
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

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

- Binary delegates provider fetch and client-event evidence to `tdw-service-api`; it does not carry independent business logic.
- Error handling exits non-zero on service failure and reports optional client-event sample failures to stderr.
- Scan signals are intentional sample/error-output calls; no stub, copied FinX-XR, OpenBB, or hidden fallback path was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. The CLI is a thin service API entrypoint; richer argument parsing is future product work, not missing bootstrap wiring.
