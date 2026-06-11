//! End-to-end tests for the MCP knowledge WRITE tools (knowledge-system B9).
//!
//! Each test builds a [`KnowledgeRuntime`] over fully in-memory engines, attaches
//! a [`ProposalQueue`] and a stub adaptivity resolver, and drives the gate end to
//! end through the SAME JSON-RPC `tools/call` surface a real client uses. The
//! contract under test: a `Learning` agent's `tdw.tags.define` enqueues a
//! `Validated` proposal that `tdw.kg.proposals list` shows, `approve` flips to
//! `Ready`, and `materialize` lands the tag in the engines; a below-`Learning`
//! agent's submit is a tool error; an unknown agent is a tool error; the write
//! tools are ABSENT from `tools/list` when the proposal queue / resolver are not
//! attached; and the `reject` path is terminal.

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

/// A shareable [`GraphEngine`] handle: one in-memory graph behind an `Arc`,
/// delegating the whole trait (mirrors the B8 harness).
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

/// A stub resolver: `agent:learning` is `Learning`, `agent:configured` is
/// `Configured` (below the gate), every other id is unknown (`None`).
fn stub_resolver() -> AdaptivityResolver {
    Arc::new(|agent_id: &str| match agent_id {
        "agent:learning" => Some(Adaptivity::Learning),
        "agent:configured" => Some(Adaptivity::Configured),
        _ => None,
    })
}

/// Build an OPERATOR runtime: in-memory engines with one seeded entity node,
/// a proposal queue, the stub resolver, and operator authority (so the
/// approve/materialize lifecycle tests can run the human path).
async fn build_write_runtime() -> Arc<KnowledgeRuntime> {
    build_runtime_with_operator(true).await
}

/// An AGENT-FACING runtime (operator authority OFF) — what a real
/// agent-reachable daemon would attach.
async fn build_agent_runtime() -> Arc<KnowledgeRuntime> {
    build_runtime_with_operator(false).await
}

async fn build_runtime_with_operator(operator: bool) -> Arc<KnowledgeRuntime> {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());
    graph
        .upsert_nodes(vec![node("instrument:AAPL")])
        .await
        .unwrap_or_else(|error| panic!("seed node: {error}"));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))))
        .with_proposals(Arc::new(tokio::sync::Mutex::new(ProposalQueue::default())))
        .with_adaptivity_resolver(stub_resolver())
        .with_agent_id("agent:learning")
        .with_operator_authority(operator);
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

/// Drive an async setup future to completion on a runtime fully torn down before
/// returning — so the subsequent SYNC server calls run with NO ambient tokio
/// runtime, exactly as the real MCP serve loop does.
fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

fn write_server() -> McpServer {
    let runtime = block(build_write_runtime());
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

const WRITE_TOOLS: [&str; 4] = [
    "tdw.tags.define",
    "tdw.tags.assign",
    "tdw.kg.annotate",
    "tdw.kg.proposals",
];

#[test]
fn write_tools_gated_on_proposals_and_resolver() {
    // A write-enabled runtime lists all four write tools, each mutating.
    let mut server = write_server();
    let names = listed_tool_names(&mut server);
    for name in WRITE_TOOLS {
        assert!(names.contains(&name.to_string()), "{name} must be listed");
    }

    // A read-only runtime (no proposals / resolver) lists NONE of them.
    let readonly = build_readonly_runtime();
    let mut server = McpServer::new().with_knowledge(readonly);
    initialize(&mut server);
    let names = listed_tool_names(&mut server);
    for name in WRITE_TOOLS {
        assert!(!names.contains(&name.to_string()), "{name} must be absent");
    }

    // A bare server (no knowledge runtime at all) lists none either.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let names = listed_tool_names(&mut bare);
    for name in WRITE_TOOLS {
        assert!(
            !names.contains(&name.to_string()),
            "{name} must be absent without a runtime"
        );
    }
}

#[test]
fn full_lifecycle_define_approve_materialize() {
    let mut server = write_server();

    // Submit via tdw.tags.define (agent at Learning) → a Validated proposal.
    let submitted = call(
        &mut server,
        "tdw.tags.define",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert_eq!(submitted["result"]["isError"], false);
    let structured = &submitted["result"]["structuredContent"];
    let proposal_id = structured["proposal_id"]
        .as_str()
        .unwrap_or_else(|| panic!("proposal_id present: {submitted}"))
        .to_string();
    assert_eq!(structured["status"], "validated");

    // tdw.kg.proposals list shows it.
    let listed = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "list" }),
    );
    assert_eq!(listed["result"]["isError"], false);
    let proposals = listed["result"]["structuredContent"]["proposals"]
        .as_array()
        .unwrap_or_else(|| panic!("proposals array"));
    assert!(
        proposals.iter().any(|p| p["id"] == proposal_id.as_str()),
        "the submitted proposal is listed"
    );

    // approve → Ready.
    let approved = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "approve", "proposal_id": proposal_id, "approved_by": "operator" }),
    );
    assert_eq!(approved["result"]["isError"], false);

    // materialize → the engines now contain the tag definition.
    let materialized = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "materialize" }),
    );
    assert_eq!(materialized["result"]["isError"], false);
    let report = &materialized["result"]["structuredContent"]["report"];
    assert!(
        report["materialized"]
            .as_array()
            .unwrap_or_else(|| panic!("materialized array"))
            .iter()
            .any(|id| id == proposal_id.as_str()),
        "the approved proposal materialized"
    );

    // Prove the tag landed in the ENGINES: a follow-up tdw.tags.assign of the
    // now-materialized tag passes its validator (which rejects unknown tags), so
    // a successful Validated proposal proves the definition is present.
    let assign = call(
        &mut server,
        "tdw.tags.assign",
        &json!({ "entity_id": "instrument:AAPL", "tag_id": "asset:equity" }),
    );
    assert_eq!(
        assign["result"]["isError"], false,
        "assigning the materialized tag must pass validation: {assign}"
    );
    assert_eq!(
        assign["result"]["structuredContent"]["status"], "validated",
        "the assignment proposal is Validated, proving the tag is defined in the engines"
    );
}

#[test]
fn below_learning_submit_is_a_tool_error() {
    // Build a runtime bound to "agent:configured" (below Learning gate).
    let runtime = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());
        graph
            .upsert_nodes(vec![node("instrument:AAPL")])
            .await
            .unwrap_or_else(|error| panic!("seed node: {error}"));
        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())))
            .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))))
            .with_proposals(Arc::new(tokio::sync::Mutex::new(ProposalQueue::default())))
            .with_adaptivity_resolver(stub_resolver())
            .with_agent_id("agent:configured")
            .with_operator_authority(false);
        Arc::new(runtime)
    });
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.tags.define",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert!(response.get("error").is_none(), "must be a tool error");
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("below Learning"),
        "names the admission gate: {response}"
    );
}

#[test]
fn unknown_agent_is_a_tool_error() {
    // Build a runtime bound to "agent:ghost" — not in the stub resolver → unknown.
    let runtime = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());
        graph
            .upsert_nodes(vec![node("instrument:AAPL")])
            .await
            .unwrap_or_else(|error| panic!("seed node: {error}"));
        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())))
            .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))))
            .with_proposals(Arc::new(tokio::sync::Mutex::new(ProposalQueue::default())))
            .with_adaptivity_resolver(stub_resolver())
            .with_agent_id("agent:ghost")
            .with_operator_authority(false);
        Arc::new(runtime)
    });
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.tags.define",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert!(response.get("error").is_none(), "must be a tool error");
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown agent"),
        "names the unknown agent: {response}"
    );
}

#[test]
fn write_tool_without_surface_is_a_tool_error_not_protocol() {
    // A read-only runtime: a write name is a tool error, never a protocol error.
    let readonly = build_readonly_runtime();
    let mut server = McpServer::new().with_knowledge(readonly);
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.tags.define",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert!(
        response.get("error").is_none(),
        "read-only write call must NOT be a protocol error"
    );
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("write surface not attached"),
        "reports the missing write surface: {response}"
    );
}

#[test]
fn reject_path_is_terminal() {
    let mut server = write_server();

    // Submit, then reject.
    let submitted = call(
        &mut server,
        "tdw.kg.annotate",
        &json!({ "entity_id": "instrument:AAPL", "note": "a note" }),
    );
    let proposal_id = submitted["result"]["structuredContent"]["proposal_id"]
        .as_str()
        .unwrap_or_else(|| panic!("proposal_id: {submitted}"))
        .to_string();

    let rejected = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "reject", "proposal_id": proposal_id, "reason": "not useful" }),
    );
    assert_eq!(rejected["result"]["isError"], false);

    // Materialize writes nothing (the rejected proposal is terminal).
    let materialized = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "materialize" }),
    );
    assert_eq!(materialized["result"]["isError"], false);
    let report = &materialized["result"]["structuredContent"]["report"];
    assert!(
        report["materialized"]
            .as_array()
            .unwrap_or_else(|| panic!("materialized array"))
            .is_empty(),
        "a rejected proposal never materializes"
    );

    // Approving the rejected proposal now is a tool error (terminal).
    let approve = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "approve", "proposal_id": proposal_id }),
    );
    assert_eq!(approve["result"]["isError"], true);
}

/// B9 security: the submit tool schemas must NOT expose an `agent_id` field —
/// identity is host-bound, not caller-supplied.
#[test]
fn submit_tools_do_not_accept_agent_id_argument() {
    let mut server = write_server();
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"));

    let submit_names = ["tdw.tags.define", "tdw.tags.assign", "tdw.kg.annotate"];
    for name in submit_names {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} must be listed"));
        let props = &tool["inputSchema"]["properties"];
        assert!(
            props.get("agent_id").is_none(),
            "{name} schema must not have agent_id field — identity is host-bound: {tool}"
        );
        let required = tool["inputSchema"]["required"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(
            !required.contains(&"agent_id"),
            "{name} must not require agent_id: {tool}"
        );
    }
}

/// B9 security: the write tools are ABSENT when the runtime has proposals+resolver
/// but NO bound agent id — the gate requires all three conditions.
#[test]
fn write_surface_absent_without_bound_identity() {
    let runtime = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());
        // proposals + resolver attached, but NO with_agent_id call
        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())))
            .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph))))
            .with_proposals(Arc::new(tokio::sync::Mutex::new(ProposalQueue::default())))
            .with_adaptivity_resolver(stub_resolver());
        Arc::new(runtime)
    });
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let names = listed_tool_names(&mut server);
    for name in WRITE_TOOLS {
        assert!(
            !names.contains(&name.to_string()),
            "{name} must be absent without a bound agent id"
        );
    }
}

/// B9 pagination: list with no limit returns at most `LIST_PAGE_DEFAULT` proposals;
/// limit=1 returns exactly 1 with correct total.
#[test]
fn list_returns_bounded_page_with_total() {
    let mut server = write_server();

    // Submit 3 proposals.
    for i in 0..3_u8 {
        let r = call(
            &mut server,
            "tdw.tags.define",
            &json!({ "tag_id": format!("asset:tag{i}") }),
        );
        assert_eq!(r["result"]["isError"], false, "submit {i}: {r}");
    }

    // List with no limit — all 3 returned, total == 3.
    let listed = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "list" }),
    );
    assert_eq!(listed["result"]["isError"], false);
    let sc = &listed["result"]["structuredContent"];
    let proposals = sc["proposals"]
        .as_array()
        .unwrap_or_else(|| panic!("proposals array: {listed}"));
    assert_eq!(
        proposals.len(),
        3,
        "3 proposals returned by default: {listed}"
    );
    assert_eq!(sc["total"], 3, "total == 3: {listed}");

    // List with limit=1 — 1 returned, total still 3.
    let limited = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "list", "limit": 1 }),
    );
    assert_eq!(limited["result"]["isError"], false);
    let sc2 = &limited["result"]["structuredContent"];
    let page = sc2["proposals"]
        .as_array()
        .unwrap_or_else(|| panic!("proposals array: {limited}"));
    assert_eq!(page.len(), 1, "limit=1 returns 1 proposal: {limited}");
    assert_eq!(sc2["total"], 3, "total still 3 with limit=1: {limited}");
}

/// B9 security review B1: an AGENT-FACING runtime (no operator authority)
/// rejects approve / reject / materialize, so an agent cannot land its own
/// proposals — submit and list still work.
#[test]
fn operator_actions_require_operator_authority() {
    let runtime = block(build_agent_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);

    // Submit is allowed for a Learning agent.
    let submit = call(
        &mut server,
        "tdw.tags.define",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert_ne!(submit["result"]["isError"], true, "submit: {submit}");

    // list is allowed.
    let list = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "list" }),
    );
    assert_ne!(list["result"]["isError"], true, "list: {list}");

    // approve / reject / materialize are all refused.
    for action in ["approve", "reject", "materialize"] {
        let response = call(
            &mut server,
            "tdw.kg.proposals",
            &json!({ "action": action, "proposal_id": "p1" }),
        );
        assert_eq!(response["result"]["isError"], true, "{action}: {response}");
        assert!(
            response.to_string().contains("operator authority"),
            "{action} must name the missing authority: {response}"
        );
    }
}
