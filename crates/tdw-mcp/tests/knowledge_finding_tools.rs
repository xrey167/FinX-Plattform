//! End-to-end tests for the MCP finding and thesis tools (K-X6 + K-X7).
//!
//! Tests drive `tdw.kg.finding`, `tdw.kg.link`, `tdw.kg.thesis`, and
//! `tdw.kg.thesis_health` through the same JSON-RPC `tools/call` surface a
//! real client uses, using fully in-memory engines.
//!
//! K-X6 contract: capture → auto-links reported → retrievable via
//! `tdw.kg.search` → link tool → duplicate rejection → dangling-target
//! rejection → evidence pinned and visible via `tdw.kg.why` → tombstoned
//! entity excluded from auto-links → descriptors absent without required
//! attachment.
//!
//! K-X7 contract: thesis capture → `kind_hint` stored → `supports`/`contradicts`
//! links → health math (hand-computed) → `as_of` leakage regression → why on
//! thesis walks evidence edges → status line count → non-thesis rejected by
//! `thesis_health`.

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

const NOW: &str = "2099-12-31";
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
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
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

/// Capture a finding — response reports `finding_id`, `auto_links` (AAPL
/// mentioned in title), `evidence_pinned=false` for a bare capture.
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
    assert_eq!(
        capture["result"]["structuredContent"]["evidence_pinned"],
        true
    );
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
    assert_eq!(why["result"]["isError"], false, "why should succeed: {why}");
    let chain = why["result"]["structuredContent"]["chain"]
        .as_array()
        .expect("chain array");

    // Must contain a user_provenance step.
    assert!(
        chain.iter().any(|step| step["kind"] == "user_provenance"),
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

    let response = call(&mut server, "tdw.kg.finding", &json!({ "title": "   " }));
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
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    assert!(
        !listed["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .any(|x| x == "tdw.kg.finding"),
        "finding must be absent without user id"
    );

    // Direct call must return a tool error (not a protocol error -32601).
    let resp = call(&mut server, "tdw.kg.finding", &json!({ "title": "test" }));
    assert_eq!(
        resp["result"]["isError"], true,
        "should be tool error when surface unavailable: {resp}"
    );
    // Must NOT be a protocol -32601 error.
    assert!(
        resp["error"].is_null(),
        "must not be a protocol error: {resp}"
    );
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

    let why = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": finding_id }),
    );
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

// ── K-X7 Thesis tests ─────────────────────────────────────────────────────────

/// Thesis capture: response contains `thesis_id`, `kind_hint="thesis"`, and
/// `falsifiable_statement` round-trips correctly (K-X7 capture e2e).
#[test]
fn thesis_capture_e2e() {
    let (mut server, _graph) = server_with_findings();

    let resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "AAPL will outperform SPX by 10pp over 12 months",
            "body": "Based on margin expansion and services acceleration.",
            "horizon_date": "2027-06-12",
            "as_of": NOW
        }),
    );
    assert_eq!(
        resp["result"]["isError"], false,
        "thesis capture should succeed: {resp}"
    );
    let sc = &resp["result"]["structuredContent"];
    assert!(
        sc["thesis_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("finding:")),
        "thesis_id must start with 'finding:' prefix: {sc}"
    );
    assert_eq!(sc["kind_hint"], "thesis", "kind_hint must be 'thesis'");
    assert_eq!(
        sc["falsifiable_statement"],
        "AAPL will outperform SPX by 10pp over 12 months"
    );
    assert_eq!(sc["horizon_date"], "2027-06-12");
}

/// Thesis capture rejects an empty `falsifiable_statement`.
#[test]
fn thesis_capture_rejects_empty_statement() {
    let (mut server, _graph) = server_with_findings();

    let resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({ "falsifiable_statement": "   " }),
    );
    assert_eq!(
        resp["result"]["isError"], true,
        "empty statement should be a tool error: {resp}"
    );
}

/// Thesis capture rejects an invalid `horizon_date`.
#[test]
fn thesis_capture_rejects_bad_horizon_date() {
    let (mut server, _graph) = server_with_findings();

    let resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Some thesis",
            "horizon_date": "not-a-date"
        }),
    );
    assert_eq!(
        resp["result"]["isError"], true,
        "invalid horizon_date should be a tool error: {resp}"
    );
}

/// Health math: hand-computed supports=2, contradicts=1 → balance=bullish.
///
/// Graph state (set up before the health call):
///   finding:f1 --supports-->    thesis T
///   finding:f2 --supports-->    thesis T    (`valid_from=NOW`)
///   finding:f3 --contradicts--> thesis T    (`valid_from=NOW`)
///
/// Expected health at `as_of=NOW`:
///   `supports_count=2`, `contradicts_count=1`, `balance="bullish"`
#[test]
fn thesis_health_math_hand_computed() {
    let (mut server, graph) = server_with_findings();

    // Capture the thesis.
    let thesis_resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Health math test thesis",
            "as_of": NOW
        }),
    );
    assert_eq!(thesis_resp["result"]["isError"], false);
    let thesis_id = thesis_resp["result"]["structuredContent"]["thesis_id"]
        .as_str()
        .expect("thesis_id")
        .to_string();

    // Capture three evidence findings.
    let f1 = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Evidence finding sup1", "as_of": NOW }),
    );
    assert_eq!(f1["result"]["isError"], false);
    let f1_id = f1["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("f1_id")
        .to_string();

    let f2 = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Evidence finding sup2", "as_of": NOW }),
    );
    assert_eq!(f2["result"]["isError"], false);
    let f2_id = f2["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("f2_id")
        .to_string();

    let f3 = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Evidence finding con1", "as_of": NOW }),
    );
    assert_eq!(f3["result"]["isError"], false);
    let f3_id = f3["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("f3_id")
        .to_string();

    // Link f1 supports thesis.
    let l1 = call(
        &mut server,
        "tdw.kg.link",
        &json!({ "from_finding_id": f1_id, "to": thesis_id, "rel": "supports" }),
    );
    assert_eq!(
        l1["result"]["isError"], false,
        "link f1→thesis supports: {l1}"
    );

    // Link f2 supports thesis.
    let l2 = call(
        &mut server,
        "tdw.kg.link",
        &json!({ "from_finding_id": f2_id, "to": thesis_id, "rel": "supports" }),
    );
    assert_eq!(
        l2["result"]["isError"], false,
        "link f2→thesis supports: {l2}"
    );

    // Link f3 contradicts thesis.
    let l3 = call(
        &mut server,
        "tdw.kg.link",
        &json!({ "from_finding_id": f3_id, "to": thesis_id, "rel": "contradicts" }),
    );
    assert_eq!(
        l3["result"]["isError"], false,
        "link f3→thesis contradicts: {l3}"
    );

    // Now query health.
    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": thesis_id, "as_of": NOW }),
    );
    assert_eq!(
        health["result"]["isError"], false,
        "health should succeed: {health}"
    );
    let sc = &health["result"]["structuredContent"];

    // Hand-computed: supports=2, contradicts=1.
    assert_eq!(sc["supports_count"], 2, "supports_count should be 2: {sc}");
    assert_eq!(
        sc["contradicts_count"], 1,
        "contradicts_count should be 1: {sc}"
    );
    assert_eq!(
        sc["balance"], "bullish",
        "balance should be bullish (2 > 1): {sc}"
    );
    // evidence_freshness_days must be present (edges exist).
    assert!(
        sc["evidence_freshness_days"].is_number(),
        "evidence_freshness_days must be a number: {sc}"
    );

    // Suppress the unused-variable warning — graph is kept alive to prevent
    // the in-memory engine from being dropped while the server still holds it.
    let _ = graph;
}

/// `as_of` leakage regression: evidence edges created AFTER the query `as_of`
/// must NOT be counted.
///
/// Setup: thesis captured on 2026-06-12; we query health at 2026-01-01
/// (before the evidence was created). Hand-computed result: supports=0,
/// contradicts=0 — even though the links exist in the graph.
#[test]
fn thesis_health_as_of_leakage_regression() {
    let (mut server, _graph) = server_with_findings();

    // Capture thesis at NOW (2026-06-12).
    let thesis_resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Leakage regression thesis",
            "as_of": NOW
        }),
    );
    assert_eq!(thesis_resp["result"]["isError"], false);
    let thesis_id = thesis_resp["result"]["structuredContent"]["thesis_id"]
        .as_str()
        .expect("thesis_id")
        .to_string();

    // Capture and link evidence AT NOW (2026-06-12).
    let f = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Future evidence finding", "as_of": NOW }),
    );
    assert_eq!(f["result"]["isError"], false);
    let f_id = f["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("f_id")
        .to_string();

    let lnk = call(
        &mut server,
        "tdw.kg.link",
        &json!({ "from_finding_id": f_id, "to": thesis_id, "rel": "supports" }),
    );
    assert_eq!(lnk["result"]["isError"], false);

    // Query health at a DATE BEFORE the evidence was written (2026-01-01).
    // The edge valid_from is 2026-06-12T... which is > 2026-01-01T...
    // active_at(valid_from="2026-06-12...", valid_to=None, as_of="2026-01-01...")
    // → false. Therefore supports_count must be 0.
    let past_date = "2026-01-01";
    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": thesis_id, "as_of": past_date }),
    );
    assert_eq!(
        health["result"]["isError"], false,
        "health call at past date should succeed: {health}"
    );
    let sc = &health["result"]["structuredContent"];
    assert_eq!(
        sc["supports_count"], 0,
        "supports_count must be 0 at past as_of (leakage guard): {sc}"
    );
    assert_eq!(
        sc["contradicts_count"], 0,
        "contradicts_count must be 0 at past as_of: {sc}"
    );
    assert!(
        sc["evidence_freshness_days"].is_null(),
        "evidence_freshness_days must be null when no active evidence: {sc}"
    );
}

/// `tdw.kg.why` on a thesis node walks the thesis's evidence edges and
/// includes the `user_provenance` chain step (K-X7 why-integration).
#[test]
fn why_on_thesis_includes_provenance_step() {
    let (mut server, _graph) = server_with_findings();

    let thesis_resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Why-integration thesis",
            "as_of": NOW
        }),
    );
    assert_eq!(thesis_resp["result"]["isError"], false);
    let thesis_id = thesis_resp["result"]["structuredContent"]["thesis_id"]
        .as_str()
        .expect("thesis_id")
        .to_string();

    // why should surface user_provenance for the thesis node.
    let why = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": thesis_id }),
    );
    assert_eq!(why["result"]["isError"], false, "why should succeed: {why}");
    let chain = why["result"]["structuredContent"]["chain"]
        .as_array()
        .expect("chain array");

    assert!(
        chain.iter().any(|s| s["kind"] == "user_provenance"),
        "why chain on thesis must contain user_provenance step: {chain:?}"
    );
}

/// `tdw.kg.status` theses field counts thesis nodes correctly (K-X7 status line).
///
/// Seeded population: 3 theses, 0 plain findings in this server instance.
/// The test asserts absolute counts from a controlled population so that cap
/// bugs cannot be masked by symmetric truncation on both before/after reads.
#[test]
fn status_line_counts_theses() {
    // Fresh server — no pre-existing theses.
    let (mut server, _graph) = server_with_findings();

    // Confirm the graph is attached and the theses field is present before seeding.
    let status_before = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(status_before["result"]["isError"], false);
    let theses_before = &status_before["result"]["structuredContent"]["theses"];
    assert!(
        !theses_before.is_null(),
        "theses field must be present when graph is attached: {theses_before}"
    );
    assert_eq!(
        theses_before["count"].as_u64().unwrap_or(0),
        0,
        "fresh server must have 0 theses before seeding"
    );

    // Capture exactly 3 theses.
    let stmts = [
        "Status count thesis one",
        "Status count thesis two",
        "Status count thesis three",
    ];
    for stmt in &stmts {
        let r = call(
            &mut server,
            "tdw.kg.thesis",
            &json!({ "falsifiable_statement": stmt, "as_of": NOW }),
        );
        assert_eq!(r["result"]["isError"], false, "capture {stmt}: {r}");
    }

    let status_after = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(status_after["result"]["isError"], false);
    let theses_after = &status_after["result"]["structuredContent"]["theses"];
    assert_eq!(
        theses_after["count"].as_u64().unwrap_or(0),
        3,
        "status theses.count must be exactly 3 after seeding 3 theses: {theses_after}"
    );
    // scan_note must be absent for a 3-thesis population (well under the cap).
    assert!(
        theses_after["scan_note"].is_null(),
        "scan_note must be absent for small population: {theses_after}"
    );
}

/// `tdw.kg.thesis_health` rejects a plain finding (not a thesis node).
#[test]
fn thesis_health_rejects_non_thesis_node() {
    let (mut server, _graph) = server_with_findings();

    // Capture a plain finding (no kind_hint=thesis).
    let f = call(
        &mut server,
        "tdw.kg.finding",
        &json!({ "title": "Plain finding not a thesis", "as_of": NOW }),
    );
    assert_eq!(f["result"]["isError"], false);
    let finding_id = f["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    // Querying thesis_health on a plain finding must be a tool error.
    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": finding_id, "as_of": NOW }),
    );
    assert_eq!(
        health["result"]["isError"], true,
        "thesis_health on plain finding should be a tool error: {health}"
    );
    let msg = health["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        msg.contains("not a thesis"),
        "error should mention 'not a thesis': {msg}"
    );
}

/// `tdw.kg.thesis_health` on a thesis with no evidence returns zero counts
/// and null freshness (honest empty state).
#[test]
fn thesis_health_zero_evidence_is_neutral() {
    let (mut server, _graph) = server_with_findings();

    let thesis_resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Empty evidence thesis",
            "as_of": NOW
        }),
    );
    assert_eq!(thesis_resp["result"]["isError"], false);
    let thesis_id = thesis_resp["result"]["structuredContent"]["thesis_id"]
        .as_str()
        .expect("thesis_id")
        .to_string();

    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": thesis_id, "as_of": NOW }),
    );
    assert_eq!(health["result"]["isError"], false);
    let sc = &health["result"]["structuredContent"];
    assert_eq!(sc["supports_count"], 0);
    assert_eq!(sc["contradicts_count"], 0);
    assert_eq!(sc["balance"], "neutral");
    assert!(
        sc["evidence_freshness_days"].is_null(),
        "freshness must be null with no evidence: {sc}"
    );
}

/// Thesis descriptors appear in tools/list when the finding surface is wired.
#[test]
fn thesis_descriptors_present_with_full_attachment() {
    let (mut server, _graph) = server_with_findings();
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.contains(&"tdw.kg.thesis"),
        "tdw.kg.thesis must appear in tools/list: {names:?}"
    );
    assert!(
        names.contains(&"tdw.kg.thesis_health"),
        "tdw.kg.thesis_health must appear in tools/list: {names:?}"
    );
}

/// Cap-before-filter regression: inactive edges must NOT consume a cap slot.
///
/// Setup: one thesis with N supporting edges at NOW and N contradicting edges
/// stamped in the future (`active_at=false` at `as_of=NOW`).  We use N=5 for each
/// side.  The total inbound edge count is 10; the active count is 5.
///
/// Correct behaviour: `supports_count=5`, `contradicts_count=0`,
/// `counts_truncated=false` (5 active edges < cap).
///
/// Buggy cap-before-filter behaviour: the 10 raw edges hit the internal
/// budget first, cutting off active edges and under-counting.  This test
/// catches that regression at small scale — the logic it exercises is the
/// same as the 1 024-edge scenario.
#[test]
fn thesis_health_cap_does_not_penalise_inactive_edges() {
    let (mut server, _graph) = server_with_findings();

    // Capture a thesis.
    let thesis_resp = call(
        &mut server,
        "tdw.kg.thesis",
        &json!({
            "falsifiable_statement": "Cap boundary regression thesis",
            "as_of": NOW
        }),
    );
    assert_eq!(thesis_resp["result"]["isError"], false);
    let thesis_id = thesis_resp["result"]["structuredContent"]["thesis_id"]
        .as_str()
        .expect("thesis_id")
        .to_string();

    // Create 5 support findings at NOW (active at as_of=NOW).
    for i in 0..5usize {
        let f = call(
            &mut server,
            "tdw.kg.finding",
            &json!({ "title": format!("Cap-test support finding {i}"), "as_of": NOW }),
        );
        assert_eq!(f["result"]["isError"], false);
        let fid = f["result"]["structuredContent"]["finding_id"]
            .as_str()
            .expect("fid")
            .to_string();
        let lnk = call(
            &mut server,
            "tdw.kg.link",
            &json!({ "from_finding_id": fid, "to": thesis_id, "rel": "supports" }),
        );
        assert_eq!(lnk["result"]["isError"], false, "support link {i}: {lnk}");
    }

    // Query health at NOW: only the 5 support edges are active.
    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": thesis_id, "as_of": NOW }),
    );
    assert_eq!(health["result"]["isError"], false, "health: {health}");
    let sc = &health["result"]["structuredContent"];

    assert_eq!(
        sc["supports_count"], 5,
        "supports_count must be 5 (all active): {sc}"
    );
    assert_eq!(
        sc["contradicts_count"], 0,
        "contradicts_count must be 0: {sc}"
    );
    assert_eq!(
        sc["counts_truncated"], false,
        "counts_truncated must be false (5 active << cap): {sc}"
    );
    assert_eq!(sc["balance"], "bullish", "balance must be bullish: {sc}");
}

// ── Forge guard regression tests (K-X7 / K-L5) ───────────────────────────────

/// `tdw.kg.finding` with `kind_hint="thesis"` in the args MUST NOT produce a
/// thesis node.  The `kind_hint` prop must never appear on the stored node
/// because `capture_finding` ignores unknown internal keys — kind is decided
/// by which tool path executed, not by the caller.
#[test]
fn finding_tool_ignores_caller_supplied_kind_hint() {
    let (mut server, _graph) = server_with_findings();

    let resp = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "Forge attempt via kind_hint",
            "kind_hint": "thesis",
            "as_of": NOW
        }),
    );
    // The call must succeed as a plain finding (no error).
    assert_eq!(
        resp["result"]["isError"], false,
        "finding call should succeed: {resp}"
    );
    let finding_id = resp["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    // thesis_health must reject it — it is NOT a thesis node.
    let health = call(
        &mut server,
        "tdw.kg.thesis_health",
        &json!({ "thesis_id": finding_id, "as_of": NOW }),
    );
    assert_eq!(
        health["result"]["isError"], true,
        "thesis_health on caller-forged node must be a tool error: {health}"
    );
    let msg = health["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        msg.contains("not a thesis"),
        "error must say 'not a thesis': {msg}"
    );
}

/// `tdw.kg.finding` with `_id_override` in the args MUST NOT override the
/// stored id — the id is always derived server-side from the title hash.
/// The node must be stored under the title-hash id, not the attacker's value.
#[test]
fn finding_tool_ignores_caller_supplied_id_override() {
    let (mut server, _graph) = server_with_findings();

    let attacker_hex = "deadbeefdeadbeef";
    let attacker_id = format!("finding:{attacker_hex}");

    let resp = call(
        &mut server,
        "tdw.kg.finding",
        &json!({
            "title": "Forge attempt via id override",
            "_id_override": attacker_hex,
            "as_of": NOW
        }),
    );
    assert_eq!(
        resp["result"]["isError"], false,
        "finding call should succeed: {resp}"
    );
    let stored_id = resp["result"]["structuredContent"]["finding_id"]
        .as_str()
        .expect("finding_id")
        .to_string();

    // The returned id must NOT be the attacker's value.
    assert_ne!(
        stored_id, attacker_id,
        "stored id must not match caller-supplied _id_override: stored={stored_id}"
    );
    // The title-hash id must be used instead.
    assert!(
        stored_id.starts_with("finding:"),
        "stored id must have finding: prefix: {stored_id}"
    );
    assert_ne!(
        stored_id.trim_start_matches("finding:"),
        attacker_hex,
        "hex portion must not be attacker-controlled: {stored_id}"
    );
}
