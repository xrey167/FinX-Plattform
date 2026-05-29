//! P7 end-to-end integration test: real Postgres backend.
//!
//! Exercises the full daemon chain:
//!   AppState (Pg-backed relational engine)
//!   → serve TCP
//!   → AppClient submits Ops
//!   → daemon dispatch
//!   → relational query against real Postgres (`select 1`)
//!   → outbox persisted
//!   → relay published to bus
//!   → rollout archive
//!   → graceful shutdown
//!
//! # Gating
//!
//! - **Feature gate**: compiled only when `tdw-service-api` is built with
//!   `real-postgres` (which activates `tdw-storage-postgres/postgres`). The
//!   `[dev-dependencies]` entry in `tdw-service/Cargo.toml` enables this
//!   feature unconditionally so the test binary is always compiled. The test
//!   body itself skips at runtime when `TDW_POSTGRES_TEST_URL` is unset.
//!
//! - **Runtime gate**: reads `TDW_POSTGRES_TEST_URL`. If unset the test
//!   returns immediately with a clear log line — matching the g013 pattern —
//!   so CI without Docker still passes (exit 0).
//!
//! # Running locally
//!
//! ```sh
//! docker compose --profile full up -d
//! export TDW_POSTGRES_TEST_URL="postgres://tdw:tdw@127.0.0.1:5432/tdw"
//! cargo test --package tdw-service --test g0xx_daemon_e2e_real_pg -- --nocapture
//! ```
//!
//! # Session store
//!
//! The session store intentionally stays on SQLite-in-memory for this test.
//! The purpose of P7 is to prove the *relational engine* (query path) against
//! a real Postgres instance; swapping the session store to Pg is a separate
//! concern tracked by g013 / future phases.

use std::time::Duration;

use tdw_app_server::{CancellationToken, serve_tcp, service_channel, spawn_inmemory_relay};
use tdw_auth_oidc::{JwksKey, JwtClaims};
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};
use tdw_service_api::{AppState, HookExecutionPolicy, IngressAuthContext, PolicyEnforcementConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// ── helpers ──────────────────────────────────────────────────────────────────

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn analyst_policy() -> PolicyEnforcementConfig {
    PolicyEnforcementConfig {
        auth: IngressAuthContext {
            claims: JwtClaims {
                sub: "alice".to_string(),
                iss: "https://issuer".to_string(),
                aud: "tdw".to_string(),
                kid: "k1".to_string(),
                roles: vec!["analyst".to_string()],
            },
            jwks: vec![JwksKey {
                kid: "k1".to_string(),
                alg: "RS256".to_string(),
            }],
            issuer: "https://issuer".to_string(),
            audience: "tdw".to_string(),
        },
        hooks: Vec::new(),
        hook_execution: HookExecutionPolicy::default(),
        mask_rules: Vec::new(),
    }
}

fn run_query_envelope(session_id: &SessionId, sequence: u64) -> OpEnvelope {
    OpEnvelope::new(
        session_id.clone(),
        sequence,
        ActorRef {
            actor_id: "user:test-e2e".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        },
    )
}

fn shutdown_envelope(session_id: &SessionId, sequence: u64) -> OpEnvelope {
    OpEnvelope::new(
        session_id.clone(),
        sequence,
        ActorRef {
            actor_id: "user:test-e2e".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::Shutdown,
    )
}

async fn write_frame(stream: &mut TcpStream, envelope: &OpEnvelope) -> std::io::Result<()> {
    let json = serde_json::to_vec(envelope).expect("serialize envelope");
    let len = (json.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&json).await?;
    stream.flush().await
}

async fn read_frame(stream: &mut TcpStream) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > 16 * 1024 * 1024 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

// ── test ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e2e_real_postgres_round_trip() {
    let Some(url) = database_url() else {
        eprintln!("skipping P7 e2e: TDW_POSTGRES_TEST_URL not set");
        return;
    };

    let result = tokio::time::timeout(Duration::from_secs(5), run_e2e(url)).await;
    result.expect("P7 e2e test timed out after 5 s — is Postgres reachable?");
}

async fn run_e2e(url: String) {
    // Build AppState with in-memory stores but swap the relational engine to
    // real Postgres. Session store remains SQLite-in-memory: the goal of P7 is
    // to prove the relational query path against a live PgEngine.
    let state = AppState::in_memory_for_tests()
        .await
        .with_policy(analyst_policy())
        .with_real_postgres(&url)
        .await
        .expect("PgEngine connect failed — check TDW_POSTGRES_TEST_URL");

    let session_id = SessionId::new("session-e2e-pg").expect("session id");

    // Wire up the service channel, relay, and TCP listener.
    let (handle, events_rx, service_loop) = service_channel(state.clone(), state.clone());

    let cancel = CancellationToken::new();

    let relay = spawn_inmemory_relay(
        state.outbox.clone(),
        state.bus.clone(),
        Duration::from_millis(50),
        cancel.clone(),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local addr");

    let cancel_tcp = cancel.clone();
    tokio::spawn(async move {
        serve_tcp(listener, handle, events_rx, cancel_tcp)
            .await
            .expect("serve_tcp");
    });

    let cancel_serve = cancel.clone();
    tokio::spawn(async move {
        tdw_app_server::serve(service_loop, relay, cancel_serve)
            .await
            .expect("serve");
    });

    // Give the listener a moment to become ready.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // Connect a TCP client and submit two ops.
    let mut client = TcpStream::connect(addr).await.expect("client connect");

    let query_env = run_query_envelope(&session_id, 1);
    let query_op_id = query_env.op_id.clone();
    write_frame(&mut client, &query_env)
        .await
        .expect("write RunQuery frame");

    let shutdown_env = shutdown_envelope(&session_id, 2);
    write_frame(&mut client, &shutdown_env)
        .await
        .expect("write Shutdown frame");

    // Collect EventMsgs until we have Started + Completed for the RunQuery.
    let mut got_started = false;
    let mut got_completed = false;
    let mut completed_result: Option<serde_json::Value> = None;

    for _ in 0..20 {
        match read_frame(&mut client).await.expect("read frame") {
            Some(bytes) => {
                let event: EventMsg = serde_json::from_slice(&bytes).expect("deserialize EventMsg");
                match &event {
                    EventMsg::Started { op_id } if op_id == &query_op_id => {
                        got_started = true;
                    }
                    EventMsg::Completed {
                        op_id,
                        result: Some(value),
                        ..
                    } if op_id == &query_op_id => {
                        got_completed = true;
                        completed_result = Some(value.clone());
                    }
                    _ => {}
                }
                if got_started && got_completed {
                    break;
                }
            }
            None => break,
        }
    }

    cancel.cancel();

    // ── assertions ──────────────────────────────────────────────────────────

    assert!(got_started, "expected Started event for RunQuery op");
    assert!(got_completed, "expected Completed event for RunQuery op");

    let result = completed_result.expect("Completed must carry a result value");

    // Verify the evidence endpoint matches the dispatcher path.
    assert_eq!(
        result["evidence"]["endpoint"], "tdw.query.run",
        "evidence.endpoint must be tdw.query.run; got: {}",
        result["evidence"]["endpoint"]
    );

    // Verify that real Postgres returned at least one row for `select 1`.
    let rows = result["rows"]
        .as_array()
        .expect("result.rows must be a JSON array");
    assert!(
        !rows.is_empty(),
        "real Postgres must return at least one row for `select 1`"
    );

    // Outbox: at minimum Started + Completed for RunQuery (2 events appended
    // by the service loop before relay marks them dispatched). We snapshot the
    // outbox with pending_after(0) which includes already-dispatched records
    // that the relay has not yet reaped — the relay tick is 50 ms and we may
    // have cancelled before it ran. Accept >= 1 here since the relay may have
    // consumed some by the time we check.
    let outbox_records = state.outbox.lock().expect("outbox lock").pending_after(0);
    // The relay may have dispatched entries already; just assert the outbox
    // was exercised (at least one record exists in the full records, which we
    // confirm by checking the bus below).

    // Bus / relay: check the bus received at least one event from the relay.
    let bus_lag = state.bus.lock().expect("bus lock").lag_since(0);
    assert!(
        bus_lag > 0 || !outbox_records.is_empty(),
        "either the bus must have received events or the outbox must still hold pending records"
    );

    // Rollout archive: the service loop appends frames during dispatch. At
    // least the RunQuery cycle (Started + Completed) should be there.
    let rollout_records = state.rollout.read_all().expect("rollout read_all");
    assert!(
        rollout_records.len() >= 2,
        "rollout archive must contain at least 2 frames (Started + Completed for RunQuery); got {}",
        rollout_records.len()
    );
}
