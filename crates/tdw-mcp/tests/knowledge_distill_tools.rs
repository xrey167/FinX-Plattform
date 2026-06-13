//! End-to-end tests for the MCP session-distillation tool (knowledge-system K-X9).
//!
//! Each test builds a [`KnowledgeRuntime`] over fully in-memory engines with the
//! B9 gate attached (proposal queue + adaptivity resolver + bound agent id) and
//! drives `tdw.kg.distill` through the SAME JSON-RPC `tools/call` surface a real
//! client uses. The contract under test:
//!
//! * a transcript mentioning a known graph entity enqueues a `Validated`
//!   PROPOSAL (never a direct write) — pending-eval, invisible to reads;
//! * `tdw.kg.status` surfaces the `distilled_pending` count additively;
//! * an empty session returns an honest empty result and enqueues nothing;
//! * the tool is ABSENT from `tools/list` when the gate is not attached.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Result, Subgraph,
    TraversalFilter,
};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::proposals::ProposalQueue;
use tdw_knowledge::runtime::{AdaptivityResolver, KnowledgeRuntime};
use tdw_mcp::McpServer;
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::{Adaptivity, EntityKind};

/// A shareable [`GraphEngine`] handle: one in-memory graph behind an `Arc`.
struct SharedGraph(Arc<InMemoryGraphEngine>);

#[async_trait]
impl GraphEngine for SharedGraph {
    async fn upsert_nodes(&self, nodes: Vec<GraphNode>) -> Result<()> {
        self.0.upsert_nodes(nodes).await
    }
    async fn upsert_edges(&self, edges: Vec<GraphEdge>) -> Result<()> {
        self.0.upsert_edges(edges).await
    }
    async fn node(&self, id: &str) -> Result<Option<GraphNode>> {
        self.0.node(id).await
    }
    async fn neighbors(
        &self,
        id: &str,
        filter: &TraversalFilter,
    ) -> Result<Vec<(GraphEdge, GraphNode)>> {
        self.0.neighbors(id, filter).await
    }
    async fn expand(&self, seeds: &[String], filter: &TraversalFilter) -> Result<Subgraph> {
        self.0.expand(seeds, filter).await
    }
    async fn shortest_path(
        &self,
        from: &str,
        to: &str,
        filter: &TraversalFilter,
    ) -> Result<Option<Vec<GraphEdge>>> {
        self.0.shortest_path(from, to, filter).await
    }
    async fn edges(
        &self,
        rel: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<GraphEdge>> {
        self.0.edges(rel, offset, limit).await
    }
    async fn delete_edges(&self, from: &str, rel: &str, to: Option<&str>) -> Result<usize> {
        self.0.delete_edges(from, rel, to).await
    }
    async fn replace_edges(&self, from: &str, rel: &str, new_edges: Vec<GraphEdge>) -> Result<()> {
        self.0.replace_edges(from, rel, new_edges).await
    }
    async fn merge_entities(
        &self,
        source: &str,
        target: &str,
        decision: &MergeDecision,
    ) -> Result<MergeReport> {
        self.0.merge_entities(source, target, decision).await
    }
}

fn node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: Vec::new(),
        props: Value::Null,
        valid_from: None,
        valid_to: None,
    }
}

fn venue(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: EntityKind::Venue,
        label: id.to_string(),
        aliases: Vec::new(),
        props: Value::Null,
        valid_from: None,
        valid_to: None,
    }
}

/// `agent:learning` is `Learning` (admitted); distill:<session> derived ids are
/// also admitted at `Learning`; everything else is unknown (`None`).
fn stub_resolver() -> AdaptivityResolver {
    Arc::new(|agent_id: &str| {
        if agent_id == "agent:learning" || agent_id.starts_with("distill:") {
            Some(Adaptivity::Learning)
        } else {
            None
        }
    })
}

/// Build a distill-enabled runtime: in-memory engines with one seeded entity +
/// an edge so the anchor scan finds it, the full B9 gate, and a bound agent id.
async fn build_distill_runtime() -> Arc<KnowledgeRuntime> {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());
    graph
        .upsert_nodes(vec![node("instrument:AAPL"), venue("venue:nasdaq")])
        .await
        .unwrap_or_else(|error| panic!("seed nodes: {error}"));
    graph
        .upsert_edges(vec![GraphEdge {
            from: "instrument:AAPL".to_string(),
            to: "venue:nasdaq".to_string(),
            rel: "listed_on".to_string(),
            props: Value::Null,
            provenance: tdw_core::Provenance::System {
                detail: "fixture".to_string(),
            },
            valid_from: None,
            valid_to: None,
        }])
        .await
        .unwrap_or_else(|error| panic!("seed edge: {error}"));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))))
        .with_proposals(Arc::new(tokio::sync::Mutex::new(ProposalQueue::default())))
        .with_adaptivity_resolver(stub_resolver())
        .with_agent_id("agent:learning")
        .with_distillation_freshness(Arc::new(std::sync::Mutex::new(
            tdw_knowledge::DistillationFreshness::Pending,
        )));
    Arc::new(runtime)
}

/// A READ-ONLY runtime: graph + tags but NO proposal queue / resolver.
fn build_readonly_runtime() -> Arc<KnowledgeRuntime> {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());
    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph))));
    Arc::new(runtime)
}

fn decode(message: &str) -> Value {
    serde_json::from_str(message)
        .unwrap_or_else(|error| panic!("response should be json: {error}; {message}"))
}

fn initialize(server: &mut McpServer) {
    let messages = server.handle_json_rpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
    );
    assert_eq!(messages.len(), 1);
}

fn call(server: &mut McpServer, name: &str, arguments: &Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    let messages = server.handle_json_rpc_line(&request.to_string());
    decode(&messages[0])
}

/// Parse the `structuredContent` out of a `tools/call` success result.
fn structured(response: &Value) -> Value {
    response["result"]["structuredContent"].clone()
}

fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

fn distill_server() -> McpServer {
    let runtime = block(build_distill_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    server
}

fn listed_tool_names(server: &mut McpServer) -> Vec<String> {
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"))
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(ToString::to_string))
        .collect()
}

#[test]
fn distill_tool_gated_on_full_b9_gate() {
    // A distill-enabled runtime lists the tool.
    let mut server = distill_server();
    let names = listed_tool_names(&mut server);
    assert!(
        names.contains(&"tdw.kg.distill".to_string()),
        "tdw.kg.distill must be listed when the gate is attached"
    );

    // A read-only runtime (no proposals / resolver) does NOT list it.
    let readonly = build_readonly_runtime();
    let mut server = McpServer::new().with_knowledge(readonly);
    initialize(&mut server);
    let names = listed_tool_names(&mut server);
    assert!(
        !names.contains(&"tdw.kg.distill".to_string()),
        "tdw.kg.distill must be absent without the B9 gate"
    );
}

#[test]
fn distill_enqueues_proposal_and_status_surfaces_pending_count() {
    let mut server = distill_server();

    // Status before: no distilled findings pending yet.
    let before = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(
        structured(&before)["distilled_pending"],
        json!(0),
        "no distilled findings pending before the call: {before}"
    );

    // Distill a session that mentions a known entity. as_of is the session time
    // (injected-now) — the only timestamp stamped onto the proposal.
    let result = call(
        &mut server,
        "tdw.kg.distill",
        &json!({
            "session_id": "sess-42",
            "as_of": "2026-06-13",
            "turns": [
                {"episode_id": "ep-0", "role": "user", "text": "How did AAPL do?"},
                {"episode_id": "ep-1", "role": "assistant", "text": "AAPL beat estimates."}
            ]
        }),
    );
    let s = structured(&result);
    assert_eq!(
        result["result"]["isError"],
        json!(false),
        "no tool error: {result}"
    );
    assert_eq!(s["empty"], json!(false), "non-empty: {s}");
    assert_eq!(s["enqueued"], json!(1), "one finding enqueued: {s}");
    assert_eq!(
        s["pending_eval"],
        json!(true),
        "honest pending-eval flag: {s}"
    );
    let finding = &s["findings"][0];
    assert_eq!(finding["anchor_entity_id"], json!("instrument:AAPL"));
    assert_eq!(finding["status"], json!("proposed"));
    assert!(
        finding["proposal_id"].as_str().is_some(),
        "proposal id assigned: {finding}"
    );
    assert_eq!(
        finding["evidence_episode_ids"],
        json!(["ep-0", "ep-1"]),
        "both mentioning turns cited in order: {finding}"
    );

    // Status after: the distilled finding is pending (Validated), surfaced
    // additively as distilled_pending AND inside the proposals.validated count.
    let after = call(&mut server, "tdw.kg.status", &json!({}));
    let sa = structured(&after);
    assert_eq!(
        sa["distilled_pending"],
        json!(1),
        "distilled_pending surfaced: {sa}"
    );
    assert_eq!(
        sa["proposals"]["validated"],
        json!(1),
        "the distilled finding is a pending Validated proposal: {sa}"
    );
    // distillation_freshness reports the real last-run Ok state.
    assert_eq!(sa["distillation_freshness"]["state"], json!("ok"), "{sa}");
    assert_eq!(sa["distillation_freshness"]["enqueued"], json!(1), "{sa}");
}

#[test]
fn empty_session_is_honest_empty_no_proposal() {
    let mut server = distill_server();
    let result = call(
        &mut server,
        "tdw.kg.distill",
        &json!({
            "session_id": "sess-empty",
            "as_of": "2026-06-13",
            "turns": []
        }),
    );
    let s = structured(&result);
    assert_eq!(s["empty"], json!(true), "empty session → honest empty: {s}");
    assert_eq!(s["enqueued"], json!(0), "nothing enqueued: {s}");

    let after = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(structured(&after)["distilled_pending"], json!(0));
}

#[test]
fn distill_tool_error_when_gate_absent() {
    // A read-only runtime exposes no distill tool; calling it is a tool error,
    // never a silent no-op or a protocol error.
    let readonly = build_readonly_runtime();
    let mut server = McpServer::new().with_knowledge(readonly);
    initialize(&mut server);
    let result = call(
        &mut server,
        "tdw.kg.distill",
        &json!({"session_id": "s", "as_of": "2026-06-13", "turns": []}),
    );
    assert_eq!(
        result["result"]["isError"],
        json!(true),
        "missing gate is a loud tool error: {result}"
    );
}
