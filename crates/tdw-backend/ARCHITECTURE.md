# tdw-backend architecture

`tdw-backend` is the unified composition layer: two facades over the `tdw-*`
crates plus the shared serving glue that both the standalone `tdw-service` daemon
and the unified `tdw-backend` binary call into.

## Module map

| Path | Contents |
| --- | --- |
| `src/lib.rs` | crate docs + the four capability groups; module re-exports |
| `src/prelude.rs` | `use tdw_backend::prelude::*;` — the whole typed surface |
| `src/config.rs` | `BackendConfig`, `Surfaces`, `McpTransport`, env parsing |
| `src/data/mod.rs` | the **async** `Backend` data/daemon facade |
| `src/agent/mod.rs` | the **sync** `AgentBackend` agent/MCP facade |
| `src/auth/mod.rs` | the auth/hooks/policy/events re-export group |
| `src/server/mod.rs` | the serving glue: `load_config`, `run_daemon`, `run`, transports |
| `src/bin/tdw-backend.rs` | the unified binary (flags → `BackendConfig` → `server::run`) |

## The dual-facade contract

- `Backend` (async) owns `AppState` (the daemon composition root) and the
  provider runner. Methods: `from_config`/`in_memory_for_tests`, engine accessors,
  `enforce_policy`, `dispatch`, `serve`/`bound_addr`/`submission_handle`/
  `shutdown`, `fetch`/`stream`/`start_binance_stream`/`stop_stream`,
  `knowledge_index`/`knowledge_search`, and the memory consolidation surface
  (`upsert_memory`/`list_memories`/`consolidate_now`).
- `AgentBackend` (sync) composes the `tdw-agent` registry, the `ToolExecutor`, an
  embedded `tdw-mcp` `McpServer`, the agent store, and the sync KG/tags/features/
  hooks. Methods: `from_config`, `with_daemon_addr`, `list_tools`,
  `handle_mcp_line`, plus registry/workflow/eval/knowledge operations.

The facades are **not** joined by a shared `Arc`. The sync side has no tokio
runtime and must never touch the async `Backend` directly; instead
`AgentBackend::with_daemon_addr(addr)` points the embedded MCP server's loopback
`DaemonClient` at the address `Backend::serve` exposed via `bound_addr`. The
agent side thus reaches the data side exactly as any external client would.

## Serving glue (`server`)

- `load_config()` — resolve the layered `TdwConfig` (honours `TDW_CONFIG`,
  `TDW_DAEMON_TCP_BIND`, `TDW_PROFILE`); applies in-memory session + temp rollout
  overrides. `resolve_profile` is the pure precedence helper.
- `run_daemon(&config)` — build `AppState`, report policy state, **refuse to start
  on a partial OIDC config**, warn on an unauthenticated non-loopback bind, wire
  `service_channel` + relay + transport, then `serve` until ctrl-c / shutdown;
  optionally runs the memory consolidation scheduler when `TDW_MEMORY_DIR` is set.
- `spawn_transport(&config, …)` — bind and spawn TCP / UDS / HTTP-SSE (ephemeral
  ports resolve via `local_addr`); fail-closed when a transport feature is absent.
- `run(BackendConfig)` — surface-aware entrypoint:
  - `DaemonOnly` → `run_daemon`.
  - `McpOnly` → the blocking MCP loop on the current thread.
  - `Both` → `Backend::serve` on the tokio runtime, then the blocking MCP loop on
    a dedicated `tdw-backend-mcp` OS thread pointed at the daemon's loopback
    address; on MCP exit the daemon is signalled and shut down.

## Runtime flow (Surfaces::Both)

```text
server::run(cfg)  [Both]
   Backend::from_config ─▶ Backend::serve (bind loopback TCP, OS port)
        │ bound_addr
        ▼
   spawn OS thread "tdw-backend-mcp"
        run_mcp_loop(transport, Some(daemon_addr))
           tdw-mcp McpServer.with_daemon_config(loopback DaemonClient)
           daemon-backed tools submit ops ──loopback──▶ in-process daemon
        ▼ (MCP loop ends: stdio EOF / HTTP close)
   Backend::shutdown  (cancel daemon, reclaim tasks)
```

## Security posture

- **Loopback default + warning (post-#150)**: the daemon binds `127.0.0.1:7878`
  by default; a non-loopback bind with no auth-backed policy emits a prominent
  `SECURITY WARNING` (`warn_on_unauthenticated_nonloopback_bind`).
- **OIDC fail-closed**: a *partial* `TDW_OIDC_*` config makes `run_daemon` refuse
  to start with the list of missing variables; a fully-unset prod config runs
  fail-closed; non-prod profiles attach a local-dev policy.
- **No `set_var`**: the embedded MCP loop is pointed at the daemon via an explicit
  `DaemonClientConfig` threaded through `run_*_with_daemon`, never by mutating the
  environment (required under `forbid(unsafe_code)` on Rust 2024).
- **Loopback-only cross-facade link**: the agent surface reaches data only over a
  loopback `DaemonClient`, so the security boundary is the same one external
  clients cross.
- Default build is offline: deterministic hash embedder, in-memory engines.

## Integration points

- `tdw-service-api` — `AppState` (the data facade's core) + the dispatch path.
- `tdw-app-server` / `tdw-app-client` — transports + the loopback daemon client.
- `tdw-mcp` — the embedded MCP server (`run_*_with_daemon`).
- `tdw-agent` / `tdw-agent-store` / `tdw-tool-exec` / `tdw-workflow-engine` /
  `tdw-eval-runner` — the agent surface.
- `tdw-kg` / `tdw-tags` / `tdw-feature-store` / `tdw-knowledge` — the knowledge
  group. `tdw-service` calls back into `server::{load_config, run_daemon}`.
