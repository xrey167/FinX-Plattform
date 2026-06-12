//! End-to-end tests for the MCP knowledge PATTERN tools (knowledge-system K-R4).
//!
//! Each test builds a [`KnowledgeRuntime`] over fully in-memory engines, seeds
//! a small temporal fixture (nodes + edges), seeds Pattern nodes via
//! [`PatternEngine`], attaches to an [`McpServer`], and drives
//! `tdw.kg.similar` and `tdw.kg.why` (Pattern branch) through the JSON-RPC surface.

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
use tdw_patterns::{MiningLimits, PatternEngine, PatternIndex};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::EntityKind;

// ── Timestamps ────────────────────────────────────────────────────────────────

const NOW: &str = "2025-06-01T00:00:00Z";
const WINDOW: &str = "2025-06-01";

// ── Shared graph wrapper (mirrors knowledge_explain_tools.rs) ─────────────────

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

/// Build a fixture graph with two motifs:
///
/// ```text
/// instrument:AAPL --listed_on--> venue:NASDAQ
/// instrument:MSFT --listed_on--> venue:NASDAQ
/// instrument:GOOG --listed_on--> venue:NASDAQ
/// instrument:AAPL --supplier_of--> instrument:MSFT
/// instrument:MSFT --supplier_of--> instrument:GOOG
/// ```
///
/// After mining, `listed_on` (support 3) and `supplier_of` (support 2) and
/// `listed_on::supplier_of` (support 2) are present.
///
/// Returns `(runtime, raw_graph)`.
async fn build_pattern_runtime() -> (Arc<KnowledgeRuntime>, Arc<InMemoryGraphEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    let prov = Provenance::Ingest {
        source: "test".to_string(),
    };

    let node = |id: &str, kind: EntityKind| GraphNode {
        id: id.to_string(),
        kind,
        label: id.to_string(),
        aliases: vec![],
        props: Value::Null,
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };
    let edge = |from: &str, rel: &str, to: &str| GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: Value::Null,
        provenance: prov.clone(),
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };

    graph
        .upsert_nodes(vec![
            node("instrument:AAPL", EntityKind::Instrument),
            node("instrument:MSFT", EntityKind::Instrument),
            node("instrument:GOOG", EntityKind::Instrument),
            node("venue:NASDAQ", EntityKind::Venue),
        ])
        .await
        .expect("upsert nodes");

    graph
        .upsert_edges(vec![
            edge("instrument:AAPL", "listed_on", "venue:NASDAQ"),
            edge("instrument:MSFT", "listed_on", "venue:NASDAQ"),
            edge("instrument:GOOG", "listed_on", "venue:NASDAQ"),
            edge("instrument:AAPL", "supplier_of", "instrument:MSFT"),
            edge("instrument:MSFT", "supplier_of", "instrument:GOOG"),
        ])
        .await
        .expect("upsert edges");

    // Run the pattern engine so Pattern nodes + pattern_instance_of edges exist
    // in the graph before the tool is called.
    let engine = PatternEngine::with_limits(MiningLimits {
        min_support: 2,
        ..MiningLimits::default()
    });
    let mut index = PatternIndex::default();
    engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("pattern mining");

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_lexical(lexical, "knowledge")
        .with_graph(Arc::new(SharedGraph(graph.clone())));

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
    let (runtime, _graph) = block(build_pattern_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    server
}

// ── spawn_pattern_mining_worker: config-gate tests (finding #2a) ──────────────

/// Mining on an empty graph (as when the worker first starts with enabled=true
/// on a fresh system) produces zero patterns — no error, just an empty report.
/// This verifies the disabled/empty-graph path without touching backend internals.
#[test]
fn spawn_worker_empty_graph_produces_no_patterns() {
    use std::sync::Arc;
    use tdw_storage_graph::InMemoryGraphEngine;

    let graph: Arc<dyn tdw_core::GraphEngine> = Arc::new(InMemoryGraphEngine::default());
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async move {
            let engine = PatternEngine::default();
            let mut index = PatternIndex::default();
            engine
                .run_pattern_mining_at(&graph, &mut index, NOW, WINDOW)
                .await
                .expect("empty mining")
        });
    assert_eq!(
        result.patterns_created, 0,
        "empty graph produces no patterns"
    );
    assert_eq!(result.motifs_examined, 0, "empty graph examines no motifs");
}

/// When enabled = true with a real tempdir index_path, a mining run persists
/// the index to disk and a subsequent load recovers the same entries.
#[test]
fn spawn_worker_index_persists_across_restart() {
    use std::sync::Arc;
    use tdw_storage_graph::InMemoryGraphEngine;

    let graph = Arc::new(InMemoryGraphEngine::default());
    let prov = Provenance::Ingest {
        source: "test".to_string(),
    };

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async move {
            let graph_dyn: Arc<dyn tdw_core::GraphEngine> = graph.clone();

            graph
                .upsert_nodes(vec![
                    GraphNode {
                        id: "instrument:X".to_string(),
                        kind: EntityKind::Instrument,
                        label: "X".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphNode {
                        id: "instrument:Y".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Y".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphNode {
                        id: "instrument:Z".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Z".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert nodes");

            graph
                .upsert_edges(vec![
                    GraphEdge {
                        from: "instrument:X".to_string(),
                        to: "instrument:Y".to_string(),
                        rel: "peer_of".to_string(),
                        props: Value::Null,
                        provenance: prov.clone(),
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphEdge {
                        from: "instrument:Y".to_string(),
                        to: "instrument:Z".to_string(),
                        rel: "peer_of".to_string(),
                        props: Value::Null,
                        provenance: prov,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert edges");

            let engine = PatternEngine::default();
            let mut index = PatternIndex::default();
            engine
                .run_pattern_mining_at(&graph_dyn, &mut index, NOW, WINDOW)
                .await
                .expect("first mining");
            index
        });

    // Simulate "persist to disk + reload" (the index_path path in the worker).
    let json = result.to_json().expect("serialize index");
    let restored = PatternIndex::from_json(&json).expect("deserialize index");

    assert_eq!(restored, result, "index round-trips exactly");
    assert!(restored.contains("peer_of"), "peer_of motif persisted");

    // Second mining run with the restored index must produce 0 new patterns.
    let report2 = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(async move {
            let graph2 = Arc::new(InMemoryGraphEngine::default());
            let prov2 = Provenance::Ingest {
                source: "test".to_string(),
            };
            let graph_dyn2: Arc<dyn tdw_core::GraphEngine> = graph2.clone();

            graph2
                .upsert_nodes(vec![
                    GraphNode {
                        id: "instrument:X".to_string(),
                        kind: EntityKind::Instrument,
                        label: "X".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphNode {
                        id: "instrument:Y".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Y".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphNode {
                        id: "instrument:Z".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Z".to_string(),
                        aliases: vec![],
                        props: Value::Null,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert nodes");

            graph2
                .upsert_edges(vec![
                    GraphEdge {
                        from: "instrument:X".to_string(),
                        to: "instrument:Y".to_string(),
                        rel: "peer_of".to_string(),
                        props: Value::Null,
                        provenance: prov2.clone(),
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                    GraphEdge {
                        from: "instrument:Y".to_string(),
                        to: "instrument:Z".to_string(),
                        rel: "peer_of".to_string(),
                        props: Value::Null,
                        provenance: prov2,
                        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                        valid_to: None,
                    },
                ])
                .await
                .expect("upsert edges");

            let engine2 = PatternEngine::default();
            let mut restored2 = restored;
            engine2
                .run_pattern_mining_at(&graph_dyn2, &mut restored2, NOW, WINDOW)
                .await
                .expect("second mining")
        });

    assert_eq!(
        report2.patterns_created, 0,
        "second run with restored index creates no new patterns"
    );
    assert!(
        report2.patterns_updated >= 1,
        "second run updates existing patterns"
    );
}

// ── tdw.kg.similar: descriptor gating ────────────────────────────────────────

/// `tdw.kg.similar` is absent when no knowledge runtime is attached.
#[test]
fn similar_absent_without_runtime() {
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
        !names.contains(&"tdw.kg.similar".to_string()),
        "tdw.kg.similar must be absent without a knowledge runtime"
    );
}

/// `tdw.kg.similar` is listed when a runtime with a graph engine is attached.
#[test]
fn similar_present_with_graph_runtime() {
    let mut server = server_with_runtime();
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"));
    let descriptor = tools
        .iter()
        .find(|t| t["name"] == "tdw.kg.similar")
        .unwrap_or_else(|| panic!("tdw.kg.similar must be listed"));
    // The descriptor must not advertise an "embedding" channel in K-R4.
    let desc_text = descriptor["description"].as_str().unwrap_or("");
    assert!(
        !desc_text.contains("embedding similarity (hybrid"),
        "descriptor must not claim hybrid embedding in K-R4; got: {desc_text}"
    );
}

// ── tdw.kg.similar: e2e motif recall (finding #2b) ───────────────────────────

/// AAPL and MSFT share both `listed_on` and `listed_on::supplier_of` patterns.
/// MSFT should surface as the top result for AAPL.
#[test]
fn similar_returns_shared_motif_results_with_scores() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.similar",
        &json!({ "entity_id": "instrument:AAPL", "top_k": 5 }),
    );
    assert_eq!(
        response["result"]["isError"], false,
        "tool must not error: {response}"
    );
    let sc = &response["result"]["structuredContent"];

    // channels_active must be ["motif"] only in K-R4.
    let channels = sc["channels_active"]
        .as_array()
        .unwrap_or_else(|| panic!("channels_active must be array"));
    assert_eq!(channels.len(), 1, "only motif channel in K-R4");
    assert_eq!(channels[0], "motif", "only motif channel in K-R4");

    // Results must include at least one entity with a motif score > 0.
    let results = sc["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results must be array"));
    assert!(!results.is_empty(), "should find similar entities for AAPL");

    // MSFT should appear — it shares listed_on and supplier_of with AAPL.
    let msft = results
        .iter()
        .find(|r| r["entity_id"] == "instrument:MSFT")
        .unwrap_or_else(|| panic!("instrument:MSFT must appear in similar results for AAPL"));

    // Score must be > 0.
    let score = msft["score"].as_f64().unwrap_or(0.0);
    assert!(
        score > 0.0,
        "MSFT motif score must be positive, got {score}"
    );

    // Per-channel motif score must be present.
    let motif_score = msft["channels"]["motif"]["score"].as_f64().unwrap_or(0.0);
    assert!(
        motif_score > 0.0,
        "MSFT per-channel motif score must be positive"
    );

    // matched_patterns must be non-empty.
    let matched = msft["channels"]["motif"]["matched_patterns"]
        .as_array()
        .unwrap_or_else(|| panic!("matched_patterns must be array"));
    assert!(
        !matched.is_empty(),
        "MSFT matched_patterns must be non-empty"
    );

    // The response must NOT include an embedding channel score in K-R4.
    assert!(
        msft.get("channels")
            .and_then(|c| c.get("embedding"))
            .is_none(),
        "embedding channel must be absent from results in K-R4"
    );
}

/// Entity not in the graph returns an empty result (not an error).
#[test]
fn similar_returns_empty_for_unknown_entity() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.similar",
        &json!({ "entity_id": "instrument:UNKNOWN" }),
    );
    assert_eq!(response["result"]["isError"], false);
    let sc = &response["result"]["structuredContent"];
    let results = sc["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results must be array"));
    assert!(
        results.is_empty(),
        "unknown entity must return empty results"
    );
}

/// top_k is respected: requesting top_k=1 returns at most 1 result.
#[test]
fn similar_respects_top_k() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.similar",
        &json!({ "entity_id": "instrument:AAPL", "top_k": 1 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let results = response["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results must be array"));
    assert!(results.len() <= 1, "top_k=1 must return at most 1 result");
}

// ── tdw.kg.why on Pattern nodes (finding #2c) ─────────────────────────────────

/// `tdw.kg.why` on a Pattern node surfaces mining provenance (kind: "pattern").
#[test]
fn why_on_pattern_node_surfaces_mining_provenance() {
    let mut server = server_with_runtime();

    // The pattern node id for "listed_on" is "pattern:listed_on".
    let response = call(
        &mut server,
        "tdw.kg.why",
        &json!({ "entity_id": "pattern:listed_on" }),
    );
    assert_eq!(
        response["result"]["isError"], false,
        "why on Pattern node must not error: {response}"
    );
    let sc = &response["result"]["structuredContent"];

    // The subject kind must be "entity".
    assert_eq!(sc["subject"]["kind"], "entity", "subject kind");

    // The chain must be non-empty.
    let chain = sc["chain"]
        .as_array()
        .unwrap_or_else(|| panic!("chain must be array"));
    assert!(
        !chain.is_empty(),
        "why chain for Pattern node must be non-empty"
    );

    // At least one step must surface pattern_mining_provenance.
    let has_mining_step = chain
        .iter()
        .any(|step| step["kind"] == "pattern_mining_provenance");
    assert!(
        has_mining_step,
        "why chain must contain a pattern_mining_provenance step; chain={chain:?}"
    );
}
