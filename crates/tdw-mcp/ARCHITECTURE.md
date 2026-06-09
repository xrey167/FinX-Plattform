# tdw-mcp architecture

`tdw-mcp` implements an MCP (Model Context Protocol) server over two transports
and exposes TDW's tools/resources/prompts. It is stateless per request apart from
the per-connection `McpServer` (initialization + cancellation tracking + the
attached registry).

## Module map

| Path | Contents |
| --- | --- |
| `src/lib.rs` | `McpServer`, JSON-RPC dispatch, the tool catalog, the daemon-tool runtime, both transports, security helpers |
| `src/main.rs` | the `tdw-mcp` binary: flag → transport selection |

## Key types

### `McpServer`

Per-connection state: `initialized`, `client_info`, `cancelled_requests`
(bounded ring), the `DaemonToolRuntime`, an optional `tdw-agent` `Registry`, its
cached `registry_descriptors` (projected once at attach, built-ins win on name
collisions), and a `tdw_tool_exec::ToolExecutor` for bound registry tools.

Builders/setters: `new`, `with_daemon_config`, `with_registry` / `set_registry`,
`with_executor`, `set_daemon_config`.

`handle_json_rpc_line(&str) -> Vec<String>` parses one JSON-RPC message and
returns the encoded response(s). Methods handled: `initialize`, `ping`,
`tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`,
`prompts/get`, plus the `notifications/initialized` and `notifications/cancelled`
notifications. Requests before `initialize` (except `initialize`/`ping`) are
rejected `-32002`.

### Tool catalog

Built-in tools (`tool_descriptors()`): deterministic offline samples —
`tdw.providers.list`, `tdw.equity.historical`, `tdw.progress.sample`,
`tdw.agent.sample`, `tdw.extensibility.sample`, `tdw.event_spine.sample`,
`tdw.kg_tag.sample`, `tdw.client_event.sample` — and two **daemon-backed** tools:
`tdw.daemon.triage` and `tdw.daemon.query.submit`. Registry tools (from an
attached `tdw-agent` registry) are appended to `tools/list`; built-ins win on
name collision, and registry tools execute through the `ToolExecutor` (or report
`-32601 not yet executable` when unbound).

### Daemon tool runtime

`DaemonToolRuntime` resolves a `DaemonClientConfig` from `TdwConfig` +
`TDW_MCP_DAEMON_*` env and submits an `OpEnvelope` via
`tdw_app_client::DaemonClient::submit_and_wait`. This is how
`tdw.daemon.query.submit` lands a `RunQuery` op on the live daemon and observes
its terminal event.

### Transports

- **stdio**: `run_stdio_json_rpc[_with_daemon]` — read lines from stdin, write
  responses to stdout.
- **Streamable HTTP**: `run_streamable_http[_with_daemon]` — a blocking
  `TcpListener` accept loop; each connection is parsed
  (`read_streamable_http_request`) and answered by
  `handle_streamable_http_request_with_config` (`POST /mcp` JSON-RPC, `GET /mcp`
  SSE-ready, `OPTIONS` CORS preflight). `StreamableHttpRequest`/`Response` are the
  testable wire types.

## Runtime flow (MCP transports)

```text
stdio:   stdin line ─▶ handle_json_rpc_line ─▶ stdout line
http:    POST /mcp body ─▶ handle_json_rpc_line ─▶ JSON or SSE response
                              │
              tools/call "tdw.daemon.query.submit"
                              ▼
              DaemonToolRuntime.submit ─▶ DaemonClient.submit_and_wait
                              ▼
              daemon RunQuery dispatch ─▶ terminal EventMsg ─▶ tool result
```

## Security posture

- **Loopback-only HTTP by default**: a non-loopback bind is refused unless
  `TDW_MCP_HTTP_TOKEN` is set (`bind_is_loopback`).
- **Bearer auth**: when `TDW_MCP_HTTP_TOKEN` is set, requests must carry a
  matching `Authorization: Bearer` token, compared in **constant time**
  (`constant_time_str_eq` over a fixed FNV digest, via `subtle::ConstantTimeEq`)
  so neither the token value nor its length leaks through timing.
- **Origin allow-list**: only `localhost`/`127.0.0.1`/`::1` origins are accepted
  (`origin_is_allowed`); others get `403`.
- **Protocol-version pinning**: a mismatched `MCP-Protocol-Version` header is
  `400`.
- **Bounded I/O**: header (`16 KiB`) and body (`1 MiB`) caps; malformed requests
  yield typed HTTP errors, never panics.
- The hidden `__fuzz_mcp_jsonrpc` / `__fuzz_mcp_http` shims must never panic
  (nightly cargo-fuzz targets).

## Integration points

- `tdw-app-client` — the daemon client for daemon-backed tools.
- `tdw-config` — resolves the daemon endpoint from `TDW_CONFIG` + env.
- `tdw-agent` / `tdw-tool-exec` — the attachable registry and its executor.
- `tdw-service-api` — the deterministic sample data behind the offline tools.
- `tdw-backend` — embeds this server pointed at the in-process daemon's loopback
  address via `run_*_with_daemon`.
