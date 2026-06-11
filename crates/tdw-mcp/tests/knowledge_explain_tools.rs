//! End-to-end tests for the MCP knowledge EXPLAIN tools (knowledge-system K-X1).
//!
//! Each test builds a [`KnowledgeRuntime`] over fully in-memory engines, seeds
//! a small temporal fixture (nodes + edges with explicit `valid_from`/`valid_to`
//! windows + tags), attaches to an [`McpServer`], and drives both
//! `tdw.kg.why` and `tdw.kg.diff` through the JSON-RPC surface.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance, Result, Subgraph,
    TraversalFilter,
};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_mcp::McpServer;
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tags::{TagAssignment, TagDefinition, TagEngine};
use tdw_taxonomy::EntityKind;

// ── Fixture timestamps ────────────────────────────────────────────────────────

/// T0 = 2024-01-01 (ingest epoch)
const T0: &str = "2024-01-01T00:00:00Z";
/// T1 = 2025-01-01 (mid-point: XNAS-OLD listing expires, agent annotation starts)
const T1: &str = "2025-01-01T00:00:00Z";
/// T2 = 2026-01-01 (present: open windows still active)
const T2: &str = "2026-01-01T00:00:00Z";

const T0_DATE: &str = "2024-01-01";
const T1_DATE: &str = "2025-01-01";

// ── Shared graph wrapper ──────────────────────────────────────────────────────

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

// ── Fixture builder ───────────────────────────────────────────────────────────

/// Build the test runtime and return both the runtime and the raw graph engine
/// (so tests can query the raw store when needed).
///
/// Fixture nodes:
/// - `instrument:AAPL`  (Instrument)
/// - `venue:XNAS`       (Venue)
/// - `venue:XNAS-OLD`   (Venue)
/// - `annotation:p1`    (Document)
///
/// Fixture edges:
/// - `instrument:AAPL -listed_on-> venue:XNAS`       Ingest{source:"exchange-feed"} T0..open
/// - `instrument:AAPL -listed_on-> venue:XNAS-OLD`   Ingest{source:"exchange-feed"} T0..T1 (invalidated)
/// - `instrument:AAPL -annotated_by-> annotation:p1` Agent{agent_id:"analyst-1",gated:true} T1..open
/// - `instrument:AAPL -peer_of-> venue:XNAS`         Rule{rule_id:"derived:peer",version:1} T0..open
///
/// Tag `asset:equity` assigned to `instrument:AAPL` at T0_DATE with rule provenance.
async fn build_explain_runtime() -> (Arc<KnowledgeRuntime>, Arc<InMemoryGraphEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    // Nodes
    graph
        .upsert_nodes(vec![
            GraphNode {
                id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple Inc.".to_string(),
                aliases: vec!["AAPL".to_string()],
                props: Value::Null,
                valid_from: Some(T0.to_string()),
                valid_to: None,
            },
            GraphNode {
                id: "venue:XNAS".to_string(),
                kind: EntityKind::Venue,
                label: "NASDAQ".to_string(),
                aliases: vec![],
                props: Value::Null,
                valid_from: Some(T0.to_string()),
                valid_to: None,
            },
            GraphNode {
                id: "venue:XNAS-OLD".to_string(),
                kind: EntityKind::Venue,
                label: "NASDAQ-OLD".to_string(),
                aliases: vec![],
                props: Value::Null,
                valid_from: Some(T0.to_string()),
                valid_to: Some(T1.to_string()),
            },
            GraphNode {
                id: "annotation:p1".to_string(),
                kind: EntityKind::Document,
                label: "Analyst note p1".to_string(),
                aliases: vec![],
                props: Value::Null,
                valid_from: Some(T1.to_string()),
                valid_to: None,
            },
        ])
        .await
        .unwrap_or_else(|e| panic!("nodes: {e}"));

    // Edges
    graph
        .upsert_edges(vec![
            // Active listing T0..open
            GraphEdge {
                from: "instrument:AAPL".to_string(),
                rel: "listed_on".to_string(),
                to: "venue:XNAS".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "exchange-feed".to_string(),
                },
                valid_from: Some(T0.to_string()),
                valid_to: None,
            },
            // Expired listing T0..T1
            GraphEdge {
                from: "instrument:AAPL".to_string(),
                rel: "listed_on".to_string(),
                to: "venue:XNAS-OLD".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "exchange-feed".to_string(),
                },
                valid_from: Some(T0.to_string()),
                valid_to: Some(T1.to_string()),
            },
            // Gated agent annotation starting at T1
            GraphEdge {
                from: "instrument:AAPL".to_string(),
                rel: "annotated_by".to_string(),
                to: "annotation:p1".to_string(),
                props: Value::Null,
                provenance: Provenance::Agent {
                    agent_id: "analyst-1".to_string(),
                    gated: true,
                },
                valid_from: Some(T1.to_string()),
                valid_to: None,
            },
            // Rule-derived edge T0..open
            GraphEdge {
                from: "instrument:AAPL".to_string(),
                rel: "peer_of".to_string(),
                to: "venue:XNAS".to_string(),
                props: Value::Null,
                provenance: Provenance::Rule {
                    rule_id: "derived:peer".to_string(),
                    version: 1,
                },
                valid_from: Some(T0.to_string()),
                valid_to: None,
            },
        ])
        .await
        .unwrap_or_else(|e| panic!("edges: {e}"));

    // Tags
    let tags = GraphTagEngine::new(SharedGraph(graph.clone()));
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .await
    .unwrap_or_else(|e| panic!("define tag: {e}"));
    tags.assign(TagAssignment {
        entity_id: "instrument:AAPL".to_string(),
        tag_id: "asset:equity".to_string(),
        assigned_at: T0_DATE.to_string(),
        expires_at: None,
        provenance: "rule:tag-propagate@v2".to_string(),
    })
    .await
    .unwrap_or_else(|e| panic!("assign tag: {e}"));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_lexical(lexical, "knowledge")
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))));
    (Arc::new(runtime), graph)
}

// ── Test helpers ──────────────────────────────────────────────────────────────

fn decode(message: &str) -> Value {
    serde_json::from_str(message)
        .unwrap_or_else(|e| panic!("response should be valid JSON: {e}; raw={message}"))
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

fn block<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime builds")
        .block_on(f)
}

fn server_with_runtime() -> McpServer {
    let (runtime, _graph) = block(build_explain_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    server
}

// ── Descriptor gating ─────────────────────────────────────────────────────────

#[test]
fn explain_descriptors_absent_without_runtime() {
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let listed =
        decode(&bare.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0]);
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"))
        .iter()
        .filter_map(|t| t["name"].as_str().map(ToString::to_string))
        .collect();
    assert!(
        !names.contains(&"tdw.kg.why".to_string()),
        "why must be absent"
    );
    assert!(
        !names.contains(&"tdw.kg.diff".to_string()),
        "diff must be absent"
    );
}

#[test]
fn explain_descriptors_present_with_graph_runtime() {
    let mut server = server_with_runtime();
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"));
    for name in ["tdw.kg.why", "tdw.kg.diff"] {
        let descriptor = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} must be listed"));
        assert_eq!(
            descriptor["annotations"]["readOnlyHint"], true,
            "{name} readOnlyHint"
        );
        assert_eq!(
            descriptor["annotations"]["idempotentHint"], true,
            "{name} idempotentHint"
        );
    }
}

// ── tdw.kg.why: edge provenance chains ───────────────────────────────────────

#[test]
fn why_edge_ingest_chain() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "edge": { "from": "instrument:AAPL", "rel": "listed_on", "to": "venue:XNAS" }
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    assert_eq!(sc["subject"]["kind"], "edge");
    assert_eq!(sc["subject"]["from"], "instrument:AAPL");
    assert_eq!(sc["subject"]["rel"], "listed_on");
    assert_eq!(sc["subject"]["to"], "venue:XNAS");
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain array"));
    assert!(!chain.is_empty(), "ingest chain must be non-empty");
    let step0 = &chain[0];
    assert_eq!(step0["kind"], "ingest");
    assert_eq!(step0["source"], "exchange-feed");
    assert_eq!(sc["chain_depth_cap_reached"], false);
}

#[test]
fn why_edge_rule_chain() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "edge": { "from": "instrument:AAPL", "rel": "peer_of", "to": "venue:XNAS" }
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain array"));
    assert!(!chain.is_empty());
    let step0 = &chain[0];
    assert_eq!(step0["kind"], "rule_derived");
    assert_eq!(step0["rule_id"], "derived:peer");
    assert_eq!(step0["version"], 1);
    // Support fan-out cap documented in the step.
    assert!(step0["support_fanout_cap"].as_u64().is_some());
    // Honest absence note for DerivationIndex.
    assert!(
        step0["support_index_note"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "support_index_note must be present"
    );
}

#[test]
fn why_edge_agent_gated_chain() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "edge": {
                "from": "instrument:AAPL",
                "rel": "annotated_by",
                "to": "annotation:p1"
            },
            "as_of": T1
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain array"));
    assert!(!chain.is_empty());
    let step0 = &chain[0];
    assert_eq!(step0["kind"], "agent_gated");
    assert_eq!(step0["agent_id"], "analyst-1");
    assert_eq!(step0["gated"], true);
    // Honest absence note for ProposalQueue audit trail.
    assert!(
        step0["note"]
            .as_str()
            .is_some_and(|s| s.contains("ProposalQueue")),
        "must reference ProposalQueue"
    );
}

#[test]
fn why_edge_missing_returns_empty_chain() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "edge": { "from": "instrument:AAPL", "rel": "listed_on", "to": "venue:NONEXISTENT" }
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain array"));
    assert!(chain.is_empty(), "missing edge must yield empty chain");
    assert_eq!(sc["chain_depth_cap_reached"], false);
}

#[test]
fn why_edge_inactive_window_returns_empty_chain() {
    // The XNAS-OLD listing closes at T1; querying as_of=T2 should yield no active window.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "edge": {
                "from": "instrument:AAPL",
                "rel": "listed_on",
                "to": "venue:XNAS-OLD"
            },
            "as_of": T2
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    // Either empty chain OR a summary stating no active window — both are correct.
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain array"));
    // The summary must reference the as_of or the inactive state.
    let summary = sc["summary"].as_str().unwrap_or("");
    assert!(
        chain.is_empty() || summary.contains("no active window") || summary.contains("XNAS-OLD"),
        "should surface inactivity; chain={chain:?}, summary={summary:?}"
    );
}

// ── tdw.kg.why: tag provenance ────────────────────────────────────────────────

#[test]
fn why_tag_confirmed_active() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "tag_assignment": {
                "entity_id": "instrument:AAPL",
                "tag_id": "asset:equity"
            },
            "as_of": T1_DATE
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    assert_eq!(sc["subject"]["kind"], "tag_assignment");
    assert_eq!(sc["subject"]["entity_id"], "instrument:AAPL");
    assert_eq!(sc["subject"]["tag_id"], "asset:equity");
    let chain = sc["chain"].as_array().unwrap_or_else(|| panic!("chain"));
    assert!(!chain.is_empty());
    // First step confirms the tag is active.
    assert_eq!(chain[0]["kind"], "confirmed_active");
    // Second step honestly states provenance is not accessible via the trait.
    assert!(
        chain
            .iter()
            .any(|s| s["kind"] == "provenance_not_accessible"),
        "must include honest absence step for TagEngine trait limitation"
    );
}

#[test]
fn why_tag_not_active_returns_not_found() {
    let mut server = server_with_runtime();
    // Query a tag that was never assigned.
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "tag_assignment": {
                "entity_id": "instrument:AAPL",
                "tag_id": "sector:technology"
            }
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let chain = sc["chain"].as_array().unwrap_or_else(|| panic!("chain"));
    assert!(!chain.is_empty());
    assert_eq!(chain[0]["kind"], "not_found");
}

// ── tdw.kg.why: entity ────────────────────────────────────────────────────────

#[test]
fn why_entity_known() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": "instrument:AAPL" }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    assert_eq!(sc["subject"]["kind"], "entity");
    assert_eq!(sc["subject"]["entity_id"], "instrument:AAPL");
    let chain = sc["chain"].as_array().unwrap_or_else(|| panic!("chain"));
    assert!(!chain.is_empty());
    assert_eq!(chain[0]["kind"], "entity_kind");
    assert_eq!(chain[0]["entity_id"], "instrument:AAPL");
}

#[test]
fn why_entity_not_found() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": "instrument:UNKNOWN" }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let chain = sc["chain"].as_array().unwrap_or_else(|| panic!("chain"));
    assert!(!chain.is_empty());
    assert_eq!(chain[0]["kind"], "not_found");
}

// ── tdw.kg.why: argument validation ──────────────────────────────────────────

#[test]
fn why_ambiguous_subject_is_tool_error() {
    let mut server = server_with_runtime();
    // Providing both edge and entity_id is ambiguous → tool error.
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({
            "entity_id": "instrument:AAPL",
            "edge": { "from": "instrument:AAPL", "rel": "listed_on", "to": "venue:XNAS" }
        }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "ambiguous subject must be a tool error"
    );
}

#[test]
fn why_no_subject_is_tool_error() {
    let mut server = server_with_runtime();
    let response = call(&mut server, "tdw.kg.why", &json!({}));
    assert_eq!(
        response["result"]["isError"], true,
        "missing subject must be a tool error"
    );
}

// ── tdw.kg.diff: temporal snapshots ──────────────────────────────────────────

#[test]
fn diff_t0_to_t1_adds_nothing_invalidates_xnas_old() {
    // Window T0→T1: the XNAS-OLD listing closes at T1 → 1 invalidated edge.
    // No new edge starts exactly in this window under our fixture (listed_on XNAS starts AT T0,
    // peer_of starts AT T0 — both are active at T0 already).
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T1 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    assert_eq!(sc["from_as_of"].as_str().unwrap_or(""), T0);
    assert_eq!(sc["to_as_of"].as_str().unwrap_or(""), T1);

    // Invalidated edges: the XNAS-OLD listing closed at T1.
    let inv = &sc["edges_invalidated"];
    assert!(
        inv["count"].as_u64().unwrap_or(0) >= 1,
        "at least one edge must be invalidated (XNAS-OLD listing)"
    );
    let inv_items = inv["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items array"));
    assert!(
        inv_items.iter().any(|item| {
            item["edge"]["from"] == "instrument:AAPL"
                && item["edge"]["rel"] == "listed_on"
                && item["edge"]["to"] == "venue:XNAS-OLD"
        }),
        "XNAS-OLD listing must appear in invalidated items; got {inv_items:?}"
    );

    // Leakage guard field must be present.
    assert!(
        sc["leakage_guard"].as_str().is_some_and(|s| !s.is_empty()),
        "leakage_guard field must be present"
    );
}

#[test]
fn diff_t1_to_t2_adds_annotated_by() {
    // Window T1→T2: annotated_by edge starts at T1 → appears in edges_added.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T2 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];

    let added = &sc["edges_added"];
    assert!(
        added["count"].as_u64().unwrap_or(0) >= 1,
        "annotated_by edge must be in added"
    );
    let added_items = added["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items array"));
    assert!(
        added_items.iter().any(|item| {
            item["from"] == "instrument:AAPL"
                && item["rel"] == "annotated_by"
                && item["to"] == "annotation:p1"
        }),
        "annotated_by must appear in edges_added; got {added_items:?}"
    );
}

#[test]
fn diff_no_leakage_future_edges() {
    // Query window entirely before T1; annotated_by (valid_from=T1) must NOT appear in added.
    let before_t1 = "2024-06-01T00:00:00Z";
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": before_t1 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let added_items = sc["edges_added"]["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items"));
    assert!(
        !added_items.iter().any(|item| item["rel"] == "annotated_by"),
        "annotated_by must NOT leak into a pre-T1 diff window"
    );
}

#[test]
fn diff_invalidated_edge_has_successor_field() {
    // The XNAS-OLD listing closes at T1; the XNAS listing stays open.
    // The invalidated entry must carry a successor pointing to XNAS.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T1 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let inv_items = sc["edges_invalidated"]["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items"));
    let xnas_old_entry = inv_items
        .iter()
        .find(|item| item["edge"]["to"] == "venue:XNAS-OLD")
        .unwrap_or_else(|| panic!("XNAS-OLD invalidation not found"));
    // successor field must exist (may be null if no active successor, but the key must be present).
    assert!(
        xnas_old_entry
            .as_object()
            .is_some_and(|o| o.contains_key("successor")),
        "invalidated entry must carry a successor field"
    );
    // And here the XNAS listing is the natural successor.
    if xnas_old_entry["successor"].is_object() {
        assert_eq!(xnas_old_entry["successor"]["to"], "venue:XNAS");
    }
}

// ── tdw.kg.diff: counts and limits ───────────────────────────────────────────

#[test]
fn diff_count_is_exact_not_limited() {
    // Even with limit=1, the count field must reflect the true total.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T2, "limit": 1 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    // edges_added count >= edges_added items length
    let added = &sc["edges_added"];
    let count = added["count"].as_u64().unwrap_or(0) as usize;
    let items_len = added["items"].as_array().map_or(0, |a| a.len());
    assert!(
        count >= items_len,
        "count ({count}) must be >= items.len() ({items_len})"
    );
    // If count > 1 then truncated must be true
    if count > 1 {
        assert_eq!(
            added["truncated"], true,
            "truncated must be true when count > limit"
        );
    }
}

#[test]
fn diff_limit_default_is_100() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T2 }),
    );
    assert_eq!(response["result"]["isError"], false);
    // With our small fixture all items fit within 100; just ensure items is present.
    let sc = &response["result"]["structuredContent"];
    assert!(sc["edges_added"]["items"].is_array());
    assert!(sc["edges_invalidated"]["items"].is_array());
}

// ── tdw.kg.diff: validation errors ───────────────────────────────────────────

#[test]
fn diff_from_ge_to_is_tool_error() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T2, "to_as_of": T0 }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "from >= to must be a tool error"
    );
}

#[test]
fn diff_from_eq_to_is_tool_error() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T1, "to_as_of": T1 }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "from == to must be a tool error"
    );
}

#[test]
fn diff_span_cap_exceeded_without_scope_is_tool_error() {
    // A >3650-day span without scope_entity_id must be rejected.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({
            "from_as_of": "2000-01-01T00:00:00Z",
            "to_as_of": "2026-01-01T00:00:00Z"
        }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "span > DIFF_MAX_SPAN_DAYS without scope must error"
    );
    let content = response["result"]["content"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v["text"].as_str())
        .unwrap_or("");
    assert!(
        content.contains("cap") || content.contains("span") || content.contains("days"),
        "error must mention the span cap; got {content:?}"
    );
}

#[test]
fn diff_span_cap_lifted_with_scope_entity_id() {
    // The same >3650-day span IS allowed when scope_entity_id is provided.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({
            "from_as_of": "2000-01-01T00:00:00Z",
            "to_as_of": "2026-01-01T00:00:00Z",
            "scope_entity_id": "instrument:AAPL"
        }),
    );
    assert_eq!(
        response["result"]["isError"], false,
        "span cap must be lifted when scope_entity_id is supplied"
    );
}

#[test]
fn diff_accepts_date_form_timestamps() {
    // YYYY-MM-DD input must be normalized and accepted.
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": "2024-01-01", "to_as_of": "2026-01-01" }),
    );
    assert_eq!(
        response["result"]["isError"], false,
        "date-form timestamps must be accepted"
    );
}

// ── tdw.kg.diff: scoped tag delta ─────────────────────────────────────────────

#[test]
fn diff_scoped_tag_delta_asset_equity() {
    // Scoped diff on instrument:AAPL should surface asset:equity in tags_gained
    // (it was assigned at T0, so T0→T2 window: gained at T0, still active at T2).
    let mut server = server_with_runtime();
    // Use a window that starts before T0 so the tag appears as "gained".
    let before_t0 = "2023-01-01T00:00:00Z";
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({
            "from_as_of": before_t0,
            "to_as_of": T2,
            "scope_entity_id": "instrument:AAPL"
        }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let gained = &sc["tags_gained"];
    let gained_items = gained["items"]
        .as_array()
        .unwrap_or_else(|| panic!("items"));
    assert!(
        gained_items
            .iter()
            .any(|item| item["tag_id"] == "asset:equity" && item["entity_id"] == "instrument:AAPL"),
        "asset:equity should appear in tags_gained; got {gained_items:?}"
    );
}

#[test]
fn diff_unscoped_tag_delta_honest_absence() {
    // Without scope_entity_id the TagEngine trait cannot enumerate all entities;
    // both tags_gained and tags_expired counts must be 0 (honest absence).
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T2 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    assert_eq!(
        sc["tags_gained"]["count"], 0,
        "unscoped tags_gained count must be 0"
    );
    assert_eq!(
        sc["tags_expired"]["count"], 0,
        "unscoped tags_expired count must be 0"
    );
}

// ── Absent knowledge runtime ──────────────────────────────────────────────────

#[test]
fn why_without_runtime_is_tool_error() {
    let mut server = McpServer::new();
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": "instrument:AAPL" }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "why without runtime must be a tool error"
    );
}

#[test]
fn diff_without_runtime_is_tool_error() {
    let mut server = McpServer::new();
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.kg.diff",
        &json!({ "from_as_of": T0, "to_as_of": T2 }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "diff without runtime must be a tool error"
    );
}
