#![forbid(unsafe_code)]

//! Error types for the `tdw-email` crate.
//!
//! [`EmailError`] covers transactional SMTP paths; [`BroadcastError`] covers
//! the marketing broadcast API path (always compiled; HTTP variants are only
//! reachable when the `broadcast` feature is enabled).

use thiserror::Error;

/// Errors produced by the transactional-email (`smtp`) path.
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

/// Errors produced by the broadcast-client path.
///
/// Variants that carry HTTP semantics (`Http`, `Api`) are reachable at runtime
/// only when the `broadcast` feature is enabled and a live network call is
/// made. They are kept unconditional so callers can match on them without
/// feature guards.
#[derive(Debug, Error)]
pub enum BroadcastError {
    /// The broadcast transport is not configured (`TDW_BROADCAST_API_BASE` or
    /// `TDW_BROADCAST_API_KEY` is absent).
    #[error(
        "broadcast transport is not configured: set TDW_BROADCAST_API_BASE and TDW_BROADCAST_API_KEY"
    )]
    NotConfigured,

    /// An HTTP-layer error (network failure, TLS error, timeout).
    #[error("broadcast http error: {0}")]
    Http(String),

    /// The marketing API returned a non-2xx status.
    ///
    /// `body` is included verbatim for diagnosability; it must not contain
    /// secrets (API keys are sent in headers, not bodies).
    #[error("broadcast api error {status}: {body}")]
    Api {
        /// HTTP status code returned by the provider.
        status: u16,
        /// Response body (may be truncated by the provider).
        body: String,
    },

    /// A request or response body could not be serialised / deserialised.
    #[error("broadcast serialization error: {0}")]
    Serialization(String),
}
