#![forbid(unsafe_code)]

//! Transactional email and marketing broadcast — SMTP send, HTML templates,
//! and audience-wide broadcast with subscriber management.
//!
//! # Features
//!
//! | Feature | Effect |
//! |---------|--------|
//! | *(none)* | Core types, template engine, `EmailConfig`, `EmailMessage`, broadcast models — always compiled, no I/O |
//! | `smtp` | Enables [`TransactionalMailer`] backed by a pooled lettre async SMTP transport |
//! | `broadcast` | Enables [`BroadcastClient`] backed by reqwest for marketing API calls |
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

pub mod broadcast;
pub mod config;
pub mod error;
pub mod message;
pub mod template;

#[cfg(feature = "smtp")]
pub mod mailer;

pub use broadcast::{
    Broadcast, BroadcastConfig, BroadcastOutcome, BroadcastVisibility, Subscriber, SubscriberList,
    classify_broadcast_response,
};
pub use config::EmailConfig;
pub use error::{BroadcastError, EmailError};
pub use message::EmailMessage;
pub use template::render_template;

#[cfg(feature = "smtp")]
pub use mailer::TransactionalMailer;

#[cfg(feature = "broadcast")]
pub use broadcast::BroadcastClient;
