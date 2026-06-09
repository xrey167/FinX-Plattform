//! Offline `tdw-exec` example: plan a read-only query into protocol events, and
//! show the SQL guard rejecting a mutating query.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-exec --example tdw_exec_basic
//! ```

#![forbid(unsafe_code)]

use tdw_exec::{ExecError, run_headless, try_run_headless};
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

fn user_envelope(sql: &str) -> OpEnvelope {
    OpEnvelope::new(
        SessionId::new("session-1").expect("session id"),
        1,
        ActorRef {
            actor_id: "user".to_string(),
            kind: ActorKind::User,
            tenant_id: None,
        },
        Op::RunQuery {
            sql: sql.to_string(),
            plan_id: None,
            cost_hint: None,
        },
    )
}

fn main() {
    // A read-only SELECT plans into Started + Completed events.
    let run = run_headless(user_envelope("select 1"));
    assert!(matches!(run.events[0], EventMsg::Started { .. }));
    assert!(matches!(run.events[1], EventMsg::Completed { .. }));
    println!("planned read-only query into {} events", run.events.len());

    // The checked entry point accepts the same query.
    assert!(try_run_headless(user_envelope("select 1")).is_ok());

    // ...but rejects a mutating one before emitting anything.
    let denied = try_run_headless(user_envelope("delete from raw.orders"));
    assert_eq!(denied, Err(ExecError::NonReadOnlySql));
    println!("mutating query rejected: {denied:?}");

    // Statement stacking is rejected too.
    let stacked = try_run_headless(user_envelope("select 1; drop table raw.orders"));
    assert_eq!(stacked, Err(ExecError::MultipleStatements));
    println!("statement stacking rejected: {stacked:?}");
}
