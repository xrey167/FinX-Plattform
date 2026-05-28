# tdw-mcp Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-mcp\Cargo.toml
- Target kinds: lib, bin
- Local dependencies: tdw-app-client, tdw-app-server, tdw-config, tdw-protocol, tdw-service-api
- External dependencies: serde, serde_json
- Dev dependencies: tokio
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 23
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

- `--stdio-json-rpc` now runs a stateful MCP 2025-06-18 stdio protocol handler
  with initialization, notification semantics, tools, resources, prompts,
  cancellation tracking, and progress notifications.
- `--streamable-http [bind]` exposes the same protocol handler over a
  Streamable HTTP `/mcp` endpoint with localhost default binding,
  loopback-only Origin validation, optional `TDW_MCP_HTTP_TOKEN` bearer auth,
  protocol-version checks, bounded HTTP input, JSON responses, SSE responses,
  and `202 Accepted` notification semantics.
- `--streamable-http-smoke` provides deterministic non-listening HTTP transport
  evidence for CI and container smoke tests.
- `tools/call` delegates deterministic read-only tool execution to
  `tdw-service-api` for provider discovery, equity historical fixtures,
  progress samples, agent evidence, extensibility evidence, event-spine
  evidence, KG/tag evidence, and client-event evidence.
- `tools/call` routes `tdw.daemon.triage` and
  `tdw.daemon.query.submit` through `tdw-app-client` to the configured TCP
  daemon endpoint, defaulting to `127.0.0.1:7878` with env/config overrides.
  These daemon-backed tools fail closed when the daemon is unavailable or a
  non-TCP transport is configured; deterministic fixture tools remain offline.
- Resources are safe static/dynamic TDW status surfaces; prompts are
  finance-specific equity research, daemon triage, and ingest-planning
  templates.
- The default binary smoke path remains intact for fast offline evidence.
- Scan signals are sample calls that prove integration surfaces; no stub,
  copied FinX-XR, OpenBB, or fallback path was found.

## Verification

- Focused MCP command passed:
  `CARGO_TARGET_DIR=target cargo test -p tdw-mcp`.
- Focused HTTP smoke passed:
  `CARGO_TARGET_DIR=target cargo run -p tdw-mcp -- --streamable-http-smoke`.

## Verdict

Ready with follow-ups. The MCP server now has stdio, local Streamable HTTP,
and narrow TCP daemon-backed tool execution. Follow-up product work is broader
daemon client transport coverage for UDS/HTTP-SSE deployments and a remote
auth/TLS deployment story.
