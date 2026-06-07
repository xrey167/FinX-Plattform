#![forbid(unsafe_code)]

//! [`EmailMessage`] — the envelope passed to [`crate::TransactionalMailer::send`].

use serde::{Deserialize, Serialize};

/// A fully-composed transactional email ready to send.
///
/// Both `text` and `html` bodies should be provided; the mailer sends a
/// `multipart/alternative` message so mail clients that cannot render HTML
/// fall back to the plain-text body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailMessage {
    /// RFC 5321 envelope sender address (e.g. `"noreply@example.com"`).
    pub from: String,
    /// RFC 5321 envelope recipient address.
    pub to: String,
    /// Message subject line.
    pub subject: String,
    /// Plain-text fallback body.
    pub text: String,
    /// HTML body (may be produced by [`crate::render_template`]).
    pub html: String,
}
