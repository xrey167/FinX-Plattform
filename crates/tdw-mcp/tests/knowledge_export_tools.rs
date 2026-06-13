//! End-to-end tests for the MCP research-trail export tool (K-X10).
//!
//! Drives `tdw.kg.export` through the same JSON-RPC `tools/call` surface a real
//! client uses, over fully in-memory engines. Coverage:
//!
//! * (a) a seeded trail (thesis + supporting/contradicting findings, each with
//!   an evidence pin + `described_by` document) exports to JSON + Markdown with
//!   all evidence present and citations resolving to real document ids;
//! * (b) a finding whose `described_by` document node is absent is recorded
//!   explicitly as `[evidence unavailable: id]`, never dropped;
//! * (c) two identical exports are byte-identical (determinism).

#![deny(clippy::pedantic, clippy::nursery)]

use std::sync::Arc;

use serde_json::{Value, json};
use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_mcp::McpServer;
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::EntityKind;

const NOW: &str = "2026-06-13";
const AS_OF_TS: &str = "2026-06-13T00:00:00Z";
const USER_ID: &str = "user:analyst-1";

// ── Test helpers ─────────────────────────────────────────────────────────────

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

fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

fn finding_node(id: &str, label: &str, with_evidence: Option<(&str, &str)>) -> GraphNode {
    let mut props = json!({
        "title": label,
        "user_id": USER_ID,
        "as_of": AS_OF_TS,
        "body": "Detail body.",
    });
    if let Some((doc_id, snippet)) = with_evidence {
        props["evidence"] = json!({
            "document_id": doc_id,
            "snippet": snippet,
            "source_url": "https://example.com/src",
        });
    }
    GraphNode {
        id: id.to_string(),
        kind: EntityKind::Finding,
        label: label.to_string(),
        aliases: Vec::new(),
        props,
        valid_from: Some(AS_OF_TS.to_string()),
        valid_to: None,
    }
}

fn doc_node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: EntityKind::Document,
        label: id.to_string(),
        aliases: Vec::new(),
        props: json!({ "plane": "user" }),
        valid_from: Some(AS_OF_TS.to_string()),
        valid_to: None,
    }
}

fn user_edge(from: &str, rel: &str, to: &str, note: Option<&str>) -> GraphEdge {
    let props = note.map_or(Value::Null, |n| json!({ "note": n }));
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props,
        provenance: Provenance::Agent {
            agent_id: USER_ID.to_string(),
            gated: false,
        },
        valid_from: Some(AS_OF_TS.to_string()),
        valid_to: None,
    }
}

/// Seed a fully consistent graph: thesis ← supports(f1) and ← contradicts(f2);
/// each finding `described_by` a real document node.
///
/// Edges require both endpoints to exist (engine contract), so every node is
/// always seeded. The unavailable-evidence path is exercised separately by the
/// [`HidingGraph`] wrapper, which makes one already-seeded node inaccessible at
/// read time — exactly the backend-inconsistency the export's honesty contract
/// defends against.
async fn seed_graph() -> Arc<InMemoryGraphEngine> {
    let graph = Arc::new(InMemoryGraphEngine::default());

    let nodes = vec![
        // Thesis root (Finding with kind_hint=thesis).
        GraphNode {
            id: "finding:thesis1".to_string(),
            kind: EntityKind::Finding,
            label: "AAPL is undervalued".to_string(),
            aliases: Vec::new(),
            props: json!({
                "title": "AAPL is undervalued",
                "kind_hint": "thesis",
                "horizon_date": "2026-12-31",
                "user_id": USER_ID,
                "as_of": AS_OF_TS,
            }),
            valid_from: Some(AS_OF_TS.to_string()),
            valid_to: None,
        },
        finding_node(
            "finding:f1",
            "Q3 revenue beat",
            Some(("finding-doc:f1", "Revenue beat consensus.")),
        ),
        finding_node(
            "finding:f2",
            "Margin compression risk",
            Some(("finding-doc:f2", "Margins fell QoQ.")),
        ),
        doc_node("finding-doc:f1"),
        doc_node("finding-doc:f2"),
    ];
    graph.upsert_nodes(nodes).await.expect("seed nodes");

    graph
        .upsert_edges(vec![
            user_edge(
                "finding:f1",
                "supports",
                "finding:thesis1",
                Some("revenue supports valuation"),
            ),
            user_edge(
                "finding:f2",
                "contradicts",
                "finding:thesis1",
                Some("margin risk"),
            ),
            user_edge("finding:f1", "described_by", "finding-doc:f1", None),
            user_edge("finding:f2", "described_by", "finding-doc:f2", None),
        ])
        .await
        .expect("seed edges");

    graph
}

/// A graph wrapper that makes one node id inaccessible at read time — its
/// `node()` lookup returns `None` even though edges still reference it. Models a
/// backend that has lost or cannot serve a node, the exact condition the
/// `[evidence unavailable: id]` contract must surface (never silently drop).
struct HidingGraph {
    inner: Arc<InMemoryGraphEngine>,
    hidden: String,
}

#[async_trait::async_trait]
impl GraphEngine for HidingGraph {
    async fn upsert_nodes(&self, nodes: Vec<GraphNode>) -> tdw_core::Result<()> {
        self.inner.upsert_nodes(nodes).await
    }
    async fn upsert_edges(&self, edges: Vec<GraphEdge>) -> tdw_core::Result<()> {
        self.inner.upsert_edges(edges).await
    }
    async fn node(&self, id: &str) -> tdw_core::Result<Option<GraphNode>> {
        if id == self.hidden {
            return Ok(None);
        }
        self.inner.node(id).await
    }
    async fn neighbors(
        &self,
        id: &str,
        filter: &tdw_core::TraversalFilter,
    ) -> tdw_core::Result<Vec<(GraphEdge, GraphNode)>> {
        self.inner.neighbors(id, filter).await
    }
    async fn expand(
        &self,
        seeds: &[String],
        filter: &tdw_core::TraversalFilter,
    ) -> tdw_core::Result<tdw_core::Subgraph> {
        self.inner.expand(seeds, filter).await
    }
    async fn shortest_path(
        &self,
        from: &str,
        to: &str,
        filter: &tdw_core::TraversalFilter,
    ) -> tdw_core::Result<Option<Vec<GraphEdge>>> {
        self.inner.shortest_path(from, to, filter).await
    }
    async fn edges(
        &self,
        rel: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> tdw_core::Result<Vec<GraphEdge>> {
        self.inner.edges(rel, offset, limit).await
    }
    async fn delete_edges(
        &self,
        from: &str,
        rel: &str,
        to: Option<&str>,
    ) -> tdw_core::Result<usize> {
        self.inner.delete_edges(from, rel, to).await
    }
    async fn replace_edges(
        &self,
        from: &str,
        rel: &str,
        new_edges: Vec<GraphEdge>,
    ) -> tdw_core::Result<()> {
        self.inner.replace_edges(from, rel, new_edges).await
    }
    async fn merge_entities(
        &self,
        source: &str,
        target: &str,
        decision: &tdw_core::MergeDecision,
    ) -> tdw_core::Result<tdw_core::MergeReport> {
        self.inner.merge_entities(source, target, decision).await
    }
}

fn server_with_engine(graph: Arc<dyn GraphEngine>) -> McpServer {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(graph)
        .with_user_id(USER_ID);
    let mut server = McpServer::new().with_knowledge(Arc::new(runtime));
    initialize(&mut server);
    server
}

fn server_with_graph(graph: Arc<InMemoryGraphEngine>) -> McpServer {
    server_with_engine(graph)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// The export descriptor appears in tools/list when a graph engine is attached.
#[test]
fn export_descriptor_present_with_graph() {
    let graph = block(seed_graph());
    let mut server = server_with_graph(graph);
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let present = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .any(|name| name == "tdw.kg.export");
    assert!(
        present,
        "tdw.kg.export must appear in tools/list when a graph engine is wired"
    );
}

/// (a) Seeded trail exports to JSON + Markdown with all evidence + citations.
#[test]
fn seeded_trail_exports_with_all_evidence_and_citations() {
    let graph = block(seed_graph());
    let mut server = server_with_graph(graph);

    let response = call(
        &mut server,
        "tdw.kg.export",
        &json!({
            "thesis_id": "finding:thesis1",
            "as_of": NOW,
            "generated_at": "caller-injected-2026-06-13T12:00:00Z",
        }),
    );

    assert_eq!(
        response["result"]["isError"], false,
        "export should succeed: {response}"
    );
    let sc = &response["result"]["structuredContent"];

    // Both evidence edges present (supports + contradicts).
    assert_eq!(sc["evidence_count"], 2, "two evidence edges expected: {sc}");
    // Two findings in the trail (f1, f2).
    assert_eq!(sc["findings_count"], 2);
    // No truncation.
    assert_eq!(sc["evidence_truncated"], false);

    // Citations resolve to the REAL document ids (both docs present).
    let json_artifact = &sc["json"];
    let citations = json_artifact["citations"].as_array().expect("citations");
    let cited_docs: Vec<&str> = citations
        .iter()
        .filter_map(|c| c["document_id"].as_str())
        .collect();
    assert!(
        cited_docs.contains(&"finding-doc:f1"),
        "f1 doc must be cited: {cited_docs:?}"
    );
    assert!(
        cited_docs.contains(&"finding-doc:f2"),
        "f2 doc must be cited: {cited_docs:?}"
    );

    // No unavailable nodes when every doc exists.
    assert_eq!(sc["unavailable_count"], 0, "no gaps expected: {sc}");

    // generated_at echoed verbatim into both artifacts.
    assert_eq!(sc["generated_at"], "caller-injected-2026-06-13T12:00:00Z");
    let markdown = sc["markdown"].as_str().expect("markdown string");
    assert!(markdown.contains("caller-injected-2026-06-13T12:00:00Z"));
    assert!(markdown.contains("# Research Trail"));
    assert!(markdown.contains("## Evidence"));
    assert!(markdown.contains("## Cited Answers"));
    // Provenance one-liner for the user-authored edges is present.
    assert!(
        markdown.contains("agent:user:analyst-1"),
        "provenance must surface in markdown: {markdown}"
    );
}

/// (b) A missing `described_by` document is recorded explicitly, not dropped.
#[test]
fn missing_evidence_node_is_marked_not_dropped() {
    // Seed a consistent graph, then wrap it so finding-doc:f2 is inaccessible at
    // read time (edges still reference it). This is the backend-inconsistency the
    // honesty contract must surface as "[evidence unavailable: …]".
    let inner = block(seed_graph());
    let graph: Arc<dyn GraphEngine> = Arc::new(HidingGraph {
        inner,
        hidden: "finding-doc:f2".to_string(),
    });
    let mut server = server_with_engine(graph);

    let response = call(
        &mut server,
        "tdw.kg.export",
        &json!({
            "thesis_id": "finding:thesis1",
            "as_of": NOW,
            "generated_at": "inject",
        }),
    );

    assert_eq!(response["result"]["isError"], false, "{response}");
    let sc = &response["result"]["structuredContent"];

    // The gap is counted and surfaced — not silently absent.
    assert_eq!(sc["unavailable_count"], 1, "one gap expected: {sc}");
    let unavailable = sc["json"]["unavailable"].as_array().expect("unavailable");
    assert!(
        unavailable
            .iter()
            .any(|v| v.as_str() == Some("[evidence unavailable: finding-doc:f2]")),
        "missing doc must be explicit in json: {unavailable:?}"
    );
    let markdown = sc["markdown"].as_str().expect("markdown");
    assert!(
        markdown.contains("[evidence unavailable: finding-doc:f2]"),
        "missing doc must be explicit in markdown: {markdown}"
    );

    // The other evidence is still intact — nothing was dropped wholesale.
    assert_eq!(sc["evidence_count"], 2);
}

/// (c) Two identical export calls are byte-identical (determinism).
#[test]
fn export_is_deterministic_across_two_runs() {
    let graph = block(seed_graph());
    let mut server = server_with_graph(graph);

    let args = json!({
        "thesis_id": "finding:thesis1",
        "as_of": NOW,
        "generated_at": "fixed-stamp",
    });

    let first = call(&mut server, "tdw.kg.export", &args);
    let second = call(&mut server, "tdw.kg.export", &args);

    let sc1 = &first["result"]["structuredContent"];
    let sc2 = &second["result"]["structuredContent"];

    // Markdown strings identical.
    assert_eq!(
        sc1["markdown"], sc2["markdown"],
        "markdown must be byte-identical across runs"
    );

    // JSON artifacts serialize to identical bytes.
    let j1 = serde_json::to_string(&sc1["json"]).expect("serialize j1");
    let j2 = serde_json::to_string(&sc2["json"]).expect("serialize j2");
    assert_eq!(j1, j2, "json must be byte-identical across runs");
}

/// A finding root (not a thesis) also exports its own pinned evidence as a citation.
#[test]
fn finding_root_exports_its_own_citation() {
    let graph = block(seed_graph());
    let mut server = server_with_graph(graph);

    let response = call(
        &mut server,
        "tdw.kg.export",
        &json!({
            "finding_id": "finding:f1",
            "as_of": NOW,
            "generated_at": "inject",
        }),
    );

    assert_eq!(response["result"]["isError"], false, "{response}");
    let sc = &response["result"]["structuredContent"];
    let citations = sc["json"]["citations"].as_array().expect("citations");
    assert!(
        citations
            .iter()
            .any(|c| c["document_id"].as_str() == Some("finding-doc:f1")),
        "finding root must cite its own document: {citations:?}"
    );
}

/// Zero or multiple roots is a tool error (not a panic, not a protocol error).
#[test]
fn ambiguous_or_missing_root_is_tool_error() {
    let graph = block(seed_graph());
    let mut server = server_with_graph(graph);

    let none = call(&mut server, "tdw.kg.export", &json!({ "as_of": NOW }));
    assert_eq!(
        none["result"]["isError"], true,
        "no root must error: {none}"
    );

    let both = call(
        &mut server,
        "tdw.kg.export",
        &json!({ "thesis_id": "finding:thesis1", "finding_id": "finding:f1" }),
    );
    assert_eq!(
        both["result"]["isError"], true,
        "two roots must error: {both}"
    );
}
