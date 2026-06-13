#![forbid(unsafe_code)]
//! `tdw-backend` — the unified serving binary.
//!
//! Thin CLI: parse flags + environment into a
//! [`BackendConfig`](tdw_backend::config::BackendConfig), then hand off to
//! [`tdw_backend::server::run`]. All composition/threading lives in the library.
//!
//! Flags (flags win over the corresponding environment variables):
//! * `--daemon-only` — serve only the data/daemon surface.
//! * `--mcp-only` — serve only the agent/MCP surface.
//! * `--mcp-stdio` — embedded MCP speaks line-delimited JSON-RPC over stdio.
//! * `--mcp-http <bind>` — embedded MCP speaks Streamable HTTP on `<bind>`.
//!
//! Environment (consulted when the matching flag is absent):
//! * `TDW_BACKEND_SURFACES` — `daemon-only` | `mcp-only` | `both` (default `both`).
//! * `TDW_BACKEND_MCP_TRANSPORT` / `TDW_BACKEND_MCP_HTTP_BIND` — MCP transport.
//! * `TDW_CONFIG` / `TDW_DAEMON_TCP_BIND` / `TDW_PROFILE` — daemon config layers.

use tdw_backend::config::{BackendConfig, McpTransport, Surfaces};
use tdw_backend::server;

type MainError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), MainError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = BackendConfig::load().await?;
    apply_flags(&mut cfg, &args)?;
    Box::pin(server::run(cfg)).await?;
    Ok(())
}

/// Apply CLI flag overrides on top of the env-derived `cfg`. Flags take
/// precedence over the environment.
fn apply_flags(cfg: &mut BackendConfig, args: &[String]) -> Result<(), MainError> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--daemon-only" => cfg.surfaces = Surfaces::DaemonOnly,
            "--mcp-only" => cfg.surfaces = Surfaces::McpOnly,
            "--mcp-stdio" => cfg.mcp_transport = McpTransport::Stdio,
            "--mcp-http" => {
                let bind = iter
                    .next()
                    .ok_or("--mcp-http requires a <bind> argument (e.g. 127.0.0.1:8788)")?;
                cfg.mcp_transport = McpTransport::Http(bind.clone());
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }
    Ok(())
}
