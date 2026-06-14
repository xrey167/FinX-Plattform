#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! A Discord + Telegram chat-bot surface over the warehouse REST/catalog and
//! charts (openbb-ecosystem-p1 **G007** — the chat-bot-equivalent, a
//! differentiator). It mirrors what a financial-data chat bot like `openbb-bot`
//! offers: `/quote`, `/news`, and `/chart` over a chat channel.
//!
//! ## A thin client with an offline core
//!
//! The crate is split so the platform's business logic lives in ONE place and is
//! testable without any bot framework or network:
//!
//! - [`command`] parses a chat string into a typed [`command::Command`].
//! - [`data::DataSource`] is the abstract data-access seam; the router calls it
//!   so a unit test can inject a fake and the core never hard-requires a live
//!   daemon.
//! - [`router::route`] is the whole core: parse → fetch through the seam →
//!   format a [`response::BotResponse`] (text / table / chart).
//! - [`response`] is the platform-agnostic reply model plus its plain-text
//!   rendering.
//! - [`chart`] turns a price series into a chart payload.
//!
//! The transport adapters ([`adapters`], compiled only under the `discord` /
//! `telegram` features) are thin: receive a message, call [`router::route`], send
//! the formatted reply. NO business logic lives in an adapter.
//!
//! ## Feature gating (CI-safe, same discipline as the chart host's `gui` gate)
//!
//! The bot frameworks (`serenity` for Discord, `teloxide` for Telegram) are heavy
//! async deps, so they are OPTIONAL and behind NON-DEFAULT features. The DEFAULT
//! build is the pure-Rust, framework-free, fully-offline core, so
//! `cargo build/clippy/test --workspace` compiles nothing heavy.
//!
//! ## Charts
//!
//! `tdw-charting` emits a Plotly figure JSON *spec* (not pixels) so a `plotly.js`
//! client renders it with zero native dependency. A chat client wants an image,
//! and rasterizing Plotly JSON in pure Rust is not feasible, so the
//! NON-DEFAULT `charts` feature renders a simple line PNG directly from the close
//! series with the pure-Rust `plotters` bitmap backend (no native/system graphics
//! dependency). Without `charts`, `/chart` attaches the figure-JSON spec plus a
//! text summary, which a chart-host can render.

pub mod chart;
pub mod command;
pub mod data;
pub mod response;
pub mod router;

#[cfg(any(feature = "discord", feature = "telegram"))]
pub mod adapters;

pub use command::{Command, ParseError, parse};
pub use data::{DataError, DataSource, NewsItem, Quote};
pub use response::{BotResponse, ChartReply, Table};
pub use router::route;
