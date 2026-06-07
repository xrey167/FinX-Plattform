#![forbid(unsafe_code)]
#![cfg(feature = "smtp")]

//! [`TransactionalMailer`] — pooled async SMTP transport backed by lettre.
//!
//! Requires the `smtp` feature. Uses STARTTLS on the configured port with
//! pure-Rust TLS (rustls) so it compiles on Windows without a native TLS library.

use lettre::message::{MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use tracing::debug;

use crate::{EmailConfig, EmailError, EmailMessage};

/// Pooled async SMTP mailer.
///
/// Construct via [`TransactionalMailer::new`] and reuse across the application
/// lifetime — the underlying lettre pool amortises connection setup.
pub struct TransactionalMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from_address: String,
}

impl TransactionalMailer {
    /// Build a mailer from `config`, establishing the SMTP relay with STARTTLS.
    ///
    /// # Errors
    ///
    /// Returns [`EmailError::Transport`] when lettre cannot build the relay
    /// (e.g. invalid hostname).
    pub fn new(config: EmailConfig) -> Result<Self, EmailError> {
        let builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.smtp_host)
            .map_err(|err| EmailError::Transport(err.to_string()))?
            .port(config.smtp_port);

        let transport = if config.smtp_username.is_empty() {
            builder.build()
        } else {
            builder
                .credentials(Credentials::new(
                    config.smtp_username.clone(),
                    config.smtp_password.clone(),
                ))
                .build()
        };

        Ok(Self {
            transport,
            from_address: config.from_address,
        })
    }

    /// Test the SMTP connection without sending a message.
    ///
    /// # Errors
    ///
    /// Returns [`EmailError::Transport`] when the SMTP handshake fails.
    pub async fn verify(&self) -> Result<(), EmailError> {
        self.transport
            .test_connection()
            .await
            .map_err(|err| EmailError::Transport(err.to_string()))?;
        Ok(())
    }

    /// Send `msg` as a `multipart/alternative` email (text + HTML).
    ///
    /// # Errors
    ///
    /// - [`EmailError::InvalidMessage`] when `to`, `from`, or `subject` is empty.
    /// - [`EmailError::Transport`] when the SMTP relay rejects the message.
    pub async fn send(&self, msg: &EmailMessage) -> Result<(), EmailError> {
        validate_message(msg)?;

        let from_addr = if msg.from.is_empty() {
            self.from_address.clone()
        } else {
            msg.from.clone()
        };

        let email = Message::builder()
            .from(
                from_addr
                    .parse()
                    .map_err(|err: lettre::address::AddressError| {
                        EmailError::InvalidMessage(format!("invalid from address: {err}"))
                    })?,
            )
            .to(msg
                .to
                .parse()
                .map_err(|err: lettre::address::AddressError| {
                    EmailError::InvalidMessage(format!("invalid to address: {err}"))
                })?)
            .subject(msg.subject.clone())
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(msg.text.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(msg.html.clone()),
                    ),
            )
            .map_err(|err| EmailError::Transport(format!("failed to build message: {err}")))?;

        debug!(to = %msg.to, subject = %msg.subject, "sending email");

        self.transport
            .send(email)
            .await
            .map_err(|err| EmailError::Transport(err.to_string()))?;

        Ok(())
    }
}

/// Validate that mandatory [`EmailMessage`] fields are non-empty.
fn validate_message(msg: &EmailMessage) -> Result<(), EmailError> {
    if msg.to.is_empty() {
        return Err(EmailError::InvalidMessage(
            "`to` address must not be empty".to_string(),
        ));
    }
    if msg.subject.is_empty() {
        return Err(EmailError::InvalidMessage(
            "`subject` must not be empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(to: &str, subject: &str) -> EmailMessage {
        EmailMessage {
            from: "sender@example.com".to_string(),
            to: to.to_string(),
            subject: subject.to_string(),
            text: "Hello".to_string(),
            html: "<p>Hello</p>".to_string(),
        }
    }

    #[test]
    fn validate_rejects_empty_to() {
        let msg = make_msg("", "Subject");
        let err = validate_message(&msg).expect_err("should reject empty to");
        assert!(err.to_string().contains("`to`"));
    }

    #[test]
    fn validate_rejects_empty_subject() {
        let msg = make_msg("user@example.com", "");
        let err = validate_message(&msg).expect_err("should reject empty subject");
        assert!(err.to_string().contains("`subject`"));
    }

    #[test]
    fn validate_accepts_valid_message() {
        let msg = make_msg("user@example.com", "Hello");
        validate_message(&msg).expect("valid message should pass");
    }
}
