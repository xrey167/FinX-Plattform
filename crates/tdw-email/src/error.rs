#![forbid(unsafe_code)]

//! [`EmailError`] — unified error type for the `tdw-email` crate.

use thiserror::Error;

/// Errors produced by the `tdw-email` crate.
#[derive(Debug, Error)]
pub enum EmailError {
    /// The SMTP transport is not configured (host or from-address missing).
    #[error("email transport is not configured: set TDW_SMTP_HOST and TDW_EMAIL_FROM")]
    NotConfigured,

    /// A field on [`crate::EmailMessage`] failed validation (e.g. empty `to`).
    #[error("invalid email message: {0}")]
    InvalidMessage(String),

    /// A template error — unknown template name or unfilled placeholders.
    #[error("template error: {0}")]
    Template(String),

    /// An SMTP transport error (only present when the `smtp` feature is enabled).
    #[cfg(feature = "smtp")]
    #[error("smtp transport error: {0}")]
    Transport(String),
}
