//! Offline, no-network example for `tdw-service-api`.
//!
//! Builds an in-memory `AppState` (offline engines + a synthesized local-dev
//! policy), lists the offline provider registry, and dispatches a `RunQuery` op
//! through the full secure request path (`dispatch_op`) to observe the emitted
//! `Started` + terminal events. No daemon socket, no network, no Docker.
//!
//! Run with: `cargo run -p tdw-service-api --example tdw_service_api_basic`

use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};
use tdw_service_api::{AppState, dispatch_op, list_providers};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. The default (no-feature) registry: exactly the 3 offline providers.
    let providers = list_providers()?;
    println!("offline providers ({}):", providers.len());
    for p in &providers {
        println!("  - {}/{} ({:?})", p.provider, p.endpoint, p.kind);
    }

    // 2. Offline composition root. `in_memory_for_tests` wires SQLite-in-memory,
    //    a temp JSONL rollout, and a synthesized local-dev policy so dispatch
    //    resolves without OIDC configuration.
    let state = AppState::in_memory_for_tests().await;
    let policy = state.policy.as_ref().expect("local-dev policy attached");
    println!(
        "policy principal={} roles={:?}",
        policy.auth.claims.sub, policy.auth.claims.roles
    );

    // 3. Dispatch a RunQuery op through the secure request path.
    let env = OpEnvelope::new(
        SessionId::new("session-example")?,
        1,
        ActorRef {
            actor_id: "user:example".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        },
    );
    let events = dispatch_op(&state, env).await;
    println!("dispatch emitted {} events", events.len());
    match events.last() {
        Some(EventMsg::Completed { result, .. }) => {
            println!(
                "completed; evidence endpoint = {}",
                result
                    .as_ref()
                    .map_or(serde_json::Value::Null, |v| v["evidence"]["endpoint"]
                        .clone(),)
            );
        }
        Some(EventMsg::Failed { error, .. }) => println!("failed: {error}"),
        other => println!("unexpected terminal event: {other:?}"),
    }

    Ok(())
}
