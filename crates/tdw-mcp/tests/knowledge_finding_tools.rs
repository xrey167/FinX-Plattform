//! End-to-end tests for the MCP finding tools (knowledge-system K-X6).
//!
//! Tests drive `tdw.kg.finding` and `tdw.kg.link` through the same JSON-RPC
//! `tools/call` surface a real client uses, using fully in-memory engines.
//! Contract: capture → auto-links reported → retrievable via `tdw.kg.search`
//! → link tool → duplicate rejection → dangling-target rejection → evidence
//! pinned and visible via `tdw.kg.why` → tombstoned entity excluded from
//! auto-links → descriptors absent without required attachment.

#![deny(clippy::pedantic, clippy::nursery)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Result, Subgraph,
    TraversalFilter,
};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};
use tdw_mcp::McpServer;
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::EntityKind;

const NOW: &str = "2026-06-12";
const LEXICAL_INDEX: &str = "knowledge";
const USER_ID: &str = "user:analyst-1";

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

/// Call one tool through the JSON-RPC surface, returning the decoded response.
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

/// Drive an async setup future to completion on a dedicated multi-thread
/// runtime that is fully torn down before returning — so the sync MCP server
/// calls run with NO ambient tokio runtime (exactly as the real serve loop).
fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

/// Build a fully-wired runtime: hash embedder, in-memory vector + lexical +
/// graph engines, and a bound user id so the finding surface is exposed.
/// Returns the runtime and the underlying graph (so tests can add edges).
async fn build_runtime_with_graph() -> (Arc<KnowledgeRuntime>, Arc<InMemoryGraphEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    // Seed an existing instrument node so auto-links have something to match.
    graph
        .upsert_nodes(vec![GraphNode {
            id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: Vec::new(),
            props: serde_json::Value::Null,
            valid_from: None,
            valid_to: None,
        }])
        .await
        .expect("seed node");

    // Index the instrument so tdw.kg.search can retrieve it.
    let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
    let mut indexer = KnowledgeIndexer::new(index)
        .with_lexical(lexical.clone(), LEXICAL_INDEX)
        .with_graph(Arc::new(SharedGraph(graph.clone())));

    indexer
        .index_at(
            KnowledgeDocument {
                id: "doc-aapl".to_string(),
                body: "Apple AAPL equity note".to_string(),
                entity: tdw_kg::Entity {
                    entity_id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: Vec::new(),
                },
                tags: vec!["asset:equity".to_string()],
                source: None,
                plane: Some("shared".to_string()),
                as_of: Some(NOW.to_string()),
                mentions: Vec::new(),
            },
            NOW,
        )
        .await
        .expect("index instrument");

    // The finding indexer wraps the same vector+lexical engines.
    let finding_index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
    let finding_indexer = KnowledgeIndexer::new(finding_index)
        .with_lexical(lexical.clone(), LEXICAL_INDEX)
        .with_graph(Arc::new(SharedGraph(graph.clone())));

    let finding_indexer_mutex = Arc::new(std::sync::Mutex::new(finding_indexer));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_lexical(lexical, LEXICAL_INDEX)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_user_id(USER_ID)
        .with_finding_indexer(finding_indexer_mutex);

    (Arc::new(runtime), graph)
}

fn server_with_findings() -> (McpServer, Arc<InMemoryGraphEngine>) {
    let (runtime, graph) = block(build_runtime_with_graph());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    (server, graph)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Finding tool descriptors are absent when the runtime has no graph or no
/// bound user id, and present when both are attached (F1 gating proof).
#[test]
fn descriptors_absent_without_required_attachment() {
    // No runtime at all.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let listed =
        decode(&bare.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0]);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"tdw.kg.finding"),
        "finding must be absent without runtime"
    );
    assert!(
        !names.contains(&"tdw.kg.link"),
        "link must be absent without runtime"
    );
}

/// Descriptors appear once a graph AND user id are wired.
#[test]
fn descriptors_present_with_full_attachment() {
    let (mut server, _graph) = server_with_findings();
    let listed =
        decode(&server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0]);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.contains(&"tdw.kg.finding"),
        "tdw.kg.finding must appear in tools/list when runtime is wired"
    );
    assert!(
        names.contains(&"tdw.kg.link"),
        "tdw.kg.link must appear in tools/list when runtime is wired"
    );
}

/// Capture a finding — response reports finding_id, auto_links (AAPL
/// mentioned in title), evidence_pinned=false for a bare capture.
#[test]
fn capture_finding_reports_id_and_auto_links() {
    let (mut server, _graph) = server_with_findings();

    let response = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "AAPL revenue beat expectations",
            "body":  "Apple reported Q3 revenue above consensus estimates.",
            "tags":  ["sector:tech"],
            "as_of": NOW
        }),
    );

    assert_eq!(
        response["result"]["isError"], false,
        "capture should succeed: {response}"
    );
    let sc = &response["result"]["structuredContent"];
    assert!(
        sc["finding_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("finding:")),
        "finding_id must start with 'finding:' prefix"
    );
    assert_eq!(sc["evidence_pinned"], false);
    // AAPL is a known entity; it should appear in auto_links.
    let auto_links = sc["auto_links"].as_array().expect("auto_links array");
    assert!(
        auto_links
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("AAPL"))),
        "AAPL should be auto-linked; got {auto_links:?}"
    );
}

/// After capture, the finding is retrievable via `tdw.kg.search`.
#[test]
fn captured_finding_is_searchable() {
    let (mut server, _graph) = server_with_findings();

    // Capture first.
    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "MSFT cloud growth unique phrase deltaX99",
            "as_of": NOW
        }),
    );
    assert_eq!(
        capture["result"]["isError"], false,
        "capture should succeed: {capture}"
    );

    // Search for the unique phrase.
    let search = call(
        &mut server,
        "tdw.kg.search",
        &json!({ "query": "deltaX99", "limit": 5 }),
    );
    assert_eq!(
        search["result"]["isError"], false,
        "search should succeed: {search}"
    );
    let hits = &search["result"]["structuredContent"]["hits"];
    assert!(
        hits.as_array().is_some_and(|arr| !arr.is_empty()),
        "the captured finding should be searchable: {hits}"
    );
}

/// `tdw.kg.link` creates a typed edge; duplicate rejection fires on repeat.
#[test]
fn link_tool_creates_edge_and_rejects_duplicate() {
    let (mut server, _graph) = server_with_findings();

    // Capture a finding to link FROM.
    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "AAPL supports thesis X7", "as_of": NOW }),
    );
    assert_eq!(capture["result"]["isError"], false);
    let finding_id = capture["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    // Link finding → instrument:AAPL.
    let link1 = call(
        &mut server,
        "tdw.kg.link",
        &json!({
            "from_finding_id": finding_id,
            "to": "instrument:AAPL",
            "rel": "supports"
        }),
    );
    assert_eq!(
        link1["result"]["isError"], false,
        "first link should succeed: {link1}"
    );
    let sc = &link1["result"]["structuredContent"];
    assert_eq!(sc["linked"]["from"].as_str(), Some(finding_id.as_str()));
    assert_eq!(sc["linked"]["rel"], "supports");
    assert_eq!(sc["linked"]["to"], "instrument:AAPL");

    // Duplicate — same from/rel/to must be rejected loudly.
    let link2 = call(
        &mut server,
        "tdw.kg.link",
        &json!({
            "from_finding_id": finding_id,
            "to": "instrument:AAPL",
            "rel": "supports"
        }),
    );
    assert_eq!(
        link2["result"]["isError"], true,
        "duplicate link should be a tool error: {link2}"
    );
    let content = link2["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        content.contains("already exists") || content.contains("duplicate"),
        "error should mention duplicate: {content}"
    );
}

/// `tdw.kg.link` rejects a dangling target (non-existent entity).
#[test]
fn link_tool_rejects_dangling_target() {
    let (mut server, _graph) = server_with_findings();

    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Finding for dangling test", "as_of": NOW }),
    );
    assert_eq!(capture["result"]["isError"], false);
    let finding_id = capture["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    let link = call(
        &mut server,
        "tdw.kg.link",
        &json!({
            "from_finding_id": finding_id,
            "to": "instrument:NONEXISTENT_ENTITY_ZZZZ",
            "rel": "relates_to"
        }),
    );
    assert_eq!(
        link["result"]["isError"], true,
        "dangling target should be a tool error: {link}"
    );
    let content = link["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        content.contains("does not exist"),
        "error should mention missing entity: {content}"
    );
}

/// Evidence is pinned and visible via `tdw.kg.why` (F3).
#[test]
fn evidence_pinned_and_visible_via_why() {
    let (mut server, _graph) = server_with_findings();

    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "AAPL margin compression risk",
            "as_of": NOW,
            "evidence": {
                "document_id": "sec-10q-aapl-2026",
                "snippet": "Operating margin declined 200bp year-over-year.",
                "source_url": "https://example.com/aapl-10q"
            }
        }),
    );
    assert_eq!(
        capture["result"]["isError"], false,
        "capture with evidence should succeed: {capture}"
    );
    assert_eq!(capture["result"]["structuredContent"]["evidence_pinned"], true);
    let finding_id = capture["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    // why on the finding should surface the evidence step.
    let why = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": finding_id }),
    );
    assert_eq!(
        why["result"]["isError"], false,
        "why should succeed: {why}"
    );
    let chain = why["result"]["structuredContent"]["chain"]
        .as_array()
        .expect("chain array");

    // Must contain a user_provenance step.
    assert!(
        chain
            .iter()
            .any(|step| step["kind"] == "user_provenance"),
        "why chain must contain a user_provenance step; chain={chain:?}"
    );

    // Must contain an evidence step with the snippet hash.
    let evidence_step = chain.iter().find(|step| step["kind"] == "evidence");
    assert!(
        evidence_step.is_some(),
        "why chain must contain an evidence step; chain={chain:?}"
    );
    let ev = evidence_step.expect("evidence step present");
    assert_eq!(ev["document_id"], "sec-10q-aapl-2026");
    assert!(
        ev["snippet_hash"].as_str().is_some_and(|h| !h.is_empty()),
        "evidence step must include snippet_hash"
    );
}

/// Tombstoned (merged) entity is excluded from auto-links (F5).
#[test]
fn tombstoned_entity_excluded_from_auto_links() {
    let (mut server, graph) = server_with_findings();

    // Add a second entity and tombstone it via merge_entities.
    block(async {
        graph
            .upsert_nodes(vec![GraphNode {
                id: "instrument:DEAD".to_string(),
                kind: EntityKind::Instrument,
                label: "Dead Corp".to_string(),
                aliases: Vec::new(),
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("add dead node");
        graph
            .merge_entities(
                "instrument:DEAD",
                "instrument:AAPL",
                &tdw_core::MergeDecision {
                    approved_by: "test".to_string(),
                },
            )
            .await
            .expect("merge dead into aapl");
    });

    // Mention DEAD in the title — it should NOT appear in auto_links.
    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "DEAD stock merged test case AAPL",
            "as_of": NOW
        }),
    );
    assert_eq!(
        capture["result"]["isError"], false,
        "capture should succeed: {capture}"
    );
    let auto_links = capture["result"]["structuredContent"]["auto_links"]
        .as_array()
        .expect("auto_links");
    assert!(
        !auto_links
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("DEAD"))),
        "tombstoned entity must be excluded from auto-links; got {auto_links:?}"
    );
}

/// Invalid relation is rejected.
#[test]
fn link_tool_rejects_invalid_relation() {
    let (mut server, _graph) = server_with_findings();

    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Relation test finding", "as_of": NOW }),
    );
    assert_eq!(capture["result"]["isError"], false);
    let finding_id = capture["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    let bad_rel = call(
        &mut server,
        "tdw.kg.link",
        &json!({
            "from_finding_id": finding_id,
            "to": "instrument:AAPL",
            "rel": "invented_relation"
        }),
    );
    assert_eq!(
        bad_rel["result"]["isError"], true,
        "invalid rel should be a tool error: {bad_rel}"
    );
}

/// Empty title is rejected as a validation error.
#[test]
fn capture_rejects_empty_title() {
    let (mut server, _graph) = server_with_findings();

    let response = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "   " }),
    );
    assert_eq!(
        response["result"]["isError"], true,
        "empty title should be a tool error: {response}"
    );
}

/// A finding without a user identity bound returns a tool error, not a
/// protocol error — the surface is safely gated.
#[test]
fn finding_tool_returns_tool_error_when_surface_unavailable() {
    // Build a runtime WITHOUT a user id — finding surface should be unavailable.
    let (runtime_no_user, _graph) = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());
        graph
            .upsert_nodes(vec![GraphNode {
                id: "instrument:X".to_string(),
                kind: EntityKind::Instrument,
                label: "X".to_string(),
                aliases: Vec::new(),
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("seed");
        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())));
        // No with_user_id — finding surface off.
        (Arc::new(runtime), graph)
    });

    let mut server = McpServer::new().with_knowledge(runtime_no_user);
    initialize(&mut server);

    // tools/list must NOT show finding tools.
    let listed =
        decode(&server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0]);
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"tdw.kg.finding"),
        "finding must be absent without user id"
    );

    // Direct call must return a tool error (not a protocol error -32601).
    let resp = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "test" }),
    );
    assert_eq!(
        resp["result"]["isError"], true,
        "should be tool error when surface unavailable: {resp}"
    );
    // Must NOT be a protocol -32601 error.
    assert!(resp["error"].is_null(), "must not be a protocol error: {resp}");
}

/// `tdw.kg.why` on a Finding without evidence still emits `user_provenance`
/// step but no `evidence` step (F3 honest-absent case).
#[test]
fn why_on_finding_without_evidence_emits_provenance_not_evidence() {
    let (mut server, _graph) = server_with_findings();

    let capture = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Simple finding no evidence", "as_of": NOW }),
    );
    assert_eq!(capture["result"]["isError"], false);
    let finding_id = capture["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    let why = call(&mut server, "tdw.kg.why", &json!({ "entity_id": finding_id }));
    assert_eq!(why["result"]["isError"], false, "why should succeed: {why}");
    let chain = why["result"]["structuredContent"]["chain"]
        .as_array()
        .expect("chain");

    assert!(
        chain.iter().any(|s| s["kind"] == "user_provenance"),
        "must contain user_provenance step"
    );
    assert!(
        !chain.iter().any(|s| s["kind"] == "evidence"),
        "must NOT contain evidence step when no evidence was pinned"
    );
}
