//! End-to-end warehouse-trading consumer embedding `tdw-backend`.
//!
//! This example shows a single process embedding **both** facades and proving
//! the dual-facade contract from the library (not just the binary):
//!
//! * the **async** [`Backend`] serves the daemon in-process on an ephemeral
//!   loopback port and answers ops submitted in-process and over a loopback
//!   [`DaemonClient`];
//! * the **sync** [`AgentBackend`] reaches that *same* daemon over loopback via
//!   [`AgentBackend::with_daemon_addr`] and drives MCP JSON-RPC
//!   (`initialize` → `tools/list` → `tools/call` for `tdw.daemon.query.submit`).
//!
//! It also touches the knowledge and agent capability groups so all four groups
//! from the prelude are exercised. It is fully offline and deterministic — no
//! Docker, no network beyond loopback — and must run to completion without
//! hanging.
//!
//! Run it with:
//!
//! ```text
//! cargo run --example trading_consumer -p tdw-backend --target-dir target
//! ```

// The prelude is the ONLY backend import: this proves the prelude surface is
// sufficient to build a real consumer end to end.
use tdw_backend::prelude::*;

use std::error::Error;
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    println!("== tdw-backend trading consumer ==");

    // --- 1. Build the async data backend on an ephemeral loopback TCP port. ---
    // `in_memory_for_tests` gives deterministic offline engines; the daemon is
    // bound on `127.0.0.1:0` so the OS assigns a free port.
    let mut cfg = BackendConfig::default();
    cfg.tdw.daemon.transport = tdw_config::DaemonTransport::Tcp;
    cfg.tdw.daemon.tcp_bind = Some("127.0.0.1:0".to_string());

    let mut backend = Backend::in_memory_for_tests().await;
    println!(
        "[data] built in-memory backend ({} providers)",
        backend.registry().entries().len()
    );

    // --- 2. Serve the daemon in-process (Both-style) and read its address. ---
    backend.serve(&cfg).await?;
    let addr = backend
        .bound_addr()
        .ok_or("served daemon must expose its bound address")?
        .to_string();
    println!("[data] daemon serving on loopback {addr}");

    // --- 3a. Submit an Op in-process via the SubmissionHandle (no socket). ----
    let submission = backend
        .submission_handle()
        .ok_or("a served daemon exposes a submission handle")?;
    let in_proc_env = run_query_envelope("session-inproc", "select 1");
    submission
        .submit(in_proc_env)
        .map_err(|error| format!("in-process submit failed: {error:?}"))?;
    println!("[data] submitted an in-process RunQuery op via the submission handle");

    // --- 3b. Submit a RunQuery over a loopback DaemonClient (the data path). --
    // `submit_and_wait` is blocking, so run it off the async worker and bound it
    // with a timeout so the example can never hang.
    let client_addr = addr.clone();
    let data_path = tokio::task::spawn_blocking(move || {
        let client = DaemonClient::new(
            DaemonClientConfig::tcp(client_addr).with_timeout(Duration::from_secs(2)),
        );
        client.submit_and_wait(&run_query_envelope("session-loopback", "select 1"))
    });
    let data_path = tokio::time::timeout(Duration::from_secs(3), data_path)
        .await
        .map_err(|_| "loopback data-path submit hung")???;
    let completed = data_path
        .events
        .iter()
        .any(|event| matches!(event, EventMsg::Completed { .. }));
    println!(
        "[data] loopback DaemonClient round-trip: {} events, terminal Completed = {completed}",
        data_path.events.len()
    );

    // --- 4. The agent/MCP path reaching the SAME daemon over loopback. -------
    // `AgentBackend` is sync and its daemon tool does a blocking TCP submit, so
    // the whole MCP drive runs on a blocking thread, bounded by a timeout.
    let agent_addr = addr.clone();
    let mcp = tokio::task::spawn_blocking(move || drive_agent_mcp(&agent_addr));
    let mcp_lines = tokio::time::timeout(Duration::from_secs(4), mcp)
        .await
        .map_err(|_| "agent/MCP drive hung")??
        .map_err(|message| -> Box<dyn Error + Send + Sync> { message.into() })?;
    for line in &mcp_lines {
        println!("[agent] mcp <- {line}");
    }

    // --- 5. Touch the remaining capability groups from the prelude. ----------
    // Knowledge group (async index on the data facade):
    backend
        .knowledge_index(KnowledgeDocument {
            id: "doc-trade-1".to_string(),
            body: "AAPL equity momentum desk note".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: tdw_kg::EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: vec!["asset:equity".to_string()],
            source: None,
            plane: None,
            as_of: None,
            author: None,
            mentions: Vec::new(),
        })
        .await?;
    let hits = backend.knowledge_search("AAPL momentum", 1).await?;
    println!(
        "[knowledge] indexed 1 doc; search top hit = {}",
        hits.first().map_or("<none>", |hit| hit.id.as_str())
    );

    println!("[done] all four capability groups exercised; shutting down.");

    // --- 6. Shut the daemon down cleanly (bounded so it cannot hang). --------
    tokio::time::timeout(Duration::from_secs(3), backend.shutdown())
        .await
        .map_err(|_| "daemon shutdown hung")??;
    println!(
        "[data] daemon shut down; bound_addr cleared = {}",
        backend.bound_addr().is_none()
    );

    Ok(())
}

/// Build the embedded agent/MCP facade, exercise the agent capability group, and
/// drive an `initialize` → `tools/list` → `tools/call` JSON-RPC session against
/// the daemon at `addr`. Returns the collected response lines.
///
/// Runs on a blocking thread: the daemon-backed tool submits over loopback TCP
/// synchronously, which must not happen on the async worker.
fn drive_agent_mcp(addr: &str) -> Result<Vec<String>, String> {
    let cfg = BackendConfig::default();
    let mut agent = AgentBackend::from_config(&cfg)
        .map_err(|error| format!("build agent backend: {error}"))?
        .with_daemon_addr(addr);

    // Agent capability group: register an agent card and list the registry tools.
    agent.upsert_agent(tdw_agent::sample_agent_card());
    println!(
        "[agent] stored agent card 'market-researcher' present = {}; registry tools = {:?}",
        agent.agent("market-researcher").is_some(),
        agent.list_tools()
    );

    let mut responses = Vec::new();

    // initialize
    let init = agent.handle_mcp_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"trading-consumer","version":"1.0.0"}}}"#,
    );
    responses.extend(init);
    // The MCP spec requires the initialized notification before normal requests.
    let _ = agent.handle_mcp_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);

    // tools/list — must surface the daemon query tool.
    let list = agent.handle_mcp_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    if let Some(first) = list.first()
        && !first.contains("tdw.daemon.query.submit")
    {
        return Err("tools/list did not surface tdw.daemon.query.submit".to_string());
    }
    responses.extend(list);

    // tools/call the daemon query tool — this reaches the daemon over loopback.
    let call = agent.handle_mcp_line(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tdw.daemon.query.submit","arguments":{"sql":"select 1","session_id":"session-mcp-example"}}}"#,
    );
    responses.extend(call);

    Ok(responses)
}

/// A `RunQuery` [`OpEnvelope`] mirroring the Phase-1 data-test construction
/// shape (a `User` actor in the `default` tenant).
fn run_query_envelope(session: &str, sql: &str) -> OpEnvelope {
    OpEnvelope::new(
        tdw_protocol::SessionId::new(session.to_string()).expect("valid session id"),
        1,
        tdw_protocol::ActorRef {
            actor_id: "user:trading-consumer".to_string(),
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
