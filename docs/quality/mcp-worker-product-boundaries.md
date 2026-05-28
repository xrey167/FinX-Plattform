# MCP and Worker Product Boundaries

Date: 2026-05-28

This note scopes the daemon-adjacent MCP and worker gaps so the daemon hardening
cycle does not silently claim a broader product surface than it ships.

## Shipped in this cycle

- `tdw-mcp --stdio-json-rpc` exposes a minimal line-delimited JSON-RPC 2.0
  boundary for smokeable stdio framing. It supports `ping`, `tools/list`, parse
  errors, invalid requests, and unknown-method errors.
- `tdw-worker --contract` prints the durable worker queue contract shape.
- `tdw-worker` has a typed `WorkerJob`, `WorkerLease`, and
  `DurableWorkerQueue` interface plus an in-memory implementation that validates
  idempotency keys, queue names, attempts, leasing, and completion semantics.
- The existing smoke modes remain intact for fast offline verification.

## Deliberately not shipped here

- Full MCP server behavior is not complete. The stdio mode does not yet
  implement MCP initialize/session negotiation, resources, prompts, tool call
  execution, cancellation, progress notifications, or HTTP transport.
- Worker scheduling is not durable yet. The in-memory queue is a contract
  harness, not a SQL/RiverQueue-backed scheduler with visibility timeouts,
  retries, dead-lettering, priority, or distributed lease recovery.
- The MCP and worker processes do not yet call the daemon transport as real
  clients. That should happen after the daemon TCP/auth surface is stable.

## Follow-up implementation path

1. Promote `tdw-mcp` stdio into a complete MCP protocol handler.
2. Route MCP `tools/call` through the daemon client boundary.
3. Move the worker queue contract into a library module when another crate needs
   it, then implement a durable backend with lease expiration, retry counters,
   dead-letter rows, and idempotent completion.
4. Add always-on protocol tests for framing and env-gated integration tests for
   daemon-backed MCP and durable worker execution.
