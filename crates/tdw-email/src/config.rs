#![forbid(unsafe_code)]

//! [`EmailConfig`] — SMTP connection parameters read from environment variables.
//!
//! | Variable | Required | Default | Purpose |
//! |----------|----------|---------|---------|
//! | `TDW_SMTP_HOST` | yes | — | SMTP relay hostname |
//! | `TDW_SMTP_PORT` | no | `587` | SMTP port (STARTTLS) |
//! | `TDW_SMTP_USERNAME` | no | `""` | SMTP auth username |
//! | `TDW_SMTP_PASSWORD` | no | `""` | SMTP auth password |
//! | `TDW_EMAIL_FROM` | yes | — | Envelope sender address |

/// SMTP connection parameters.
///
/// Construct via [`EmailConfig::from_env`]; returns `None` when the mandatory
/// variables (`TDW_SMTP_HOST`, `TDW_EMAIL_FROM`) are absent, indicating the
/// transport is unconfigured rather than misconfigured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmailConfig {
    /// SMTP relay hostname (e.g. `"smtp.mailgun.org"`).
    pub smtp_host: String,
    /// SMTP port — defaults to `587` (STARTTLS).
    pub smtp_port: u16,
    /// SMTP authentication username (empty string = no auth).
    pub smtp_username: String,
    /// SMTP authentication password (empty string = no auth).
    pub smtp_password: String,
    /// Envelope sender address used in the `From:` header.
    pub from_address: String,
}

impl EmailConfig {
    /// Read SMTP configuration from environment variables.
    ///
    /// Returns `None` when `TDW_SMTP_HOST` or `TDW_EMAIL_FROM` is absent,
    /// meaning the transport is intentionally unconfigured for this deployment.
    /// Callers that require a configured transport should log a warning and
    /// disable the send path rather than returning an error at startup.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let smtp_host = std::env::var("TDW_SMTP_HOST").ok()?;
        let from_address = std::env::var("TDW_EMAIL_FROM").ok()?;

        let smtp_port = std::env::var("TDW_SMTP_PORT")
            .ok()
            .and_then(|val| val.parse::<u16>().ok())
            .unwrap_or(587);

        let smtp_username = std::env::var("TDW_SMTP_USERNAME").unwrap_or_default();
        let smtp_password = std::env::var("TDW_SMTP_PASSWORD").unwrap_or_default();

        Some(Self {
            smtp_host,
            smtp_port,
            smtp_username,
            smtp_password,
            from_address,
        })
    }
}
