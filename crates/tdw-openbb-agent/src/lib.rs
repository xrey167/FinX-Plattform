#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! `OpenBB` Workspace **agent protocol bridge** (gap-matrix item **WSB2**).
//!
//! This crate is the pure, I/O-free mapping layer that makes the
//! trading-data-warehouse's agents callable **from** `OpenBB` Workspace as
//! custom copilots. It implements the public `openbb-ai` copilot contract — the
//! `agents.json` discovery document and the `POST /query` Server-Sent-Events
//! protocol — as serde types plus deterministic mapping functions, so the
//! transport crate (`tdw-app-server`) stays a thin parse → map → stream shell.
//!
//! # Clean-room provenance
//!
//! Every contract type is a faithful projection of the **public** `OpenBB`
//! Workspace developer documentation and the `openbb-ai` SDK reference — no
//! `OpenBB` source code was consulted. Doc URLs are cited per module:
//! - `agents.json` reference + copilot protocol: <https://docs.openbb.co/workspace/copilots>
//! - copilot developer overview: <https://docs.openbb.co/workspace/developers>
//!
//! Where the public docs are ambiguous (the `message_chunk` payload key, the
//! `get_widget_data` container key), the chosen reading is documented inline in
//! [`event`]; this crate prefers the most recent SDK helper shapes.
//!
//! # Layout
//!
//! - [`request`] — tolerant `QueryRequest` / `Widget` / `Message` serde types.
//! - [`event`] — the SSE event vocabulary + [`event::SseEvent::to_sse_frame`].
//! - [`manifest`] — the `agents.json` document ([`manifest::default_manifest`]).
//! - [`prompt`] — prompt assembly ([`prompt::assemble_chat_request`]).
//! - [`decision`] — the two-request widget-data decision
//!   ([`decision::needs_widget_data`]).
//! - [`drive`] — the pure event sequencer ([`drive::answer`]) that turns a
//!   request + a streaming model into the ordered SSE events the transport
//!   writes.
//!
//! The crate performs **no I/O** and enforces **no policy**: the transport owns
//! the listener, CORS, auth, and the actual socket writes.

pub mod decision;
pub mod drive;
pub mod event;
pub mod manifest;
pub mod prompt;
pub mod request;

pub use decision::needs_widget_data;
pub use drive::{Answer, answer};
pub use event::{ChartType, Citation, ReasoningStatus, SourceInfo, SseEvent, WidgetDataRequest};
pub use manifest::{
    AgentEndpoints, AgentEntry, AgentFeature, AgentFeatures, default_agent_id,
    default_configurable_features, default_manifest,
};
pub use prompt::{DEFAULT_MAX_OUTPUT_TOKENS, assemble_chat_request, assemble_default};
pub use request::{ContextFile, Message, MessageRole, QueryRequest, Widget, Widgets};
