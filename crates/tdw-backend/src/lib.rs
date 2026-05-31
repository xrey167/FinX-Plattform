#![forbid(unsafe_code)]
//! tdw-backend — unified embeddable backend facade.
//!
//! Pure composition + re-export over the underlying `tdw-*` crates: it wires the
//! data/daemon surface and the agent/MCP surface behind two facades and a shared
//! prelude. This crate holds **no business logic** — every method is a thin,
//! typed delegation to a `tdw-*` crate.
//!
//! # The dual-facade contract
//!
//! The backend exposes **two** facades with deliberately different shapes:
//!
//! * [`data::Backend`] — the **async** data/daemon facade. It owns the daemon
//!   composition root ([`AppState`](tdw_service_api::AppState)) and the provider
//!   runner, and serves the in-process daemon via
//!   [`Backend::serve`](data::Backend::serve). Its methods are `async`.
//! * [`agent::AgentBackend`] — the **sync** agent/MCP facade. It composes the
//!   `tdw-agent` registry, the tool executor, an embedded
//!   [`McpServer`](tdw_mcp::McpServer), the agent store, and the synchronous
//!   knowledge graph / tags / features / hooks. Its methods are blocking.
//!
//! ## The agent side reaches the data side as a daemon **client**
//!
//! The two facades are **not** stitched together through a shared `Arc`. The
//! sync agent/MCP surface has no tokio runtime and must never touch the async
//! `Backend` directly. Instead, the agent side reaches the data side exactly as
//! any external client would: as a [`DaemonClient`](tdw_app_client::DaemonClient)
//! over a **shared loopback TCP address**.
//!
//! The wiring is:
//!
//! 1. [`Backend::serve`](data::Backend::serve) binds the daemon transport and
//!    exposes the OS-assigned address via
//!    [`Backend::bound_addr`](data::Backend::bound_addr).
//! 2. [`AgentBackend::with_daemon_addr`](agent::AgentBackend::with_daemon_addr)
//!    points the embedded [`McpServer`](tdw_mcp::McpServer) at that address, so
//!    its daemon-backed tools (e.g. `tdw.daemon.query.submit`) submit ops over
//!    loopback and observe the same event spine the daemon emits.
//!
//! This is the same mechanism the serving binary uses for
//! [`Surfaces::Both`](config::Surfaces::Both) (see [`server::run`]); the library
//! facade just makes it callable without the binary.
//!
//! # Capability groups
//!
//! The surface is organized into four capability groups, all reachable from the
//! [`prelude`]:
//!
//! * **Data** — async query/ingest/fetch/stream + the daemon lifecycle on
//!   [`Backend`](data::Backend).
//! * **Agent / MCP** — registry hot-reload, tool listing/dispatch, single-line
//!   MCP JSON-RPC, workflow compilation, eval runs, and agent/workflow CRUD on
//!   [`AgentBackend`](agent::AgentBackend).
//! * **Knowledge** — the sync knowledge graph / tag taxonomy / feature store on
//!   [`AgentBackend`](agent::AgentBackend), plus the async knowledge index on
//!   [`Backend`](data::Backend).
//! * **Auth / hooks / policy / events** — policy enforcement and the event-spine
//!   accessors on [`Backend`](data::Backend), plus the hook registry on
//!   [`AgentBackend`](agent::AgentBackend).
//!
//! # Prelude usage
//!
//! ```no_run
//! use tdw_backend::prelude::*;
//!
//! # async fn run() -> BackendResult<()> {
//! // Async data side: serve the daemon in-process on an ephemeral loopback port.
//! let mut cfg = BackendConfig::default();
//! cfg.tdw.daemon.transport = tdw_config::DaemonTransport::Tcp;
//! cfg.tdw.daemon.tcp_bind = Some("127.0.0.1:0".to_string());
//!
//! let mut backend = Backend::from_config(cfg.tdw.clone()).await?;
//! backend.serve(&cfg).await?;
//! let addr = backend.bound_addr().expect("served daemon exposes its address").to_string();
//!
//! // Sync agent side: reach the same daemon over loopback, never a shared Arc.
//! let mut agent = AgentBackend::from_config(&cfg)?.with_daemon_addr(&addr);
//! let _responses = agent.handle_mcp_line(
//!     r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"c","version":"1.0.0"}}}"#,
//! );
//!
//! backend.shutdown().await?;
//! # Ok(())
//! # }
//! ```

pub mod agent;
pub mod auth;
pub mod config;
pub mod data;
pub mod error;
pub mod prelude;
pub mod server;
