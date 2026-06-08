#![forbid(unsafe_code)]

//! Application [`FunctionDef`]s for the FinX function runtime.
//!
//! This crate hosts concrete, application-level functions that are registered
//! into a [`tdw_functions::FunctionRegistry`] and activated by the function
//! engine's event/cron wiring. The first such function is the **welcome
//! function**, triggered by the [`USER_CREATED_EVENT`] (`user.created`) emitted
//! when a new user registers.
//!
//! # Mailer port
//!
//! Sending email is abstracted behind the [`WelcomeMailer`] trait so the
//! function body is fully testable offline without a live SMTP transport. The
//! production implementation [`TransactionalWelcomeMailer`] (behind the `smtp`
//! feature) wraps [`tdw_email::TransactionalMailer`]; tests inject a recording
//! mock instead.
//!
//! # Features
//!
//! | Feature | Effect |
//! |---------|--------|
//! | *(none)* | Core types, [`welcome_function`], [`register_app_functions`], the [`WelcomeMailer`] port — no I/O |
//! | `smtp` | Enables [`TransactionalWelcomeMailer`], wrapping the real SMTP mailer |

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use tdw_functions::{FunctionDef, FunctionError, FunctionRegistry, Trigger};

/// Stable identifier of the welcome function.
pub const WELCOME_FUNCTION_ID: &str = "welcome.on-user-created";

/// Event name that activates the welcome function.
///
/// Matches the `user.created` event emitted by the registration op (integration
/// slice A).
pub const USER_CREATED_EVENT: &str = "user.created";

/// Name of the welcome step inside the function body (memoization key).
const WELCOME_STEP: &str = "send-welcome";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by application functions in this crate.
#[derive(Debug, Error)]
pub enum FunctionsAppError {
    /// The incoming event payload could not be parsed into the expected shape.
    #[error("invalid payload: {0}")]
    Payload(String),

    /// Composing the email body failed (template render error).
    #[error("compose error: {0}")]
    Compose(String),

    /// Sending the email failed.
    #[error("mail error: {0}")]
    Mail(String),
}

// ---------------------------------------------------------------------------
// Mailer port
// ---------------------------------------------------------------------------

/// Port for delivering the welcome email.
///
/// Abstracted so the welcome function can be exercised offline with a mock.
/// The single method is synchronous: the rendered HTML `body` is fully composed
/// by the caller, so the implementation only has to deliver it.
pub trait WelcomeMailer: Send + Sync {
    /// Deliver the welcome email `body` (HTML) to `to_email`.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionsAppError::Mail`] when delivery fails.
    fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError>;
}

// ---------------------------------------------------------------------------
// Production mailer (smtp feature)
// ---------------------------------------------------------------------------

/// Production [`WelcomeMailer`] backed by [`tdw_email::TransactionalMailer`].
///
/// `TransactionalMailer::send` is async, while [`WelcomeMailer::send_welcome`]
/// is synchronous (the function step that invokes it is already the async
/// boundary). The adapter owns a dedicated current-thread Tokio runtime and
/// drives the async send to completion on it, keeping the port synchronous and
/// independent of the caller's runtime flavor.
///
/// Requires the `smtp` feature.
#[cfg(feature = "smtp")]
pub struct TransactionalWelcomeMailer {
    mailer: tdw_email::TransactionalMailer,
    runtime: tokio::runtime::Runtime,
    subject: String,
}

#[cfg(feature = "smtp")]
impl std::fmt::Debug for TransactionalWelcomeMailer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransactionalWelcomeMailer")
            .field("mailer", &"<TransactionalMailer>")
            .field("subject", &self.subject)
            .finish()
    }
}

#[cfg(feature = "smtp")]
impl TransactionalWelcomeMailer {
    /// Build the mailer from an [`tdw_email::EmailConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`FunctionsAppError::Mail`] when the SMTP transport cannot be
    /// constructed, or when the internal send runtime cannot be created.
    pub fn new(config: tdw_email::EmailConfig) -> Result<Self, FunctionsAppError> {
        let mailer = tdw_email::TransactionalMailer::new(config)
            .map_err(|err| FunctionsAppError::Mail(err.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| FunctionsAppError::Mail(err.to_string()))?;
        Ok(Self {
            mailer,
            runtime,
            subject: "Welcome to FinX".to_string(),
        })
    }
}

#[cfg(feature = "smtp")]
impl WelcomeMailer for TransactionalWelcomeMailer {
    fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError> {
        let msg = tdw_email::EmailMessage {
            from: String::new(), // mailer uses its configured from_address
            to: to_email.to_string(),
            subject: self.subject.clone(),
            text: format!("Welcome to FinX! Your account ({to_email}) is ready."),
            html: body.to_string(),
        };
        self.runtime
            .block_on(self.mailer.send(&msg))
            .map_err(|err| FunctionsAppError::Mail(err.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Payload
// ---------------------------------------------------------------------------

/// Minimal view of the `user.created` event payload consumed by the welcome
/// function. Extra fields in the payload are ignored.
#[derive(Debug, Deserialize)]
struct UserCreated {
    email: String,
    #[serde(default)]
    display_name: Option<String>,
}

/// Compose the welcome email HTML body from the new user's details.
fn compose_welcome_body(user: &UserCreated) -> Result<String, FunctionsAppError> {
    let name = user
        .display_name
        .as_deref()
        .filter(|n| !n.is_empty())
        .unwrap_or(user.email.as_str());

    let mut vars: BTreeMap<&str, String> = BTreeMap::new();
    vars.insert("name", name.to_string());
    vars.insert(
        "body",
        "Thanks for creating your FinX account. We're glad to have you on board.".to_string(),
    );
    vars.insert(
        "cta",
        "Sign in any time to start exploring the platform.".to_string(),
    );
    // No per-user unsubscribe link wired yet; the welcome message is
    // transactional. Use a stable placeholder so the strict template renderer
    // is satisfied.
    vars.insert("unsubscribe_url", "#".to_string());

    tdw_email::render_template("welcome", &vars)
        .map_err(|err| FunctionsAppError::Compose(err.to_string()))
}

// ---------------------------------------------------------------------------
// welcome_function
// ---------------------------------------------------------------------------

/// Map a [`FunctionsAppError`] into a [`FunctionError::Step`] for the welcome
/// step.
fn into_step_error(err: &FunctionsAppError) -> FunctionError {
    FunctionError::Step {
        step: WELCOME_STEP.to_string(),
        message: err.to_string(),
    }
}

/// Build the welcome [`FunctionDef`], event-triggered on [`USER_CREATED_EVENT`].
///
/// The handler parses the `user.created` payload, composes a welcome email
/// body, and delivers it through the injected `mailer` inside a memoizable
/// step. It returns `{"sent": true, "to": <email>}` on success.
#[must_use]
pub fn welcome_function(mailer: Arc<dyn WelcomeMailer>) -> FunctionDef {
    FunctionDef::from_fn(
        WELCOME_FUNCTION_ID,
        vec![Trigger::Event(USER_CREATED_EVENT.to_string())],
        move |ctx, payload| {
            let mailer = Arc::clone(&mailer);
            Box::pin(async move {
                let user: UserCreated = serde_json::from_value(payload)
                    .map_err(|err| into_step_error(&FunctionsAppError::Payload(err.to_string())))?;

                if user.email.is_empty() {
                    return Err(into_step_error(&FunctionsAppError::Payload(
                        "missing email".to_string(),
                    )));
                }

                let email = user.email.clone();
                ctx.step(WELCOME_STEP, || {
                    let mailer = Arc::clone(&mailer);
                    let email = email.clone();
                    async move {
                        let body =
                            compose_welcome_body(&user).map_err(|err| into_step_error(&err))?;
                        mailer
                            .send_welcome(&email, &body)
                            .map_err(|err| into_step_error(&err))?;
                        Ok(json!({"sent": true, "to": email}))
                    }
                })
                .await
            })
        },
    )
}

// ---------------------------------------------------------------------------
// register_app_functions
// ---------------------------------------------------------------------------

/// Register all application functions into `registry`.
///
/// Currently registers [`welcome_function`]; this is the extension point for
/// further application functions (e.g. the re-engagement function in a later
/// slice).
///
/// # Errors
///
/// Returns [`FunctionError::Store`] if a function id is already registered.
pub fn register_app_functions(
    registry: &mut FunctionRegistry,
    mailer: Arc<dyn WelcomeMailer>,
) -> Result<(), FunctionError> {
    registry.register(welcome_function(mailer))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{
        Arc, FunctionRegistry, FunctionsAppError, Trigger, USER_CREATED_EVENT, WELCOME_FUNCTION_ID,
        WelcomeMailer, json, register_app_functions, welcome_function,
    };

    /// Recording mock that captures every `(to_email, body)` it is asked to send.
    #[derive(Default)]
    struct MockMailer {
        sent: Mutex<Vec<(String, String)>>,
    }

    impl WelcomeMailer for MockMailer {
        fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError> {
            self.sent
                .lock()
                .expect("lock")
                .push((to_email.to_string(), body.to_string()));
            Ok(())
        }
    }

    const NOW: i64 = 1_700_000_000_000;

    #[tokio::test]
    async fn welcome_dispatch_sends_one_email() {
        let mock = Arc::new(MockMailer::default());
        let mut reg = FunctionRegistry::new();
        register_app_functions(&mut reg, Arc::clone(&mock) as Arc<dyn WelcomeMailer>)
            .expect("register");

        let payload = json!({"user_id": "u1", "email": "a@b.com", "created_at_ms": 1});
        let result = reg
            .invoke(WELCOME_FUNCTION_ID, payload, "run-welcome".to_string(), NOW)
            .await
            .expect("invoke");

        assert_eq!(result, json!({"sent": true, "to": "a@b.com"}));

        let sent = mock.sent.lock().expect("lock");
        assert_eq!(sent.len(), 1, "exactly one email sent");
        assert_eq!(sent[0].0, "a@b.com");
        assert!(!sent[0].1.is_empty(), "body must be non-empty");
    }

    #[tokio::test]
    async fn welcome_uses_display_name_when_present() {
        let mock = Arc::new(MockMailer::default());
        let def = welcome_function(Arc::clone(&mock) as Arc<dyn WelcomeMailer>);
        let mut reg = FunctionRegistry::new();
        reg.register(def).expect("register");

        let payload = json!({
            "user_id": "u2",
            "email": "c@d.com",
            "display_name": "Carol",
            "created_at_ms": 2,
        });
        reg.invoke(WELCOME_FUNCTION_ID, payload, "run-name".to_string(), NOW)
            .await
            .expect("invoke");

        let sent = mock.sent.lock().expect("lock");
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].1.contains("Carol"),
            "body should greet the display name: {}",
            sent[0].1
        );
    }

    #[tokio::test]
    async fn malformed_payload_errors_and_sends_nothing() {
        let mock = Arc::new(MockMailer::default());
        let mut reg = FunctionRegistry::new();
        register_app_functions(&mut reg, Arc::clone(&mock) as Arc<dyn WelcomeMailer>)
            .expect("register");

        // Missing `email` field.
        let payload = json!({"user_id": "u3", "created_at_ms": 3});
        let err = reg
            .invoke(WELCOME_FUNCTION_ID, payload, "run-bad".to_string(), NOW)
            .await
            .expect_err("should fail on malformed payload");
        assert!(matches!(err, tdw_functions::FunctionError::Step { .. }));

        assert!(
            mock.sent.lock().expect("lock").is_empty(),
            "no email on malformed payload"
        );
    }

    #[test]
    fn welcome_function_triggers_and_id() {
        let mock = Arc::new(MockMailer::default());
        let def = welcome_function(mock as Arc<dyn WelcomeMailer>);
        assert_eq!(def.id, WELCOME_FUNCTION_ID);
        assert!(
            def.triggers
                .contains(&Trigger::Event(USER_CREATED_EVENT.to_string())),
            "must carry the user.created event trigger"
        );
    }

    #[test]
    fn register_app_functions_is_idempotent_guarded() {
        let mock = Arc::new(MockMailer::default());
        let mut reg = FunctionRegistry::new();
        register_app_functions(&mut reg, Arc::clone(&mock) as Arc<dyn WelcomeMailer>)
            .expect("first register");
        let err = register_app_functions(&mut reg, mock as Arc<dyn WelcomeMailer>)
            .expect_err("duplicate register must fail");
        assert!(matches!(err, tdw_functions::FunctionError::Store(_)));
    }
}
