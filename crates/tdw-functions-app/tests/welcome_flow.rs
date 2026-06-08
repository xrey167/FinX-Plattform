//! End-to-end composition test for the user-lifecycle integration tier.
//!
//! This is a **library-level** integration test (no network, no live SMTP, no
//! daemon process). It proves the three slices fit together at runtime by
//! driving the real registration → event → function → mailer path:
//!
//! 1. **Register** a user through the actual service dispatcher
//!    ([`tdw_service_api::dispatch_op`] with [`Op::RegisterUser`], `identity`
//!    feature). This is the same code path the daemon runs.
//! 2. **Capture** the `user.created` event the dispatcher emits onto the
//!    in-memory outbox — the canonical, non-secret [`UserCreatedPayload`]
//!    projection (`user_id` / `email` / `created_at_ms`).
//! 3. **Route + invoke** the welcome function: build a [`FunctionRegistry`],
//!    register the application functions ([`register_app_functions`]) with a
//!    mock [`WelcomeMailer`], and invoke [`WELCOME_FUNCTION_ID`] with the
//!    captured payload. The captured event payload is fed in verbatim — the two
//!    halves meet at the `user.created` payload contract.
//! 4. **Assert** the mock mailer received exactly one welcome send to the
//!    registered user's email, and that the [`EventRouter`] built from the
//!    registry routes `user.created` to the welcome function id.
//!
//! The function-invoke path used here is [`FunctionRegistry::invoke`] rather
//! than [`EventRouter::dispatch`]: `dispatch` *enqueues* a worker job onto a
//! [`tdw_worker::DurableWorkerQueue`] (the worker process then invokes the
//! function), whereas `invoke` runs the function body directly. For an
//! in-process composition assertion, `invoke` is the function-execution
//! entrypoint; `EventRouter::subscribers` is used to assert the routing
//! contract that `dispatch` relies on. See the test-suite report for the full
//! composition analysis.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use tdw_functions::FunctionRegistry;
use tdw_functions::event_wiring::EventRouter;
use tdw_functions_app::{
    FunctionsAppError, USER_CREATED_EVENT, WELCOME_FUNCTION_ID, WelcomeMailer,
    register_app_functions,
};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
use tdw_service_api::AppState;
use tdw_service_api::user_events::USER_CREATED_EVENT_TYPE;

/// Recording mock [`WelcomeMailer`] capturing every `(to_email, body)` send.
#[derive(Default)]
struct MockWelcomeMailer {
    sent: Mutex<Vec<(String, String)>>,
}

impl WelcomeMailer for MockWelcomeMailer {
    fn send_welcome(&self, to_email: &str, body: &str) -> Result<(), FunctionsAppError> {
        self.sent
            .lock()
            .expect("mock mailer lock")
            .push((to_email.to_string(), body.to_string()));
        Ok(())
    }
}

/// Build an [`OpEnvelope`] for the dispatcher (mirrors the dispatcher unit
/// tests' `make_envelope`).
fn make_envelope(op: Op) -> OpEnvelope {
    OpEnvelope::new(
        SessionId::new("welcome-flow-session").expect("session id"),
        1,
        ActorRef {
            actor_id: "user:test".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        op,
    )
}

/// Register → `user.created` → `EventRouter` routing → welcome function →
/// mock mailer, all in one in-process composition.
#[tokio::test]
async fn register_user_drives_welcome_email_end_to_end() {
    const REGISTERED_EMAIL: &str = "newuser@example.com";

    // (a) InMemoryUserStore (via AppState::in_memory_for_tests) + mock mailer.
    // `in_memory_for_tests` wires an `InMemoryUserStore` and a local-dev policy
    // whose principal holds the `analyst` role that authorizes `UserRegister`.
    let state = AppState::in_memory_for_tests().await;
    let mailer = Arc::new(MockWelcomeMailer::default());

    // (b) Register the user through the real service dispatcher.
    let env = make_envelope(Op::RegisterUser {
        id: "user-e2e-1".to_string(),
        email: REGISTERED_EMAIL.to_string(),
        password: "correct horse battery".to_string(),
        display_name: "New User".to_string(),
        now_ms: 1_700_000_000_000,
    });
    let events = tdw_service_api::dispatch_op(&state, env).await;
    assert_eq!(events.len(), 2, "Started + terminal event expected");
    assert!(
        matches!(&events[1], tdw_protocol::EventMsg::Completed { .. }),
        "registration must complete, got {:?}",
        events[1]
    );

    // (c) Capture the emitted `user.created` payload from the outbox — the same
    // relay path the EventSink uses. This is the canonical projection a real
    // event consumer would receive.
    let captured_payload: Value = {
        let outbox = state.outbox.lock().expect("outbox lock");
        let pending = outbox.pending_after(0);
        pending
            .iter()
            .find(|record| record.envelope.event_type == USER_CREATED_EVENT_TYPE)
            .unwrap_or_else(|| panic!("a user.created envelope must be emitted; got {pending:?}"))
            .envelope
            .payload
            .clone()
    };
    // The payload carries exactly the contract the welcome function consumes.
    assert_eq!(captured_payload["user_id"], "user-e2e-1");
    assert_eq!(captured_payload["email"], REGISTERED_EMAIL);
    assert!(
        captured_payload.get("password").is_none()
            && captured_payload.get("password_hash").is_none(),
        "the emitted event must never carry secret material: {captured_payload}"
    );

    // (d) Build the function registry, register the welcome function with the
    // mock mailer, and build the EventRouter from that registry.
    let mut registry = FunctionRegistry::new();
    register_app_functions(&mut registry, Arc::clone(&mailer) as Arc<dyn WelcomeMailer>)
        .expect("register app functions");
    let router = EventRouter::from_registry(&registry);

    // (f) Routing contract: EventRouter routes `user.created` to the welcome fn.
    // (Asserted before invocation so a routing regression fails loudly even if
    // the direct invoke below were to change.)
    assert_eq!(USER_CREATED_EVENT, USER_CREATED_EVENT_TYPE);
    assert!(
        router
            .subscribers(USER_CREATED_EVENT)
            .contains(&WELCOME_FUNCTION_ID.to_string()),
        "EventRouter must route {USER_CREATED_EVENT} to {WELCOME_FUNCTION_ID}; got {:?}",
        router.subscribers(USER_CREATED_EVENT)
    );

    // Drive the welcome function with the captured payload. `invoke` is the
    // direct function-execution entrypoint (EventRouter::dispatch would instead
    // enqueue a worker job for out-of-process execution).
    let run_id = format!("welcome-flow:{WELCOME_FUNCTION_ID}");
    let result = registry
        .invoke(
            WELCOME_FUNCTION_ID,
            captured_payload,
            run_id,
            1_700_000_000_000,
        )
        .await
        .expect("welcome function invoke");
    assert_eq!(result["sent"], true);
    assert_eq!(result["to"], REGISTERED_EMAIL);

    // (e) The mock mailer received exactly one welcome send to the registered
    // user's email, with a non-empty rendered body.
    let sent = mailer.sent.lock().expect("mock mailer lock");
    assert_eq!(sent.len(), 1, "exactly one welcome email must be sent");
    assert_eq!(sent[0].0, REGISTERED_EMAIL);
    assert!(!sent[0].1.is_empty(), "welcome body must be non-empty");
}
