//! Progressive, offline-runnable examples for the `FinX` <-> `OpenBB` Workspace
//! surface.
//!
//! Each example is a small library function here, driven by a thin `main()` in a
//! sibling `NN-name/` directory and exercised directly by the CI integration
//! tests in `tests/`. Every function runs network-free: the data examples boot
//! the in-memory daemon (`AppState::in_memory_for_tests`) and resolve the
//! always-registered `fileset` fixture; the copilot examples drive the offline
//! `StubLanguageModel`. Nothing here reaches the network or needs credentials.
//!
//! The modules are layered the way a developer learns the surface:
//!
//! 1. [`minimal`] — the raw `widgets.json` + data-endpoint contract, hand-rolled
//!    over a few lines of a standalone HTTP server (no daemon).
//! 2. [`derived`] — the real backend: boot `serve_workspace_http` and serve the
//!    catalog-derived `widgets.json` (60+ widgets) and offline `widget-data`.
//! 3. [`app`] — the curated `apps.json` plus a custom app definition.
//! 4. [`agent_echo`] — the minimal copilot: `agents.json` + a streamed answer.
//! 5. [`reasoning_citations`] — reasoning steps + a citation to a widget.
//! 6. [`widget_data`] — the stateless two-request `get_widget_data` round trip.
//! 7. [`charts_tables`] — table + chart SSE artifacts from widget data.
//!
//! The pure helpers (document builders, the in-process copilot driver) return
//! plain `serde_json` values so a test can assert golden outputs without going
//! over the wire; the server helpers return a [`RunningServer`] handle a `main()`
//! or a test can hold while it issues real HTTP requests.

pub mod agent_echo;
pub mod app;
pub mod charts_tables;
pub mod client;
pub mod derived;
pub mod minimal;
pub mod reasoning_citations;
pub mod server;
pub mod widget_data;

pub use server::RunningServer;
