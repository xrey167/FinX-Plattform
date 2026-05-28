//! P6 daemon smoke test: exercises the full daemon-binary wiring in-process.
//!
//! This test:
//! 1. Builds an `AppState` backed by in-memory stores and attaches an analyst
//!    policy so dispatches succeed.
//! 2. Constructs `service_channel` + relay + `serve_tcp` on a kernel-assigned
//!    ephemeral port (`127.0.0.1:0`).
//! 3. Opens a client `TcpStream`, submits a framed `Op::RunQuery { sql: "select 1" }`
//!    and reads framed `EventMsg` responses.
//! 4. Asserts that both `Started` and `Completed` events are received within 3 s.
//! 5. Cancels all tasks cleanly.

use std::time::Duration;

use tdw_app_server::{CancellationToken, service_channel, serve_tcp, spawn_inmemory_relay};
use tdw_auth_oidc::{JwksKey, JwtClaims};
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};
use tdw_service_api::{AppState, IngressAuthContext, PolicyEnforcementConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

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
        hook_execution: Default::default(),
        mask_rules: Vec::new(),
    }
}

fn make_run_query_envelope() -> OpEnvelope {
    OpEnvelope::new(
        SessionId::new("session-daemon-smoke").expect("session id"),
        1,
        ActorRef {
            actor_id: "user:test".to_string(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_smoke_run_query_receives_started_and_completed() {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        // Build in-memory state with analyst policy.
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(analyst_policy());

        // Wire up the service channel.
        let (handle, events_rx, service_loop) =
            service_channel(state.clone(), state.clone());

        let cancel = CancellationToken::new();

        // Spawn the in-memory relay.
        let relay = spawn_inmemory_relay(
            state.outbox.clone(),
            state.bus.clone(),
            Duration::from_millis(10),
            cancel.clone(),
        );

        // Bind TCP listener on ephemeral port.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let addr = listener.local_addr().expect("local addr");

        // Spawn the TCP transport.
        let cancel_tcp = cancel.clone();
        tokio::spawn(async move {
            serve_tcp(listener, handle, events_rx, cancel_tcp)
                .await
                .expect("serve_tcp");
        });

        // Spawn the service loop.
        let cancel_serve = cancel.clone();
        tokio::spawn(async move {
            tdw_app_server::serve(service_loop, relay, cancel_serve)
                .await
                .expect("serve");
        });

        // Give the listener a moment to become ready.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Connect a client and submit Op::RunQuery.
        let mut client = TcpStream::connect(addr).await.expect("connect");
        let envelope = make_run_query_envelope();
        let submitted_op_id = envelope.op_id.clone();

        write_frame(&mut client, &envelope).await.expect("write frame");

        // Read events until we have both Started and Completed for our op.
        let mut got_started = false;
        let mut got_completed = false;

        for _ in 0..10 {
            match read_frame(&mut client).await.expect("read frame") {
                Some(bytes) => {
                    let event: EventMsg =
                        serde_json::from_slice(&bytes).expect("deserialize event");
                    match &event {
                        EventMsg::Started { op_id } if op_id == &submitted_op_id => {
                            got_started = true;
                        }
                        EventMsg::Completed { op_id, .. } if op_id == &submitted_op_id => {
                            got_completed = true;
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

        assert!(got_started, "expected Started event for the submitted op");
        assert!(got_completed, "expected Completed event for the submitted op");
    })
    .await;

    result.expect("daemon smoke test timed out");
}
