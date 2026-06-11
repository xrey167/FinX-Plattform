//! End-to-end tests for the MCP knowledge FEEDBACK tool (knowledge-system B10).
//!
//! Each test drives `tdw.kg.feedback` through the SAME JSON-RPC `tools/call`
//! surface a real client uses, asserting:
//!
//! - Descriptor gating: the tool appears only when BOTH a [`KnowledgeRuntime`]
//!   and a [`RetrievalFeedbackStore`] are attached; absent either → not listed.
//! - Append-only posture: calling the tool appends a [`RetrievalEvent`] and does
//!   NOT mutate the graph, tags, or proposals.
//! - Payload caps: `hit_ids` > 64 are silently truncated at admission.
//! - Invalid `agent_id` (`;` or control char) → tool error, not protocol error.
//! - Missing required argument → tool error, not protocol error.
//! - Without the feedback surface → tool error, not protocol error.

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

fn build_runtime() -> Arc<KnowledgeRuntime> {
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
// Tests
// ---------------------------------------------------------------------------

/// Descriptor gating: tool is listed when BOTH runtime AND store are attached;
/// absent when either is missing.
#[test]
fn descriptor_gated_on_runtime_and_store() {
    let runtime = build_runtime();
    let store = fresh_store();

    // Both attached → listed.
    let mut full = McpServer::new()
        .with_knowledge(Arc::clone(&runtime))
        .with_feedback_store(Arc::clone(&store));
    initialize(&mut full);
    assert!(
        listed_tool_names(&mut full).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be listed when both runtime and store are attached"
    );

    // Runtime only, no store → absent.
    let mut no_store = McpServer::new().with_knowledge(Arc::clone(&runtime));
    initialize(&mut no_store);
    assert!(
        !listed_tool_names(&mut no_store).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent without the feedback store"
    );

    // Store only, no runtime → absent.
    let mut no_runtime = McpServer::new().with_feedback_store(Arc::clone(&store));
    initialize(&mut no_runtime);
    assert!(
        !listed_tool_names(&mut no_runtime).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent without the knowledge runtime"
    );

    // Bare server → absent.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    assert!(
        !listed_tool_names(&mut bare).contains(&FEEDBACK_TOOL.to_string()),
        "tool must be absent on a bare server"
    );
}

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
        &json!({ "agent_id": "agent:test", "query_fingerprint": "fp-1" }),
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
            "agent_id": "agent:test",
            "query_fingerprint": "fp-abc",
            "hit_ids": ["doc-1", "doc-2"],
            "used": true,
        }),
    );

    assert!(
        !is_tool_error(&resp),
        "valid call must not be a tool error: {resp}"
    );
    assert_eq!(
        resp["result"]["content"][0]["text"]
            .as_str()
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v["recorded"].as_bool()),
        Some(true),
        "response must contain recorded:true: {resp}"
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
            &json!({ "agent_id": "agent:a", "query_fingerprint": fp, "used": used }),
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
            "agent_id": "agent:a",
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

/// An `agent_id` containing `;` (reserved separator in the provenance format)
/// is rejected with a tool error, not a protocol error.
#[test]
fn invalid_agent_id_semicolon_is_a_tool_error() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "agent_id": "bad;agent", "query_fingerprint": "fp-1" }),
    );
    assert!(
        is_tool_error(&resp),
        "agent_id with `;` must be a tool error: {resp}"
    );
    assert!(
        resp.get("error").is_none(),
        "must not be a protocol error: {resp}"
    );
}

/// An `agent_id` containing a control character is rejected.
#[test]
fn invalid_agent_id_control_char_is_a_tool_error() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "agent_id": "bad\x01agent", "query_fingerprint": "fp-1" }),
    );
    assert!(
        is_tool_error(&resp),
        "agent_id with control char must be a tool error: {resp}"
    );
}

/// An `agent_id` containing `:` is VALID (it is part of the allowed grammar
/// `[A-Za-z0-9:._-]`).
#[test]
fn agent_id_with_colon_is_valid() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "agent_id": "agent:market-researcher", "query_fingerprint": "fp-ok" }),
    );
    assert!(
        !is_tool_error(&resp),
        "agent_id with `:` must be valid: {resp}"
    );
}

/// Missing required `query_fingerprint` argument is a tool error.
#[test]
fn missing_required_argument_is_a_tool_error() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "agent_id": "agent:a" }), // query_fingerprint missing
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

/// Missing required `agent_id` argument is a tool error.
#[test]
fn missing_agent_id_is_a_tool_error() {
    let mut server = McpServer::new()
        .with_knowledge(build_runtime())
        .with_feedback_store(fresh_store());
    initialize(&mut server);

    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "query_fingerprint": "fp-1" }), // agent_id missing
    );
    assert!(
        is_tool_error(&resp),
        "missing agent_id must be a tool error: {resp}"
    );
}

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

    // Call the feedback tool.
    let resp = call(
        &mut server,
        FEEDBACK_TOOL,
        &json!({ "agent_id": "agent:a", "query_fingerprint": "fp-1", "used": true }),
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
