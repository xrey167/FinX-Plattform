# tdw-backend

The unified, embeddable TDW backend. It is a pure composition + re-export layer
over the underlying `tdw-*` crates, exposing two facades and a shared serving
binary:

- **`data::Backend`** — the **async** data/daemon facade. Owns the `AppState`
  composition root and the provider runner; serves the in-process daemon and
  answers query/ingest/fetch/stream ops.
- **`agent::AgentBackend`** — the **sync** agent/MCP facade. Composes the
  `tdw-agent` registry, the tool executor, an embedded `tdw-mcp` `McpServer`, the
  agent store, and the sync knowledge graph / tags / features / hooks.

The two facades are **not** stitched through a shared `Arc`. The sync agent side
reaches the async data side exactly as any external client would: as a loopback
`DaemonClient` at the daemon's bound address. The crate holds no business
logic — every method is a thin delegation. `#![forbid(unsafe_code)]`.

## Binaries produced

- **`tdw-backend`** — the unified serving binary. Flags (win over env):
  - `--daemon-only` / `--mcp-only` — serve a single surface (default: both).
  - `--mcp-stdio` / `--mcp-http <bind>` — embedded MCP transport.

## Feature flags

| Feature | Effect |
| --- | --- |
| `openai` | compile the real OpenAI HTTP embedder selector arm |
| `google` | compile the real Google/Gemini HTTP embedder selector arm |
| `transport-http` | forward `transport-http` to `tdw-app-server` |
| `transport-uds` | forward `transport-uds` (Unix domain socket) |

Default build is fully offline (deterministic hash embedder, in-memory engines).

## Key environment variables

(Full reference in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).)

- Surfaces / transport: `TDW_BACKEND_SURFACES` (`daemon-only|mcp-only|both`),
  `TDW_BACKEND_MCP_TRANSPORT` (`stdio|http`), `TDW_BACKEND_MCP_HTTP_BIND`.
- Daemon config (via the shared loader): `TDW_CONFIG`, `TDW_DAEMON_TCP_BIND`
  (default loopback `127.0.0.1:7878`), `TDW_PROFILE`.
- Engine/auth: inherited from `tdw-service-api` (`real-engines`, `TDW_OIDC_*`).
- Memory consolidation (daemon mode): `TDW_MEMORY_DIR`.

## Quickstart

Binary:

```bash
# Serve both surfaces (daemon + embedded MCP over stdio):
cargo run -p tdw-backend

# Daemon only, bound to loopback:
cargo run -p tdw-backend -- --daemon-only

# MCP only over Streamable HTTP:
cargo run -p tdw-backend -- --mcp-only --mcp-http 127.0.0.1:8788
```

Library (offline, in-process — no socket):

```rust,ignore
use tdw_backend::prelude::*;

let backend = Backend::in_memory_for_tests().await;       // async data facade
let mut agent = AgentBackend::from_config(&BackendConfig::default())?; // sync facade
let _tools = agent.list_tools();
# Ok::<(), BackendError>(())
```

Examples (all offline, run to completion):

- [`examples/basic.rs`](examples/basic.rs) — minimal in-process dual-facade demo:
  `cargo run -p tdw-backend --example tdw_backend_basic`.
- `examples/trading_consumer.rs` / `examples/learning_consumer.rs` — fuller
  loopback-served consumers proving the dual-facade contract end to end.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the dual-facade contract and serving glue.
- `tdw-service-api` — the `AppState` the data facade owns.
- `tdw-mcp` — the embedded MCP server the agent facade composes.
