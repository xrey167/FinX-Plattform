# MCP and Worker Product Boundaries

Date: 2026-05-28

This note scopes the daemon-adjacent MCP and worker gaps so the daemon hardening
cycle does not silently claim a broader product surface than it ships.

## Shipped in this cycle

- `tdw-mcp --stdio-json-rpc` exposes a stateful MCP 2025-06-18 stdio server
  over line-delimited JSON-RPC 2.0. It supports `initialize`,
  `notifications/initialized`, `ping`, `tools/list`, `tools/call`,
  `resources/list`, `resources/read`, `prompts/list`, `prompts/get`,
  fire-and-forget cancellation notifications, parse errors, invalid requests,
  and unknown-method errors.
- `tools/call` executes deterministic, read-only TDW service-api backed tools
  for provider discovery, equity historical fixtures, progress samples, agent
  evidence, extensibility evidence, event-spine evidence, KG/tag evidence, and
  client-event evidence. Calls return MCP content plus `structuredContent`;
  business failures return tool results with `isError: true`.
- `tdw.progress.sample` emits `notifications/progress` before the final tool
  response when clients supply `_meta.progressToken`.
- MCP resources expose safe TDW status/config/readiness documents; prompts
  expose finance-specific equity research, daemon triage, and ingest planning
  templates.
- `tdw-worker --contract` prints the durable worker queue contract shape.
- `tdw-worker` has a typed `WorkerJob`, `WorkerLease`, and
  `DurableWorkerQueue` interface plus an in-memory implementation that validates
  idempotency keys, queue names, attempts, leasing, and completion semantics.
- The existing smoke modes remain intact for fast offline verification.

## Deliberately not shipped here

- Streamable HTTP MCP transport is not shipped yet. The current MCP server
  implements its advertised stdio capabilities but does not expose the
  2025-06-18 HTTP endpoint.
- Worker scheduling is not durable yet. The in-memory queue is a contract
  harness, not a SQL/RiverQueue-backed scheduler with visibility timeouts,
  retries, dead-lettering, priority, or distributed lease recovery.
- The MCP and worker processes do not yet call the daemon transport as real
  clients. MCP `tools/call` currently routes through deterministic service-api
  functions; daemon-backed MCP calls should happen after the daemon TCP/auth
  surface is stable.

## Follow-up implementation path

1. Add Streamable HTTP MCP transport with localhost binding, Origin validation,
   and authentication.
2. Route MCP `tools/call` through the daemon client boundary where the tool
   semantics require live daemon state.
3. Move the worker queue contract into a library module when another crate needs
   it, then implement a durable backend with lease expiration, retry counters,
   dead-letter rows, and idempotent completion.
4. Add always-on protocol tests for framing and env-gated integration tests for
   daemon-backed MCP and durable worker execution.
