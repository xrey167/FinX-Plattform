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
- `tools/call` also exposes a narrow daemon-backed path for live daemon state:
  `tdw.daemon.triage` and `tdw.daemon.query.submit` build `OpEnvelope`
  `RunQuery` operations, submit them through `tdw-app-client` to the configured
  daemon endpoint, wait for the terminal `EventMsg`, and return the daemon
  event evidence as structured MCP output. TCP, UDS on Unix, and plain HTTP/SSE
  daemon endpoints are supported by the app-client boundary; HTTPS daemon URLs
  fail closed. The default daemon target is
  `127.0.0.1:7878`; `TDW_CONFIG`, `TDW_CONFIG_CONTENT`,
  `TDW_MCP_DAEMON_TRANSPORT`, `TDW_MCP_DAEMON_ADDR`, and
  `TDW_MCP_DAEMON_TIMEOUT_MS` can override it. If the daemon is unavailable or
  the configured transport is unsupported, these tools fail closed with
  `isError: true`; deterministic fixture tools continue to run offline.
- `tdw.progress.sample` emits `notifications/progress` before the final tool
  response when clients supply `_meta.progressToken`.
- MCP resources expose safe TDW status/config/readiness documents; prompts
  expose finance-specific equity research, daemon triage, and ingest planning
  templates.
- `tdw-mcp --streamable-http [bind]` exposes the same MCP server over a local
  Streamable HTTP endpoint at `/mcp`. It defaults to `127.0.0.1:8788`, accepts
  JSON-RPC POST bodies, returns `application/json` or `text/event-stream`
  depending on `Accept`, returns `202 Accepted` for fire-and-forget
  notifications, validates `Origin` against localhost loopback origins, checks
  `MCP-Protocol-Version`, bounds HTTP header/body sizes, and supports optional
  bearer authentication through `TDW_MCP_HTTP_TOKEN`.
- `tdw-mcp --streamable-http-smoke` exercises initialize plus an SSE
  progress-emitting tool call without opening a long-running listener.
- `tdw-worker --contract` prints the durable worker queue contract shape.
- `tdw-worker` has a typed `WorkerJob`, `WorkerLease`,
  `DeadLetterRecord`, `WorkerQueueStats`, and `DurableWorkerQueue` interface.
  It includes an in-memory contract backend, a SQLite-backed durable scheduler,
  and a Postgres-backed distributed scheduler behind `--features postgres`.
  The durable backends persist `OpEnvelope` jobs, lease by priority, reap
  expired leases, track retry attempts, dead-letter exhausted jobs, expose
  stats, and treat duplicate enqueue / repeated completion as idempotent.
- `tdw-worker --durable-smoke` exercises the SQLite scheduler end to end with
  enqueue -> lease -> complete -> stats.
- The existing smoke modes remain intact for fast offline verification.
- Always-on app-client tests cover daemon length-delimited framing, malformed
  frame rejection, terminal-event matching, and HTTP/SSE submit-path derivation.
- Env-gated integration coverage now exists for daemon-backed MCP execution
  (`TDW_MCP_DAEMON_INTEGRATION_ADDR`) and durable Postgres worker execution
  (`TDW_POSTGRES_TEST_URL`).

## Deliberately not shipped here

- The MCP HTTP transport is intentionally local-first. Remote deployment still
  needs a deployment-level TLS/reverse-proxy/OAuth story; direct non-loopback
  binding is refused unless `TDW_MCP_HTTP_TOKEN` is set.
- The Postgres worker backend is a scheduler primitive, not a full production
  worker process supervisor. Operational rollout still needs deployment wiring,
  process ownership, and monitoring around leases/dead letters.

## Follow-up implementation path

1. ✅ Deployment-level TLS/reverse-proxy/OAuth guidance for remote MCP HTTP
   exposure: [`docs/release/mcp-remote-deployment.md`](../release/mcp-remote-deployment.md).
2. ✅ Supervised worker process that does real work: `tdw-worker --serve` /
   `--serve-once` run a `WorkerRunner` lease loop over the durable queue
   (graceful Ctrl-C drain, retry/dead-letter wiring), and `DaemonJobHandler`
   submits each leased job's `OpEnvelope` to the configured daemon via
   `tdw-app-client` (selected by `TDW_WORKER_DISPATCH=daemon` /
   `TDW_WORKER_DAEMON_*`; `LoggingAckHandler` remains the offline default).
   Operating guide: [`docs/release/worker-deployment.md`](../release/worker-deployment.md).
   Remaining: concurrent in-flight jobs and a Postgres-backed `--serve` mode
   (the loop is already generic over both backends via `ServeQueue`).
3. Promote the live daemon integration recipe into a dedicated CI job when a
   long-running daemon service is available in the target environment.
