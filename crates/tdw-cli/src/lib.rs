//! Library facade for `tdw-cli` — exposes the `demo` module so integration
//! tests can call [`demo::run_smoke`] directly without spawning a subprocess.
//!
//! Production use: `tdw-cli` is a binary; this lib target exists solely for
//! the integration test suite.

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

/// The shared error type used by all `tdw-cli` commands.
pub type CliError = Box<dyn std::error::Error + Send + Sync>;

pub mod demo;
