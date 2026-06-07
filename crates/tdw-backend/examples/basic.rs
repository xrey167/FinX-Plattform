//! Offline, no-network, no-serve example for `tdw-backend`.
//!
//! The minimal dual-facade demo: it exercises BOTH facades fully in-process,
//! without binding any socket or spawning a long-running server.
//!
//!  * async `Backend` — dispatch a `RunQuery` op directly through the in-memory
//!    daemon composition root (`dispatch`, not `serve`);
//!  * sync `AgentBackend` — list the MCP tools and run an `initialize` JSON-RPC
//!    line against the embedded MCP server.
//!
//! For the fuller loopback-served end-to-end demos, see `trading_consumer.rs` and
//! `learning_consumer.rs`.
//!
//! Run with: `cargo run -p tdw-backend --example tdw_backend_basic`

use tdw_backend::prelude::*;
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- async data facade: dispatch an op in-process (no socket) ---
    let backend = Backend::in_memory_for_tests().await;
    println!(
        "[data] in-memory backend with {} providers",
        backend.registry().entries().len()
    );

    let env = OpEnvelope::new(
        SessionId::new("session-backend-example")?,
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
    );
    let events = backend.dispatch(env).await;
    let terminal = match events.last() {
        Some(EventMsg::Completed { .. }) => "Completed",
        Some(EventMsg::Failed { .. }) => "Failed",
        _ => "other",
    };
    println!(
        "[data] dispatch emitted {} events, terminal={terminal}",
        events.len()
    );

    // --- sync agent/MCP facade: tools + an initialize handshake (no daemon) ---
    let mut agent = AgentBackend::from_config(&BackendConfig::default())?;
    // `list_tools` reports the attached tdw-agent REGISTRY tool resources (0 with
    // no registry); the built-in MCP catalog is reached via `handle_mcp_line`.
    println!("[agent] registry tools: {}", agent.list_tools().len());

    let initialize = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"example","version":"1.0.0"}}}"#;
    let responses = agent.handle_mcp_line(initialize);
    println!(
        "[agent] initialize produced {} response line(s)",
        responses.len()
    );

    Ok(())
}
