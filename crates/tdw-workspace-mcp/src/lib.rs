#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! The Workspace CONTROL-PLANE MCP server (openbb-ecosystem-p1 item **G006**).
//!
//! The Workspace ecosystem ships two distinct MCP surfaces: a DATA MCP (financial
//! data — served here by `tdw-mcp`) and a *control-plane* MCP that lets an agent
//! manipulate the Workspace itself — create and arrange dashboards, place and move
//! widgets, inspect widget schemas, and read back the layout. `tdw-mcp` carries
//! only the READ-ONLY widget-catalog tools (`tdw.widgets.list` / `.describe`);
//! this crate is the WRITE/control surface that was missing.
//!
//! # What it exposes
//!
//! A [`WorkspaceServer`] dispatches the `tdw.workspace.*` MCP tools against a
//! mutable [`WorkspaceState`] document:
//!
//! - `tdw.workspace.dashboard.list` / `.create` / `.delete` / `.rename`
//! - `tdw.workspace.widget.add` / `.move` / `.resize` / `.remove`
//! - `tdw.workspace.widget.schema` (inspect a widget's params schema)
//! - `tdw.workspace.layout.get` (the current `apps.json`-shaped layout)
//! - `tdw.workspace.navigate` (select the active dashboard)
//!
//! # Validation
//!
//! The document is validated against the [`tdw_widgets`] widget catalog: a placed
//! widget id must exist in the catalog, and grid placement must fit the bounded
//! 40-column grid without overlap. The [`WorkspaceState`] serializes to the
//! `apps.json` shape (via [`tdw_widgets::AppConfig`]) so the produced layout is a
//! valid Workspace app.
//!
//! # Transport
//!
//! [`run_stdio_json_rpc`] serves the tools over a blocking stdio JSON-RPC 2.0 loop
//! (`initialize` / `ping` / `tools/list` / `tools/call`), mirroring how the data
//! MCP serves; the `tdw-workspace-mcp` binary calls it.

pub mod server;
pub mod state;

pub use server::{MCP_PROTOCOL_VERSION, WorkspaceServer, run_stdio_json_rpc};
pub use state::{Dashboard, GRID_COLUMNS, PlacedWidget, Placement, WorkspaceError, WorkspaceState};
