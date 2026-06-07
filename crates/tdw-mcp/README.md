# tdw-mcp

The TDW Model Context Protocol (MCP) server. Speaks MCP JSON-RPC over two
transports — line-delimited **stdio** and **Streamable HTTP** (`POST/GET /mcp`) —
and exposes TDW's deterministic offline tools plus explicitly daemon-backed query
and triage tools. A `tdw-agent` registry can be attached so its `tool` resources
appear in `tools/list`.

MCP protocol version: `2025-06-18`. The crate is `#![forbid(unsafe_code)]`. The
HTTP transport is hardened: loopback-only by default (a non-loopback bind requires
`TDW_MCP_HTTP_TOKEN`), Origin allow-listing, optional bearer-token auth with
constant-time comparison, and bounded header/body sizes.

## Binaries produced

- **`tdw-mcp`** — the MCP server. Flags:
  - `--stdio-json-rpc` — line-delimited JSON-RPC over stdin/stdout.
  - `--streamable-http [BIND]` (`--http`) — Streamable HTTP on `BIND`
    (default `127.0.0.1:8788`).
  - `--streamable-http-smoke` (`--http-smoke`) — offline self-check.
  - no flag — prints a deterministic evidence summary.

## Feature flags

None.

## Key environment variables

(Full reference in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).)

- Daemon endpoint for daemon-backed tools: `TDW_MCP_DAEMON_TRANSPORT`,
  `TDW_MCP_DAEMON_ADDR` (or `TDW_DAEMON_TCP_BIND`), `TDW_MCP_DAEMON_TIMEOUT_MS`;
  `TDW_CONFIG` / `TDW_CONFIG_CONTENT` resolve the daemon config layers. Default
  endpoint: loopback TCP `127.0.0.1:7878`.
- HTTP auth: `TDW_MCP_HTTP_TOKEN` — required to bind a non-loopback address;
  enables bearer-token auth when set.
- Registry: `TDW_AGENT_REGISTRY_DIR` — directory of `tdw-agent` tool definitions
  to attach to `tools/list`.

## Quickstart (binary)

```bash
# Offline self-check (no daemon, no network):
cargo run -p tdw-mcp -- --streamable-http-smoke

# stdio JSON-RPC (editor/agent host pipes JSON-RPC lines on stdin):
cargo run -p tdw-mcp -- --stdio-json-rpc

# Streamable HTTP on loopback:
cargo run -p tdw-mcp -- --streamable-http 127.0.0.1:8788
```

A client sends `initialize`, then `tools/list` / `tools/call`. Daemon-backed
tools (`tdw.daemon.query.submit`, `tdw.daemon.triage`) require a running daemon.

See [`examples/basic.rs`](examples/basic.rs) for an offline in-process JSON-RPC
session: `cargo run -p tdw-mcp --example tdw_mcp_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — transports, tool catalog, security.
- `tdw-app-client` — the daemon client the daemon-backed tools use.
- `tdw-backend` — embeds this server pointed at the in-process daemon.
