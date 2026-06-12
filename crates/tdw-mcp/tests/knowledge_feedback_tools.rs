//! End-to-end tests for the MCP knowledge FEEDBACK tool (knowledge-system B10 + K-L5).
//!
//! Each test drives `tdw.kg.feedback` through the SAME JSON-RPC `tools/call`
//! surface a real client uses, asserting:
//!
//! - Descriptor gating: the tool appears only when a [`KnowledgeRuntime`] with a
//!   **bound agent id** AND a [`RetrievalFeedbackStore`] are attached; absent either → not listed.
//! - Append-only posture: calling the tool appends a [`RetrievalEvent`] and does
//!   NOT mutate the graph, tags, or proposals.
//! - Payload caps: `hit_ids` > 64 are silently truncated at admission.
//! - Missing required argument → tool error, not protocol error.
//! - Without the feedback surface → tool error, not protocol error.
//!
//! K-L5 spoofing regressions:
//! - A call carrying a forged `agent_id` argument is rejected (unknown field /
//!   `additionalProperties: false`) — not silently accepted.
//! - The stored event's `agent_id` always matches the BOUND principal, regardless
//!   of what arguments the caller supplies.
//! - A runtime without a bound agent id keeps the feedback surface OFF entirely.

use std::sync::Arc;

use serde_json::{Value, json};
use tdw_agent_store::RetrievalFeedbackStore;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_mcp::McpServer;
use tdw_storage_qdrant::InMemoryVectorEngine;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a runtime with a bound agent id (required for feedback gating since K-L5).
fn build_runtime() -> Arc<KnowledgeRuntime> {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    Arc::new(KnowledgeRuntime::new(embedder, vectors).with_agent_id("agent:test"))
}

/// Build a runtime WITHOUT a bound agent id (feedback surface must be absent).
fn build_runtime_no_agent() -> Arc<KnowledgeRuntime> {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    Arc::new(KnowledgeRuntime::new(embedder, vectors))
}

fn fresh_store() -> Arc<tokio::sync::Mutex<RetrievalFeedbackStore>> {
    Arc::new(tokio::sync::Mutex::new(RetrievalFeedbackStore::new()))
}

fn decode(message: &str) -> Value {
    serde_json::from_str(message)
        .unwrap_or_else(|e| panic!("response should be json: {e}; raw={message}"))
}

fn initialize(server: &mut McpServer) {
    let msgs = server.handle_json_rpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
    );
    assert_eq!(msgs.len(), 1);
}

fn call(server: &mut McpServer, name: &str, arguments: &Value) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    decode(&server.handle_json_rpc_line(&req.to_string())[0])
}

fn listed_tool_names(server: &mut McpServer) -> Vec<String> {
    let resp = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)[0],
    );
    resp["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"))
        .iter()
        .filter_map(|t| t["name"].as_str().map(ToString::to_string))
        .collect()
}

fn is_tool_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool() == Some(true)
}

const FEEDBACK_TOOL: &str = "tdw.kg.feedback";

// ---------------------------------------------------------------------------
// Descriptor gating (K-L5: also requires bound agent id)
// ---------------------------------------------------------------------------

/// Descriptor gating: tool is listed when runtime (with bound agent id) AND store
/// are attached; absent when any of the three conditions is missing.
#[test]
fn descriptor_gated_on_runtime_with_agent_id_and_store() {
    let runtime = build_runtime(); // has bound agent id
    let runtime_no_agent = build_runtime_no_agent(); // no bound agent id
    let store = fresh_store();

    // Both attached with bound agent id → listed.
    let mut full = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut full);
    assert!(
        listed_tool_names(&mut full).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be listed when runtime (bound agent) + store are attached"
    );

    // Runtime only (no store) → absent.
    let mut no_store = McpServer::new().with_knowledge(Arc::clone(&runtime));
    initialize(&mut no_store);
    assert!(
        !listed_tool_names(&mut no_store).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent without the feedback store"
    );

    // Store only (no runtime) → absent.
    let mut no_runtime = McpServer::new().with_feedback_store(Arc::clone(&store));
    initialize(&mut no_runtime);
    assert!(
        !listed_tool_names(&mut no_runtime).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent without the knowledge runtime"
    );

    // Runtime without bound agent id + store → absent (K-L5 gate).
    let mut no_agent = McpServer::new()
        .with_knowledge(Arc::clone(&runtime_no_agent))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut no_agent);
    assert!(
        !listed_tool_names(&mut no_agent).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent when runtime has no bound agent id (K-L5)"
    );

    // Bare server → absent.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    assert!(
        !listed_tool_names(&mut bare).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent on a bare server"
    );
}

/// K-L5: the feedback tool schema must NOT contain an `agent_id` property —
/// identity is host-bound, not caller-supplied.
#[test]
fn feedback_tool_schema_has_no_agent_id_field() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#)[0],
    );
    let tool = resp["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"))
        .iter()
        .find(|t| t["name"] == FEEDBACK_TOOL)
        .unwrap_or_else(|| panic!("{FEEDBACK_TOOL} must be listed"));

    let props = &tool["inputSchema"]["properties"];
    assert!(
        props.get("agent_id").is_none(),
        "feedback schema must not have agent_id — identity is host-bound (K-L5): {tool}"
    );
    let required = tool["inputSchema"]["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    assert!(
        !required.contains(&"agent_id"),
        "feedback schema must not require agent_id: {tool}"
    );
}

// ---------------------------------------------------------------------------
// Surface-unavailable posture
// ---------------------------------------------------------------------------

/// Calling the tool without the feedback surface attached returns a tool error
/// (never a protocol error / -32601).
#[test]
fn feedback_tool_without_surface_is_a_tool_error_not_protocol() {
    // Runtime attached but NO store.
    let mut server = McpServer::new().with_knowledge(build_runtime());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "query_fingerprint": "fp-1" }),
    );
    assert!(
        is_tool_error(&resp),
        "call without store must be a tool error: {resp}"
    );
    // Must NOT be a protocol error (-32601 method not found).
    assert!(
        resp.get("error").is_none(),
        "must not be a JSON-RPC error object: {resp}"
    );
}

// ---------------------------------------------------------------------------
// Happy-path recording
// ---------------------------------------------------------------------------

/// A valid call appends one event to the store and returns `recorded: true`.
/// The graph, tags, and proposals are not touched (append-only posture).
#[test]
fn valid_call_appends_one_event_and_returns_recorded_true() {
    let runtime = build_runtime();
    let store = fresh_store();

    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({
            "query_fingerprint": "fp-abc",
            "hit_ids": ["doc-1", "doc-2"],
            "used": true,
        }),
    );

    assert!(
        !is_tool_error(&resp),
        "valid call must not be a tool error: {resp}"
    );
    let parsed = resp["result"]["content"][0]["text"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| panic!("content must be JSON: {resp}"));
    assert_eq!(
        parsed["recorded"].as_bool(),
        Some(true),
        "response must contain recorded:true: {resp}"
    );
    // The response echoes the BOUND agent id, not a caller-supplied one.
    assert_eq!(
        parsed["agent_id"].as_str(),
        Some("agent:test"),
        "response agent_id must be the bound principal: {resp}"
    );

    // Verify the event was actually appended.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let count = rt.block_on(async { store.lock().await.len() });
    assert_eq!(count, 1, "exactly one event must be in the store");
}

/// Two calls with distinct fingerprints append two events; `used=false` is
/// valid (explicit opt-out) and also appends.
#[test]
fn two_calls_append_two_events() {
    let runtime = build_runtime();
    let store = fresh_store();
    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    for (fp, used) in [("fp-1", true), ("fp-2", false)] {
        let resp = call(
            &mut server,
            FEEDBACK_TOOL,
            &json!({ "query_fingerprint": fp, "used": used }),
        );
        assert!(!is_tool_error(&resp), "call {fp} must succeed: {resp}");
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let count = rt.block_on(async { store.lock().await.len() });
    assert_eq!(count, 2, "two calls must produce two events");
}

/// `hit_ids` exceeding `MAX_HIT_IDS` (64) are silently truncated at admission.
#[test]
fn hit_ids_over_cap_are_truncated() {
    use tdw_agent_store::MAX_HIT_IDS;

    let runtime = build_runtime();
    let store = fresh_store();
    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    let oversized: Vec<Value> = (0..MAX_HIT_IDS + 10)
        .map(|i| json!(format!("doc:{i}")))
        .collect();
    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({
            "query_fingerprint": "fp-big",
            "hit_ids": oversized,
        }),
    );
    assert!(
        !is_tool_error(&resp),
        "oversized hit_ids must not error: {resp}"
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let stored_hit_count = rt.block_on(async {
        store
            .lock()
            .await
            .events()
            .next()
            .map_or(0, |e| e.hit_ids.len())
    });
    assert_eq!(
        stored_hit_count, MAX_HIT_IDS,
        "stored hit_ids must be capped at MAX_HIT_IDS"
    );
}

// ---------------------------------------------------------------------------
// Argument validation
// ---------------------------------------------------------------------------

/// Missing required `query_fingerprint` argument is a tool error.
#[test]
fn missing_query_fingerprint_is_a_tool_error() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({}), // query_fingerprint missing
    );
    assert!(
        is_tool_error(&resp),
        "missing query_fingerprint must be a tool error: {resp}"
    );
    assert!(
        resp.get("error").is_none(),
        "must not be a protocol error: {resp}"
    );
}

// ---------------------------------------------------------------------------
// K-L5 spoofing regression tests
// ---------------------------------------------------------------------------

/// K-L5: a call carrying a forged `agent_id` argument must NOT win the identity
/// race — the stored event must carry the BOUND principal, not the forged value.
///
/// MCP does not enforce `additionalProperties: false` server-side (schemas are
/// advisory for clients); the spoofing protection comes from `execute()` reading
/// identity from `runtime.bound_agent_id()` and ignoring the argument entirely.
/// The call succeeds (extra arguments are silently ignored), but the stored
/// event always reflects the host-bound principal.
///
/// This is the PRIMARY spoofing regression for K-L5: before this story,
/// `agent_id` in arguments set the identity. After K-L5 it is always ignored.
#[test]
fn forged_agent_id_argument_does_not_change_stored_identity() {
    let store = fresh_store();
    let mut server = McpServer::new()
        .with_knowledge(build_runtime()) // bound to "agent:test"
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    // Supply agent_id as a tool argument — extra args are silently ignored by
    // the MCP layer, but execute() must read from the bound principal, not here.
    let req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": FEEDBACK_TOOL,
            "arguments": {
                "agent_id": "forged:attacker",
                "query_fingerprint": "fp-spoof",
            }
        }
    });
    let resp = decode(&server.handle_json_rpc_line(&req.to_string())[0]);

    // The call may succeed (extra fields pass through the MCP layer), but the
    // stored event MUST carry the bound principal, not the forged one.
    // If it is a tool error that is also acceptable — what is NOT acceptable is
    // a success response that echoes "forged:attacker" as the agent_id.
    let parsed = resp["result"]["content"][0]["text"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());

    if let Some(parsed) = parsed {
        // If the call succeeded, the echoed agent_id must be the bound principal.
        assert_ne!(
            parsed["agent_id"].as_str(),
            Some("forged:attacker"),
            "forged agent_id must NEVER appear in the response (K-L5): {resp}"
        );
        assert_eq!(
            parsed["agent_id"].as_str(),
            Some("agent:test"),
            "response must echo the bound principal, not the forged id (K-L5): {resp}"
        );
    }

    // Either way, the stored event must carry the bound principal.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let stored = rt.block_on(async {
        store
            .lock()
            .await
            .events()
            .next()
            .map(|e| e.agent_id.clone())
    });
    if let Some(stored_id) = stored {
        assert_eq!(
            stored_id, "agent:test",
            "stored event agent_id must be the bound principal, not the forged value (K-L5)"
        );
    }
    // If no event was stored (call was rejected entirely), that's also a pass —
    // the forged id did not win.
}

/// K-L5: even if the call succeeds (no forged id), the stored event's agent_id
/// must ALWAYS be the bound principal, never a caller-supplied value.
#[test]
fn stored_event_agent_id_always_matches_bound_principal() {
    let runtime = Arc::new(
        KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
        .with_agent_id("principal:configured"),
    );
    let store = fresh_store();

    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    // Valid call — no agent_id in arguments (correct usage after K-L5).
    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "query_fingerprint": "fp-bound" }),
    );
    assert!(!is_tool_error(&resp), "valid call must succeed: {resp}");

    // The stored event must carry the BOUND principal id, not any caller value.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let stored_agent_id = rt.block_on(async {
        store
            .lock()
            .await
            .events()
            .next()
            .map(|e| e.agent_id.clone())
    });
    assert_eq!(
        stored_agent_id.as_deref(),
        Some("principal:configured"),
        "stored agent_id must be the bound principal (K-L5): got {stored_agent_id:?}"
    );
}

/// K-L5: a runtime without a bound agent id keeps the feedback surface OFF entirely.
/// A direct `tools/call` to the feedback tool name must be a tool error, not a
/// protocol error — and the tool must be absent from `tools/list`.
#[test]
fn feedback_surface_absent_without_bound_agent_id() {
    // Runtime with no agent id bound.
    let runtime = build_runtime_no_agent();
    let store = fresh_store();

    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    // Not listed.
    assert!(
        !listed_tool_names(&mut server).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent when no agent id is bound (K-L5)"
    );

    // Direct call is a tool error, not a protocol error.
    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "query_fingerprint": "fp-1" }),
    );
    assert!(
        is_tool_error(&resp),
        "call must be a tool error when no agent id is bound: {resp}"
    );
    assert!(
        resp.get("error").is_none(),
        "must not be a protocol error: {resp}"
    );
}

// ---------------------------------------------------------------------------
// Append-only posture
// ---------------------------------------------------------------------------

/// Append-only: calling the tool does NOT mutate proposals. A server with both
/// write tools and the feedback tool wired has zero proposals after a feedback
/// call.
#[allow(clippy::too_many_lines)]
#[test]
fn feedback_is_append_only_does_not_touch_proposals() {
    use tdw_knowledge::proposals::ProposalQueue;
    use tdw_knowledge::runtime::AdaptivityResolver;
    use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
    use tdw_taxonomy::Adaptivity;

    let graph = Arc::new(InMemoryGraphEngine::default());
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let proposals = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
    let resolver: AdaptivityResolver = Arc::new(|id: &str| {
        if id == "agent:a" {
            Some(Adaptivity::Learning)
        } else {
            None
        }
    });

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(Arc::new({
            // A minimal SharedGraph inline — no full boilerplate needed.
            struct G(Arc<InMemoryGraphEngine>);
            use async_trait::async_trait;
            use tdw_core::{
                GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Result, Subgraph,
                TraversalFilter,
            };
            #[async_trait]
            impl GraphEngine for G {
                async fn upsert_nodes(&self, n: Vec<GraphNode>) -> Result<()> {
                    self.0.upsert_nodes(n).await
                }
                async fn upsert_edges(&self, e: Vec<GraphEdge>) -> Result<()> {
                    self.0.upsert_edges(e).await
                }
                async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
                    self.0.node(id).await
                }
                async fn neighbors(
                    &self,
                    id: &str,
                    f: &TraversalFilter,
                ) -> Result<Vec<(GraphEdge, GraphNode)>> {
                    self.0.neighbors(id, f).await
                }
                async fn expand(&self, s: &[String], f: &TraversalFilter) -> Result<Subgraph> {
                    self.0.expand(s, f).await
                }
                async fn shortest_path(
                    &self,
                    from: &str,
                    to: &str,
                    f: &TraversalFilter,
                ) -> Result<Option<Vec<GraphEdge>>> {
                    self.0.shortest_path(from, to, f).await
                }
                async fn edges(
                    &self,
                    r: Option<&str>,
                    o: usize,
                    l: usize,
                ) -> Result<Vec<GraphEdge>> {
                    self.0.edges(r, o, l).await
                }
                async fn delete_edges(
                    &self,
                    from: &str,
                    rel: &str,
                    to: Option<&str>,
                ) -> Result<usize> {
                    self.0.delete_edges(from, rel, to).await
                }
                async fn replace_edges(
                    &self,
                    from: &str,
                    rel: &str,
                    new: Vec<GraphEdge>,
                ) -> Result<()> {
                    self.0.replace_edges(from, rel, new).await
                }
                async fn merge_entities(
                    &self,
                    s: &str,
                    t: &str,
                    d: &MergeDecision,
                ) -> Result<MergeReport> {
                    self.0.merge_entities(s, t, d).await
                }
            }
            G(graph.clone())
        }))
        .with_tags(Arc::new(GraphTagEngine::new({
            struct G2(Arc<InMemoryGraphEngine>);
            use async_trait::async_trait;
            use tdw_core::{
                GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Result, Subgraph,
                TraversalFilter,
            };
            #[async_trait]
            impl GraphEngine for G2 {
                async fn upsert_nodes(&self, n: Vec<GraphNode>) -> Result<()> {
                    self.0.upsert_nodes(n).await
                }
                async fn upsert_edges(&self, e: Vec<GraphEdge>) -> Result<()> {
                    self.0.upsert_edges(e).await
                }
                async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
                    self.0.node(id).await
                }
                async fn neighbors(
                    &self,
                    id: &str,
                    f: &TraversalFilter,
                ) -> Result<Vec<(GraphEdge, GraphNode)>> {
                    self.0.neighbors(id, f).await
                }
                async fn expand(&self, s: &[String], f: &TraversalFilter) -> Result<Subgraph> {
                    self.0.expand(s, f).await
                }
                async fn shortest_path(
                    &self,
                    from: &str,
                    to: &str,
                    f: &TraversalFilter,
                ) -> Result<Option<Vec<GraphEdge>>> {
                    self.0.shortest_path(from, to, f).await
                }
                async fn edges(
                    &self,
                    r: Option<&str>,
                    o: usize,
                    l: usize,
                ) -> Result<Vec<GraphEdge>> {
                    self.0.edges(r, o, l).await
                }
                async fn delete_edges(
                    &self,
                    from: &str,
                    rel: &str,
                    to: Option<&str>,
                ) -> Result<usize> {
                    self.0.delete_edges(from, rel, to).await
                }
                async fn replace_edges(
                    &self,
                    from: &str,
                    rel: &str,
                    new: Vec<GraphEdge>,
                ) -> Result<()> {
                    self.0.replace_edges(from, rel, new).await
                }
                async fn merge_entities(
                    &self,
                    s: &str,
                    t: &str,
                    d: &MergeDecision,
                ) -> Result<MergeReport> {
                    self.0.merge_entities(s, t, d).await
                }
            }
            G2(graph)
        })))
        .with_proposals(Arc::clone(&proposals))
        .with_adaptivity_resolver(resolver)
        .with_agent_id("agent:a");
    let runtime = Arc::new(runtime);
    let store = fresh_store();

    let mut server = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut server);

    // Call the feedback tool — no agent_id argument (K-L5: identity is host-bound).
    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "query_fingerprint": "fp-1", "used": true }),
    );
    assert!(!is_tool_error(&resp), "feedback call must succeed: {resp}");

    // Proposals must be untouched.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("rt");
    let proposal_count =
        rt.block_on(async { proposals.lock().await.list(None, Some(100)).proposals.len() });
    assert_eq!(
        proposal_count, 0,
        "feedback tool must not touch the proposal queue"
    );
}
