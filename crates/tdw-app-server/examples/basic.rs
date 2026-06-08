//! Offline, no-network example for `tdw-app-server`.
//!
//! Wires an in-example `Dispatcher` + `EventSink` into a `ServiceLoop` via
//! `service_channel`, submits one `OpEnvelope` over the in-process handle, and
//! drives `run_once` to observe the emitted events. Also validates a daemon
//! endpoint. No socket is bound — this is the in-process path the network
//! transports sit on top of.
//!
//! Run with: `cargo run -p tdw-app-server --example tdw_app_server_basic`

use async_trait::async_trait;
use tdw_app_server::{
    DaemonEndpoint, DaemonTransport, Dispatcher, EventSink, SinkResult, service_channel,
    validate_endpoint,
};
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

/// Minimal dispatcher: emit `Started` then a terminal `Completed` per op.
struct EchoDispatcher;

#[async_trait]
impl Dispatcher for EchoDispatcher {
    async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg> {
        vec![
            EventMsg::Started {
                op_id: env.op_id.clone(),
            },
            EventMsg::Completed {
                op_id: env.op_id,
                summary: Some("echoed".to_string()),
                result: None,
            },
        ]
    }
}

/// No-op sink: a real daemon persists to outbox/rollout/cost ledger here.
struct CountingSink;

#[async_trait]
impl EventSink for CountingSink {
    async fn persist_event(
        &self,
        _env: &OpEnvelope,
        event: &EventMsg,
        sequence: u64,
    ) -> SinkResult<()> {
        println!("  persist seq={sequence} event={event:?}");
        Ok(())
    }

    async fn record_cost(&self, _env: &OpEnvelope, backend: &str) -> SinkResult<()> {
        println!("  record_cost backend={backend}");
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Endpoint validation: the loopback TCP default is accepted; a UDS address
    // with a parent-path segment is rejected.
    let ok = validate_endpoint(&DaemonEndpoint {
        transport: DaemonTransport::Tcp,
        address: "127.0.0.1:7878".to_string(),
    });
    let bad = validate_endpoint(&DaemonEndpoint {
        transport: DaemonTransport::Uds,
        address: "../escape.sock".to_string(),
    });
    println!("validate tcp loopback: {ok:?}; validate ../uds: {bad:?}");

    // Wire the durable service loop and submit one op in-process.
    let (handle, _events_rx, mut service_loop) = service_channel(EchoDispatcher, CountingSink);
    // `SubmissionError` is not `std::error::Error`, so expect rather than `?`.
    handle
        .submit(OpEnvelope::new(
            SessionId::new("session-example")?,
            1,
            ActorRef {
                actor_id: "user:example".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::RunQuery {
                sql: "select 1".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        ))
        .expect("submission channel is open");

    let emitted = service_loop.run_once().await.expect("loop produced events");
    println!("service loop emitted {} events", emitted.len());
    assert!(matches!(emitted.first(), Some(EventMsg::Started { .. })));
    assert!(matches!(emitted.last(), Some(EventMsg::Completed { .. })));

    Ok(())
}
