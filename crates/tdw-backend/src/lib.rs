#![forbid(unsafe_code)]
//! tdw-backend — unified embeddable backend facade.
//!
//! Pure composition + re-export over the underlying `tdw-*` crates: it wires the
//! data/daemon surface and the agent/MCP surface behind two facades and a shared
//! prelude. This crate holds **no business logic** — later phases extend the
//! surface (knowledge, auth/hooks/policy/events) and add the serving binary.

pub mod agent;
pub mod auth;
pub mod config;
pub mod data;
pub mod error;
pub mod prelude;
