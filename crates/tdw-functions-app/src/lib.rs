#![forbid(unsafe_code)]

//! Application [`FunctionDef`]s for the `FinX` function runtime.
//!
//! This crate hosts concrete, application-level functions that are registered
//! into a [`tdw_functions::FunctionRegistry`] and activated by the function
//! engine's event/cron wiring. Two functions live here:
//!
//! * the **welcome function**, triggered by the [`USER_CREATED_EVENT`]
//!   (`user.created`) emitted when a new user registers;
//! * the **re-engagement function** ([`reengagement_function`]), a daily cron
//!   ([`Trigger::Cron`] `0 9 * * *`) that scans for dormant users via
//!   [`tdw_identity::UserStore::find_dormant_users`] and emails each one,
//!   stamping [`tdw_identity::UserStore::mark_reengagement_sent`] so the
//!   cooldown window prevents re-emailing on the next run (fire-once).
//!
//! # Mailer ports
//!
//! Sending email is abstracted behind the [`WelcomeMailer`] and
//! [`ReengagementMailer`] traits so the function bodies are fully testable
//! offline without a live SMTP transport. The production implementations
//! [`TransactionalWelcomeMailer`] / [`TransactionalReengagementMailer`] (behind
//! the `smtp` feature) wrap [`tdw_email::TransactionalMailer`]; tests inject a
//! recording mock instead.
//!
//! # Clock boundary
//!
//! The re-engagement cron reads `now_ms` **once** at handler entry via
//! `SystemTime::now()` — the exact pattern used by `tdw-alert-evaluator`. The
//! two cutoffs (`now_ms - dormant_after_ms`, `now_ms - reengagement_cooldown_ms`)
//! and the `mark_reengagement_sent` stamp all use that single timestamp so the
//! memoized steps are deterministic across a replay. Tests supply a fixed
//! `now_ms` through [`FunctionRegistry::invoke`].
//!
//! # Features
//!
//! | Feature | Effect |
//! |---------|--------|
//! | *(none)* | Core types, [`welcome_function`], [`reengagement_function`], [`register_app_functions`], the mailer ports — no I/O |
//! | `smtp` | Enables [`TransactionalWelcomeMailer`] / [`TransactionalReengagementMailer`], wrapping the real SMTP mailer |

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use tdw_functions::{FunctionDef, FunctionError, FunctionRegistry, Trigger};
use tdw_identity::{User, UserStore};

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
// Re-engagement mailer port
// ---------------------------------------------------------------------------

/// Port for delivering a dormant-user re-engagement email.
///
/// Mirrors [`WelcomeMailer`]: abstracted so the re-engagement function can be
/// exercised offline with a recording mock. The single method is synchronous;
/// the rendered HTML `body` is composed by the caller, so the implementation
/// only has to deliver it.
pub trait ReengagementMailer: Send + Sync {
    /// Deliver the re-engagement email `body` (HTML) to `to_email`.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionsAppError::Mail`] when delivery fails.
    fn send_reengagement(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError>;
}

// ---------------------------------------------------------------------------
// Production re-engagement mailer (smtp feature)
// ---------------------------------------------------------------------------

/// Production [`ReengagementMailer`] backed by [`tdw_email::TransactionalMailer`].
///
/// Mirrors [`TransactionalWelcomeMailer`]: owns a dedicated current-thread Tokio
/// runtime and drives the async send to completion on it, keeping the port
/// synchronous and independent of the caller's runtime flavor.
///
/// Requires the `smtp` feature.
#[cfg(feature = "smtp")]
pub struct TransactionalReengagementMailer {
    mailer: tdw_email::TransactionalMailer,
    runtime: tokio::runtime::Runtime,
    subject: String,
}

#[cfg(feature = "smtp")]
impl std::fmt::Debug for TransactionalReengagementMailer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransactionalReengagementMailer")
            .field("mailer", &"<TransactionalMailer>")
            .field("subject", &self.subject)
            .finish()
    }
}

#[cfg(feature = "smtp")]
impl TransactionalReengagementMailer {
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
            subject: "We miss you at FinX".to_string(),
        })
    }
}

#[cfg(feature = "smtp")]
impl ReengagementMailer for TransactionalReengagementMailer {
    fn send_reengagement(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError> {
        let msg = tdw_email::EmailMessage {
            from: String::new(), // mailer uses its configured from_address
            to: to_email.to_string(),
            subject: self.subject.clone(),
            text: "It's been a while — sign in to FinX to pick up where you left off.".to_string(),
            html: body.to_string(),
        };
        self.runtime
            .block_on(self.mailer.send(&msg))
            .map_err(|err| FunctionsAppError::Mail(err.to_string()))
    }
}

// ---------------------------------------------------------------------------
// ReengagementDeps
// ---------------------------------------------------------------------------

/// Stable identifier of the re-engagement function.
pub const REENGAGEMENT_FUNCTION_ID: &str = "reengagement.dormant-scan";

/// Name of the scan step inside the function body (memoization key).
const REENGAGEMENT_SCAN_STEP: &str = "scan";

/// Name of the notify step inside the function body (memoization key).
const REENGAGEMENT_NOTIFY_STEP: &str = "notify";

/// Dependency bundle passed to [`reengagement_function`].
///
/// `user_store` and `mailer` are `Arc`-wrapped trait objects so they can be
/// shared cheaply across async tasks and the registry's `Arc<FunctionDef>`.
///
/// Following the `tdw-alert-evaluator` precedent, there is **no** clock field:
/// `now_ms` is read once at handler entry from `SystemTime::now()` and tests
/// inject a fixed timestamp through [`FunctionRegistry::invoke`].
#[derive(Clone)]
pub struct ReengagementDeps {
    /// Source of dormant [`User`] records and the `mark_reengagement_sent`
    /// fire-once stamp.
    pub user_store: Arc<dyn UserStore>,
    /// Synchronous re-engagement email port.
    pub mailer: Arc<dyn ReengagementMailer>,
    /// Dormancy window in milliseconds; a user inactive for at least this long
    /// is eligible. Typically [`tdw_identity::DEFAULT_DORMANT_AFTER_MS`].
    pub dormant_after_ms: i64,
    /// Re-engagement cooldown in milliseconds; a user re-engaged within this
    /// window is skipped. Typically
    /// [`tdw_identity::DEFAULT_REENGAGEMENT_COOLDOWN_MS`].
    pub reengagement_cooldown_ms: i64,
    /// Maximum number of dormant users processed per run.
    pub batch_limit: usize,
}

/// Compose the re-engagement email HTML body for `user`.
fn compose_reengagement_body(user: &User) -> Result<String, FunctionsAppError> {
    let name = if user.display_name.is_empty() {
        user.email.as_str()
    } else {
        user.display_name.as_str()
    };

    let mut vars: BTreeMap<&str, String> = BTreeMap::new();
    vars.insert("name", name.to_string());
    vars.insert(
        "body",
        "We noticed you have not signed in for a while. Your FinX account is \
         still here whenever you are ready to come back."
            .to_string(),
    );
    vars.insert("cta_url", "#".to_string());
    vars.insert("cta_label", "Sign back in".to_string());
    // No per-user unsubscribe link wired yet; use a stable placeholder so the
    // strict template renderer is satisfied (mirrors the welcome body).
    vars.insert("unsubscribe_url", "#".to_string());

    tdw_email::render_template("reengagement", &vars)
        .map_err(|err| FunctionsAppError::Compose(err.to_string()))
}

// ---------------------------------------------------------------------------
// reengagement_function
// ---------------------------------------------------------------------------

/// Build the re-engagement [`FunctionDef`], a daily cron on `0 9 * * *`.
///
/// The handler reads `now_ms` once at entry, then runs two memoized steps:
///
/// 1. **`scan`** — [`UserStore::find_dormant_users`] with cutoffs
///    `now_ms - dormant_after_ms` (dormant) and
///    `now_ms - reengagement_cooldown_ms` (cooldown), capped at `batch_limit`.
/// 2. **`notify`** — for each dormant user, deliver a composed re-engagement
///    email then [`UserStore::mark_reengagement_sent`] (fire-once: the stamp
///    makes the user fail the cooldown filter on the next run). A per-user mail
///    failure is logged and skipped — it does **not** abort the run, exactly
///    like `tdw-alert-evaluator`'s notifier-error handling.
///
/// Returns `{"scanned": <dormant count>, "reengaged": <emailed count>}`.
#[must_use]
pub fn reengagement_function(deps: ReengagementDeps) -> FunctionDef {
    FunctionDef::from_fn(
        REENGAGEMENT_FUNCTION_ID,
        vec![Trigger::Cron("0 9 * * *".into())],
        move |ctx, _payload| {
            let deps = deps.clone();
            Box::pin(async move {
                // Clock boundary: read once at handler entry so both cutoffs and
                // the fire-once stamp share one timestamp across replays.
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));

                // ----------------------------------------------------------
                // Step 1: scan for dormant users
                // ----------------------------------------------------------
                let step_scan: Value = ctx
                    .step(REENGAGEMENT_SCAN_STEP, || {
                        let store = Arc::clone(&deps.user_store);
                        let dormant_before_ms = now_ms.saturating_sub(deps.dormant_after_ms);
                        let reengaged_before_ms =
                            now_ms.saturating_sub(deps.reengagement_cooldown_ms);
                        let limit = deps.batch_limit;
                        async move {
                            let dormant = store
                                .find_dormant_users(dormant_before_ms, reengaged_before_ms, limit)
                                .await
                                .map_err(|err| FunctionError::Step {
                                    step: REENGAGEMENT_SCAN_STEP.to_string(),
                                    message: err.to_string(),
                                })?;
                            serde_json::to_value(&dormant)
                                .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                let dormant: Vec<User> = serde_json::from_value(step_scan).map_err(|err| {
                    FunctionError::Serialization(format!("deserialize scan result: {err}"))
                })?;

                let scanned = dormant.len();

                // Short-circuit: nothing to do.
                if dormant.is_empty() {
                    return Ok(json!({"scanned": 0, "reengaged": 0}));
                }

                // ----------------------------------------------------------
                // Step 2: notify each dormant user (fire-once)
                // ----------------------------------------------------------
                let step_notify: Value = ctx
                    .step(REENGAGEMENT_NOTIFY_STEP, || {
                        let store = Arc::clone(&deps.user_store);
                        let mailer = Arc::clone(&deps.mailer);
                        let users = dormant.clone();
                        async move {
                            let mut reengaged: usize = 0;
                            for user in &users {
                                let body = match compose_reengagement_body(user) {
                                    Ok(body) => body,
                                    Err(err) => {
                                        tracing::warn!(
                                            user_id = %user.id,
                                            error = %err,
                                            "re-engagement compose failed — skipping user"
                                        );
                                        continue;
                                    }
                                };

                                // Best-effort: a per-user mail failure is logged
                                // and skipped, never aborting the whole run.
                                if let Err(err) = mailer.send_reengagement(&user.email, &body) {
                                    tracing::warn!(
                                        user_id = %user.id,
                                        error = %err,
                                        "re-engagement send failed — skipping user"
                                    );
                                    continue;
                                }

                                // Fire-once stamp: marking the user reengaged at
                                // now_ms makes them fail the cooldown filter on
                                // the next run, so they are not re-emailed.
                                store
                                    .mark_reengagement_sent(&user.id, now_ms)
                                    .await
                                    .map_err(|err| FunctionError::Step {
                                        step: REENGAGEMENT_NOTIFY_STEP.to_string(),
                                        message: format!(
                                            "mark_reengagement_sent({}): {err}",
                                            user.id
                                        ),
                                    })?;

                                reengaged += 1;
                            }

                            serde_json::to_value(json!({
                                "scanned": scanned,
                                "reengaged": reengaged,
                            }))
                            .map_err(|err| FunctionError::Serialization(err.to_string()))
                        }
                    })
                    .await?;

                Ok(step_notify)
            })
        },
    )
}

// ---------------------------------------------------------------------------
// register_app_functions
// ---------------------------------------------------------------------------

/// Register all application functions into `registry`.
///
/// Currently registers [`welcome_function`]; the re-engagement cron is
/// registered separately via [`register_reengagement`] because it carries its
/// own [`ReengagementDeps`] bundle.
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

/// Register the [`reengagement_function`] into `registry`.
///
/// Kept separate from [`register_app_functions`] so each function's distinct
/// dependency bundle is supplied independently (mirrors
/// `tdw_alert_evaluator::register`).
///
/// # Errors
///
/// Returns [`FunctionError::Store`] if [`REENGAGEMENT_FUNCTION_ID`] is already
/// registered.
pub fn register_reengagement(
    registry: &mut FunctionRegistry,
    deps: ReengagementDeps,
) -> Result<(), FunctionError> {
    registry.register(reengagement_function(deps))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{
        Arc, FunctionRegistry, FunctionsAppError, REENGAGEMENT_FUNCTION_ID, ReengagementDeps,
        ReengagementMailer, Trigger, USER_CREATED_EVENT, WELCOME_FUNCTION_ID, WelcomeMailer, json,
        reengagement_function, register_app_functions, register_reengagement, welcome_function,
    };
    use tdw_identity::{InMemoryUserStore, NewUser, UserStore};

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
    #[allow(clippy::significant_drop_tightening)] // test: mock Arc outlives guard block intentionally
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

        {
            let sent = mock.sent.lock().expect("lock");
            assert_eq!(sent.len(), 1, "exactly one email sent");
            assert_eq!(sent[0].0, "a@b.com");
            assert!(!sent[0].1.is_empty(), "body must be non-empty");
        }
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)] // test: mock Arc outlives guard block intentionally
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

        {
            let sent = mock.sent.lock().expect("lock");
            assert_eq!(sent.len(), 1);
            assert!(
                sent[0].1.contains("Carol"),
                "body should greet the display name: {}",
                sent[0].1
            );
        }
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

    // -----------------------------------------------------------------------
    // Re-engagement tests
    // -----------------------------------------------------------------------

    /// 30 days in milliseconds — the dormancy window / cooldown used by tests.
    const WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

    /// A timestamp far beyond any plausible wall clock (year ~+5000).
    ///
    /// The re-engagement handler reads `now_ms` from `SystemTime::now()` (the
    /// `now_ms` passed to [`FunctionRegistry::invoke`] is consumed only by the
    /// registry's run bookkeeping, not the handler — same as the
    /// `tdw-alert-evaluator` precedent). So test data cannot be pinned to a
    /// fixed past `NOW`; instead "recently active / recently re-engaged" stamps
    /// use this far-future value, which always exceeds `now_real - WINDOW_MS`
    /// regardless of the actual wall clock, keeping the filter deterministic.
    const FUTURE: i64 = 100_000_000_000_000;

    /// Recording mock [`ReengagementMailer`].  When `fail_for` matches the
    /// recipient email, the send returns an error (to exercise per-user
    /// failure tolerance) but the attempt is still recorded.
    #[derive(Default)]
    struct MockReengagementMailer {
        sent: Mutex<Vec<(String, String)>>,
        fail_for: Option<String>,
    }

    impl ReengagementMailer for MockReengagementMailer {
        fn send_reengagement(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError> {
            self.sent
                .lock()
                .expect("lock")
                .push((to_email.to_string(), body.to_string()));
            if self.fail_for.as_deref() == Some(to_email) {
                return Err(FunctionsAppError::Mail("forced test failure".to_string()));
            }
            Ok(())
        }
    }

    /// Register a user (left dormant: `last_active_at_ms` is `None`).
    async fn seed_user(store: &InMemoryUserStore, id: &str, email: &str) {
        store
            .register(
                NewUser {
                    email: email.to_string(),
                    password: "password123".to_string(),
                    display_name: format!("User {id}"),
                },
                id.to_string(),
                NOW - 2 * WINDOW_MS,
            )
            .await
            .expect("seed register");
    }

    fn deps(
        store: Arc<InMemoryUserStore>,
        mailer: Arc<MockReengagementMailer>,
    ) -> ReengagementDeps {
        ReengagementDeps {
            user_store: store as Arc<dyn UserStore>,
            mailer: mailer as Arc<dyn ReengagementMailer>,
            dormant_after_ms: WINDOW_MS,
            reengagement_cooldown_ms: WINDOW_MS,
            batch_limit: 100,
        }
    }

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)] // test: store/mailer outlive the guard block intentionally
    async fn reengagement_emails_only_dormant_not_recently_reengaged() {
        let store = Arc::new(InMemoryUserStore::new());

        // Dormant + never re-engaged → should be emailed.
        seed_user(&store, "dormant", "dormant@x.com").await;

        // Recently active → fails the dormancy filter (FUTURE > now - window).
        seed_user(&store, "active", "active@x.com").await;
        store
            .touch_last_active("active", FUTURE)
            .await
            .expect("touch active");

        // Dormant but re-engaged recently → fails the cooldown filter.
        seed_user(&store, "cooled", "cooled@x.com").await;
        store
            .mark_reengagement_sent("cooled", FUTURE)
            .await
            .expect("mark cooled");

        let mailer = Arc::new(MockReengagementMailer::default());
        let mut reg = FunctionRegistry::new();
        register_reengagement(&mut reg, deps(Arc::clone(&store), Arc::clone(&mailer)))
            .expect("register");

        let result = reg
            .invoke(
                REENGAGEMENT_FUNCTION_ID,
                json!({}),
                "run-reengage".to_string(),
                NOW,
            )
            .await
            .expect("invoke");

        assert_eq!(result, json!({"scanned": 1, "reengaged": 1}));

        {
            let sent = mailer.sent.lock().expect("lock");
            assert_eq!(sent.len(), 1, "exactly one email sent");
            assert_eq!(sent[0].0, "dormant@x.com");
            assert!(!sent[0].1.is_empty(), "body must be non-empty");
        }

        // Fire-once: the dormant user is now stamped reengaged (at the
        // handler's wall-clock `now_ms`), so the cooldown filter would exclude
        // them on the next run.
        let dormant_user = store.get_by_id("dormant").await.expect("get dormant");
        assert!(
            dormant_user.last_reengagement_sent_at_ms.is_some(),
            "dormant user must be stamped reengaged (fire-once)"
        );
    }

    #[tokio::test]
    async fn reengagement_respects_batch_limit() {
        let store = Arc::new(InMemoryUserStore::new());
        for n in 0..5 {
            seed_user(&store, &format!("d{n}"), &format!("d{n}@x.com")).await;
        }

        let mailer = Arc::new(MockReengagementMailer::default());
        let mut d = deps(Arc::clone(&store), Arc::clone(&mailer));
        d.batch_limit = 2;

        let def = reengagement_function(d);
        let mut reg = FunctionRegistry::new();
        reg.register(def).expect("register");

        let result = reg
            .invoke(
                REENGAGEMENT_FUNCTION_ID,
                json!({}),
                "run-limit".to_string(),
                NOW,
            )
            .await
            .expect("invoke");

        assert_eq!(result, json!({"scanned": 2, "reengaged": 2}));
        assert_eq!(mailer.sent.lock().expect("lock").len(), 2);
    }

    #[tokio::test]
    async fn reengagement_per_user_mail_failure_does_not_abort_run() {
        let store = Arc::new(InMemoryUserStore::new());
        seed_user(&store, "a", "a@x.com").await;
        seed_user(&store, "b", "b@x.com").await;
        seed_user(&store, "c", "c@x.com").await;

        // Force a failure for the middle recipient.
        let mailer = Arc::new(MockReengagementMailer {
            sent: Mutex::new(Vec::new()),
            fail_for: Some("b@x.com".to_string()),
        });

        let mut reg = FunctionRegistry::new();
        register_reengagement(&mut reg, deps(Arc::clone(&store), Arc::clone(&mailer)))
            .expect("register");

        let result = reg
            .invoke(
                REENGAGEMENT_FUNCTION_ID,
                json!({}),
                "run-fail".to_string(),
                NOW,
            )
            .await
            .expect("run must not abort on a per-user mail failure");

        // All three scanned; the failing user is not counted as reengaged.
        assert_eq!(result, json!({"scanned": 3, "reengaged": 2}));

        // All three were attempted (mock records before failing).
        assert_eq!(mailer.sent.lock().expect("lock").len(), 3);

        // Only the successful sends are stamped; the failed one is not.
        assert!(
            store
                .get_by_id("a")
                .await
                .expect("a")
                .last_reengagement_sent_at_ms
                .is_some(),
            "successful send must stamp user a"
        );
        assert!(
            store
                .get_by_id("b")
                .await
                .expect("b")
                .last_reengagement_sent_at_ms
                .is_none(),
            "failed send must NOT stamp user b"
        );
        assert!(
            store
                .get_by_id("c")
                .await
                .expect("c")
                .last_reengagement_sent_at_ms
                .is_some(),
            "successful send must stamp user c"
        );
    }

    #[test]
    fn reengagement_function_triggers_and_id() {
        let store = Arc::new(InMemoryUserStore::new());
        let mailer = Arc::new(MockReengagementMailer::default());
        let def = reengagement_function(deps(store, mailer));
        assert_eq!(def.id, REENGAGEMENT_FUNCTION_ID);
        assert_eq!(def.triggers, vec![Trigger::Cron("0 9 * * *".to_string())]);
    }
}
