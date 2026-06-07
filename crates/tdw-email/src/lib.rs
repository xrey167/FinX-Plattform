#![forbid(unsafe_code)]

//! Transactional email — SMTP send and HTML template fill.
//!
//! # Features
//!
//! | Feature | Effect |
//! |---------|--------|
//! | *(none)* | Core types, template engine, `EmailConfig`, `EmailMessage` — always compiled, no I/O |
//! | `smtp` | Enables [`TransactionalMailer`] backed by a pooled lettre async SMTP transport |
//!
//! # Quick start (smtp feature)
//!
//! ```ignore
//! use tdw_email::{EmailConfig, EmailMessage, TransactionalMailer};
//! use std::collections::BTreeMap;
//!
//! let config = EmailConfig::from_env().expect("SMTP not configured");
//! let mailer = TransactionalMailer::new(config)?;
//! mailer.verify().await?;
//! let mut vars = BTreeMap::new();
//! vars.insert("name", "Alice".to_string());
//! let html = tdw_email::render_template("welcome", &vars)?;
//! let msg = EmailMessage {
//!     from: "noreply@example.com".to_string(),
//!     to: "alice@example.com".to_string(),
//!     subject: "Welcome".to_string(),
//!     text: "Welcome, Alice!".to_string(),
//!     html,
//! };
//! mailer.send(&msg).await?;
//! ```

pub mod config;
pub mod error;
pub mod message;
pub mod template;

#[cfg(feature = "smtp")]
pub mod mailer;

pub use config::EmailConfig;
pub use error::EmailError;
pub use message::EmailMessage;
pub use template::render_template;

#[cfg(feature = "smtp")]
pub use mailer::TransactionalMailer;
