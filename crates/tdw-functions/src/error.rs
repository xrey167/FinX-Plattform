//! [`FunctionError`] — the unified error type for this crate.

use thiserror::Error;

/// Errors produced by the function registry and step execution.
#[derive(Debug, Error)]
pub enum FunctionError {
    /// No function with the given id is registered.
    #[error("unknown function: {0}")]
    UnknownFunction(String),

    /// No run record exists for the given run id.
    #[error("unknown run: {0}")]
    UnknownRun(String),

    /// A step handler returned an error.
    #[error("step {step:?} failed: {message}")]
    Step {
        /// The name of the step that failed.
        step: String,
        /// The human-readable failure description.
        message: String,
    },

    /// A storage backend operation failed.
    #[error("store error: {0}")]
    Store(String),

    /// A JSON serialization or deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// A trigger definition is invalid (e.g. unparseable cron expression).
    #[error("trigger error: {0}")]
    Trigger(String),
}
