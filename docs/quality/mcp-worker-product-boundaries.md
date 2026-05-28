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
- `tdw-worker` has a typed `WorkerJob`, `WorkerLease`,
  `DeadLetterRecord`, `WorkerQueueStats`, and `DurableWorkerQueue` interface.
  It includes both an in-memory contract backend and a SQLite-backed durable
  scheduler. The durable backend persists `OpEnvelope` jobs, leases by
  priority, reaps expired leases, tracks retry attempts, dead-letters exhausted
  jobs, and treats duplicate enqueue / repeated completion as idempotent.
- `tdw-worker --durable-smoke` exercises the SQLite scheduler end to end with
  enqueue -> lease -> complete -> stats.
- The existing smoke modes remain intact for fast offline verification.

## Deliberately not shipped here

- Streamable HTTP MCP transport is not shipped yet. The current MCP server
  implements its advertised stdio capabilities but does not expose the
  2025-06-18 HTTP endpoint.
- Worker scheduling is durable for the local/embedded SQLite backend. A
  RiverQueue/Postgres-backed distributed scheduler is still a follow-up for
  deployments that need multi-process lease recovery across machines.
- The MCP and worker processes do not yet call the daemon transport as real
  clients. MCP `tools/call` currently routes through deterministic service-api
  functions; daemon-backed MCP calls should happen after the daemon TCP/auth
  surface is stable.

## Follow-up implementation path

1. Add Streamable HTTP MCP transport with localhost binding, Origin validation,
   and authentication.
2. Route MCP `tools/call` through the daemon client boundary where the tool
   semantics require live daemon state.
3. Add a Postgres/RiverQueue worker backend that mirrors the SQLite scheduler
   contract for distributed deployments.
4. Add always-on protocol tests for framing and env-gated integration tests for
   daemon-backed MCP and durable worker execution.
