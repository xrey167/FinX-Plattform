#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! `OpenBB` Workspace bridge (gap-matrix item **WSB1**, the widgets backend).
//!
//! This crate is the *projection* that makes the trading-data-warehouse usable
//! as a custom data backend inside `OpenBB` Workspace (`pro.openbb.co`). The
//! typed [`tdw_endpoint_catalog`] stays the single source of truth; this crate
//! derives the `OpenBB` Workspace `widgets.json` / `apps.json` contract from it
//! automatically. `OpenBB`'s `JSON` shapes live **only** here, never leaking
//! into the catalog.
//!
//! # Clean-room provenance
//!
//! Every contract type is a faithful projection of the **public** `OpenBB`
//! Workspace developer documentation — no `OpenBB` source code was consulted.
//! The documented field names (camelCase where the docs use camelCase) match
//! exactly so a serialized document is byte-compatible with what Workspace
//! ingests. Doc sources are cited per module:
//! - `widgets.json` reference: <https://docs.openbb.co/workspace/data-integration/widgets.json>
//! - `apps.json` / templates: <https://docs.openbb.co/workspace/data-integration/templates>
//! - Custom backend setup: <https://docs.openbb.co/workspace/getting-started/custom-backend>
//!
//! # Layout
//!
//! - [`config`] — serde types for the `widgets.json` / `apps.json` contract.
//! - [`derive`] — the per-route derivation engine ([`derive::derive_widget`]).
//! - [`catalog`] — the full-document assembly ([`catalog::catalog_widgets`],
//!   [`catalog::widgets_json`], [`catalog::apps_json`], [`catalog::default_app`]).
//!
//! The crate performs **no I/O** and enforces **no policy**: it is a pure,
//! deterministic projection plus serde types.

pub mod catalog;
pub mod config;
pub mod derive;
pub mod graph;

pub use catalog::{apps_json, catalog_widgets, default_app, widgets_json};
pub use config::{
    AppConfig, AppTab, ChartConfig, ColumnDef, GridData, LayoutItem, McpServerRef, McpTool,
    ParamDef, ParamOption, TableConfig, WidgetConfig, WidgetData,
};
pub use derive::{DATA_KEY, derive_widget};
pub use graph::{
    GRAPH_DATA_KEY, GRAPH_DEFAULT_DEPTH, GRAPH_WIDGET_ENDPOINT, GRAPH_WIDGET_ID, graph_widget,
};
