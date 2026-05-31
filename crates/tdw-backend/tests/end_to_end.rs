//! End-to-end integration test for the dual-facade contract.
//!
//! Asserts the full library wiring exercised by `examples/trading_consumer.rs`:
//! the async [`Backend`] serves the daemon in-process on an ephemeral loopback
//! port, a loopback [`DaemonClient`] submits a `RunQuery` and observes a terminal
//! [`EventMsg::Completed`], and the sync [`AgentBackend`] — pointed at the *same*
//! daemon via [`AgentBackend::with_daemon_addr`] — drives MCP JSON-RPC so the
//! full **MCP → loopback → daemon** path returns query evidence proving it
//! reached the daemon.
//!
//! Deterministic and offline (no Docker/network beyond loopback); every wait is
//! bounded by a [`tokio::time::timeout`] so the test cannot hang. Passes on
//! default features (the `transport-tcp` default of `tdw-app-server` makes the
//! loopback TCP path available).

use std::time::Duration;

use tdw_backend::prelude::*;

/// A `RunQuery` [`OpEnvelope`] mirroring the Phase-1 data-test construction shape.
fn run_query_envelope(session: &str, sql: &str) -> OpEnvelope {
    OpEnvelope::new(
        tdw_protocol::SessionId::new(session.to_string()).expect("valid session id"),
        1,
        tdw_protocol::ActorRef {
            actor_id: "user:e2e".to_string(),
            kind: tdw_protocol::ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::RunQuery {
            sql: sql.to_string(),
            plan_id: None,
            cost_hint: None,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dual_facade_data_and_mcp_paths_reach_the_same_daemon() {
    // --- Serve the daemon on an ephemeral loopback port. -------------------
    let mut cfg = BackendConfig::default();
    cfg.tdw.daemon.transport = tdw_config::DaemonTransport::Tcp;
    cfg.tdw.daemon.tcp_bind = Some("127.0.0.1:0".to_string());

    let mut backend = Backend::in_memory_for_tests().await;
    backend.serve(&cfg).await.expect("serve should start");

    let addr = backend
        .bound_addr()
        .expect("a served daemon exposes its bound address")
        .to_string();
    assert!(
        addr.parse::<std::net::SocketAddr>().is_ok(),
        "bound address must resolve to a concrete socket addr: {addr}"
    );
    assert_ne!(
        addr.rsplit(':').next().expect("port segment"),
        "0",
        "ephemeral bind must resolve to a concrete OS-assigned port"
    );

    // --- Data path: loopback DaemonClient submit → terminal Completed. ------
    let client_addr = addr.clone();
    let submission = tokio::task::spawn_blocking(move || {
        let client = DaemonClient::new(
            DaemonClientConfig::tcp(client_addr).with_timeout(Duration::from_secs(2)),
        );
        client.submit_and_wait(&run_query_envelope("session-e2e-data", "select 1"))
    });
    let submission = tokio::time::timeout(Duration::from_secs(3), submission)
        .await
        .expect("loopback data-path submit must not hang")
        .expect("spawn_blocking join")
        .expect("loopback submission should reach the in-process daemon");
    assert!(
        submission
            .events
            .iter()
            .any(|event| matches!(event, EventMsg::Completed { .. })),
        "the daemon must emit a terminal Completed event for the RunQuery op"
    );

    // --- Agent/MCP path: drive the full MCP → loopback → daemon round-trip. -
    // `AgentBackend` is sync and its daemon tool submits over loopback TCP, so
    // the whole drive runs on a blocking thread and is bounded by a timeout.
    let agent_addr = addr.clone();
    let mcp = tokio::task::spawn_blocking(move || {
        let cfg = BackendConfig::default();
        let mut agent = AgentBackend::from_config(&cfg)
            .expect("agent backend should build")
            .with_daemon_addr(&agent_addr);

        // initialize
        let init = agent.handle_mcp_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"1.0.0"}}}"#,
        );
        assert_eq!(init.len(), 1, "initialize yields exactly one response");
        assert!(
            init[0].contains("\"protocolVersion\""),
            "initialize response must carry a protocolVersion: {}",
            init[0]
        );
        let _ = agent.handle_mcp_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

        // tools/list must surface the daemon query tool.
        let list = agent.handle_mcp_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert_eq!(list.len(), 1, "tools/list yields exactly one response");
        assert!(
            list[0].contains("tdw.daemon.query.submit"),
            "tools/list must surface the daemon query tool: {}",
            list[0]
        );

        // tools/call the daemon query tool — reaches the daemon over loopback.
        let call = agent.handle_mcp_line(
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.daemon.query.submit","arguments":{"sql":"select 1","session_id":"session-e2e-mcp"}}}"#,
        );
        call.last().cloned()
    });
    let call_response = tokio::time::timeout(Duration::from_secs(4), mcp)
        .await
        .expect("agent/MCP drive must not hang")
        .expect("spawn_blocking join")
        .expect("tools/call should yield a response line");

    // The response must be a well-formed JSON-RPC result carrying daemon query
    // evidence proving the MCP → loopback → daemon path completed.
    let parsed: serde_json::Value =
        serde_json::from_str(&call_response).expect("tools/call response must be valid JSON");
    assert_eq!(parsed["id"], 3, "response correlates with the request id");
    let result = &parsed["result"];
    assert_eq!(
        result["isError"], false,
        "the daemon query tool must succeed: {call_response}"
    );
    let structured = &result["structuredContent"];
    assert_eq!(structured["tool"], "tdw.daemon.query.submit");
    assert_eq!(
        structured["daemon"]["transport"], "tcp",
        "the embedded MCP server reached the daemon over loopback TCP"
    );
    assert_eq!(structured["extra"]["sql"], "select 1");
    assert!(
        structured["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["type"] == "completed")),
        "the daemon must emit a terminal Completed event over the MCP path: {call_response}"
    );

    // --- Bounded, clean shutdown. ------------------------------------------
    tokio::time::timeout(Duration::from_secs(3), backend.shutdown())
        .await
        .expect("shutdown must not hang")
        .expect("shutdown should return Ok");
    assert!(
        backend.bound_addr().is_none(),
        "the daemon handle must be cleared after shutdown"
    );
}
