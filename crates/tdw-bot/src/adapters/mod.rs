//! Transport adapters: thin wrappers that connect the core [`crate::router`] to
//! a chat platform. Compiled only under the `discord` / `telegram` features.
//!
//! An adapter does exactly three things: receive a message, call
//! [`crate::router::route`], and send the formatted reply. ALL business logic
//! (parsing, fetching, formatting) lives in the framework-free core, so an
//! adapter never grows platform-specific behavior beyond wire plumbing.
//!
//! The shared [`rest::RestDataSource`] is the production [`crate::DataSource`]:
//! a thin REST client over the warehouse data API the chat commands map onto.

pub mod rest;

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "telegram")]
pub mod telegram;
