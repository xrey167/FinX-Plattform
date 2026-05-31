//! Unified error type for the backend facade.
//!
//! Kept minimal but `#[from]`-friendly so later phases can extend it with the
//! error types of each capability group they wire in.

use thiserror::Error;

/// A failure constructing or driving a backend facade.
#[derive(Debug, Error)]
pub enum BackendError {
    /// Composition-root construction failed (e.g. building [`AppState`] from
    /// config). The underlying engine error is flattened to a string so this
    /// crate does not need a direct dependency on the engine error type.
    ///
    /// [`AppState`]: tdw_service_api::AppState
    #[error("backend init failed: {0}")]
    Init(String),

    /// Loading a `tdw-agent` registry directory failed.
    #[error("registry load failed: {0}")]
    Load(#[from] tdw_agent::LoadError),

    /// A filesystem error surfaced while resolving a path or directory.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience alias for fallible backend operations.
pub type BackendResult<T> = Result<T, BackendError>;
