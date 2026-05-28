# tdw-app-client Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-app-client\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-app-server, tdw-protocol
- External dependencies: serde ^1.0.228 features=[derive], serde_json
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 3
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

- Added `AppClient::try_new` and `validate_client_info` so client identity and daemon endpoint validation can fail before enqueueing protocol ops.
- Added `DaemonClient` and `DaemonClientConfig` for blocking TCP daemon
  submission using the same length-delimited `OpEnvelope` / `EventMsg` frame
  contract as `tdw-app-server`. The client defaults to local TCP
  `127.0.0.1:7878`, validates endpoint shape, bounds frames, waits for the
  submitted op's terminal event, and fails closed on unsupported transports or
  unavailable daemons.
- Runtime behavior remains thin by design: submission is delegated to `tdw-app-server::SubmissionHandle`, with no copied service business logic.
- Scan signals are test `expect` calls only; no stub, copied FinX-XR, OpenBB, or fallback runtime branch was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.

## Verdict

Ready with follow-ups. The app client is a validated thin submission wrapper
and TCP daemon submitter; production retry/backoff policy and non-TCP daemon
client transports belong above or beside this crate.
