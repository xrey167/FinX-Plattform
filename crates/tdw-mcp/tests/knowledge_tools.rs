//! End-to-end tests for the MCP knowledge READ tools (knowledge-system B8).
//!
//! Each test builds a [`KnowledgeRuntime`] over fully in-memory engines (hash
//! embedder + in-process vector/lexical/graph/tags), indexes a few documents
//! through [`KnowledgeIndexer`], attaches the runtime to an [`McpServer`], and
//! drives every tool through the SAME JSON-RPC `tools/call` surface a real
//! client uses. The contract under test: explained search hits + versions,
//! entity node+neighbors, the traverse hop cap (4 → tool error) with provenance
//! visible, path found/missed, all three `tdw.tags.query` modes plus the
//! ambiguous-args error, and — crucially — that EVERY knowledge tool is a tool
//! error (not a protocol error) when the runtime is absent, with descriptors
//! gated on attachment.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Result, Subgraph,
    TraversalFilter,
};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex, collection_name};
use tdw_mcp::McpServer;
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tags::{TagAssignment, TagDefinition, TagEngine};

const NOW: &str = "2026-05-22";
const LEXICAL_INDEX: &str = "knowledge";

/// A shareable [`GraphEngine`] handle: one in-memory graph behind an `Arc`,
/// delegating the whole trait. Lets the indexer, the runtime's graph handle, and
/// the graph-backed tag engine all read/write the SAME store in a test.
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

fn document(
    id: &str,
    body: &str,
    entity_id: &str,
    label: &str,
    tags: Vec<String>,
) -> KnowledgeDocument {
    KnowledgeDocument {
        id: id.to_string(),
        body: body.to_string(),
        entity: Entity {
            entity_id: entity_id.to_string(),
            kind: EntityKind::Instrument,
            label: label.to_string(),
            aliases: Vec::new(),
        },
        tags,
        source: None,
        plane: Some("shared".to_string()),
        as_of: Some(NOW.to_string()),
        mentions: Vec::new(),
    }
}

/// Build a fully wired runtime over shared in-memory engines, indexing three
/// linked documents and seeding the temporal tag taxonomy. Returns the runtime
/// plus the shared graph (so tests can add extra edges).
async fn build_runtime() -> (Arc<KnowledgeRuntime>, Arc<InMemoryGraphEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    // The index shares the runtime's embedder + vector engine so the search tool
    // reads exactly what the indexer wrote.
    let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
    let mut indexer = KnowledgeIndexer::new(index)
        .with_lexical(lexical.clone(), LEXICAL_INDEX)
        .with_graph(Arc::new(SharedGraph(graph.clone())));

    for document in [
        document(
            "doc-aapl",
            "Apple AAPL equity momentum research note",
            "instrument:AAPL",
            "Apple",
            vec!["asset:equity".to_string()],
        ),
        document(
            "doc-msft",
            "Microsoft MSFT equity software platform",
            "instrument:MSFT",
            "Microsoft",
            vec!["asset:equity".to_string()],
        ),
    ] {
        indexer
            .index_at(document, NOW)
            .await
            .unwrap_or_else(|error| panic!("index should succeed: {error}"));
    }

    // A peer edge so traverse/path have something to walk between the two
    // entity nodes (the documents only link entity → document).
    graph
        .upsert_edges(vec![GraphEdge {
            from: "instrument:AAPL".to_string(),
            to: "instrument:MSFT".to_string(),
            rel: "peer_of".to_string(),
            props: Value::Null,
            provenance: tdw_core::Provenance::Rule {
                rule_id: "derived:peer".to_string(),
                version: 1,
            },
            valid_from: None,
            valid_to: None,
        }])
        .await
        .unwrap_or_else(|error| panic!("peer edge should upsert: {error}"));

    // The tag engine writes into the SAME graph, so tdw.tags.query and the
    // retriever's tag channel see the taxonomy + assignments.
    let tags = GraphTagEngine::new(SharedGraph(graph.clone()));
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .await
    .unwrap_or_else(|error| panic!("define parent: {error}"));
    tags.define(TagDefinition {
        tag_id: "asset:equity:tech".to_string(),
        parent: Some("asset:equity".to_string()),
        ttl_days: None,
    })
    .await
    .unwrap_or_else(|error| panic!("define child: {error}"));
    tags.assign(TagAssignment {
        entity_id: "instrument:AAPL".to_string(),
        tag_id: "asset:equity:tech".to_string(),
        assigned_at: NOW.to_string(),
        expires_at: None,
        provenance: "fixture".to_string(),
    })
    .await
    .unwrap_or_else(|error| panic!("assign: {error}"));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_lexical(lexical, LEXICAL_INDEX)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))))
        .with_versions(Some(7), Some(3));
    (Arc::new(runtime), graph)
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

/// Drive an async setup future to completion on a dedicated runtime that is
/// fully torn down before returning — so the subsequent SYNC server calls run
/// with NO ambient tokio runtime, exactly as the real MCP serve loop (a plain
/// std thread) does. This is what lets `knowledge_tools`' own per-call
/// current-thread runtime bridge work in tests without nesting runtimes.
fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

fn server_with_runtime() -> McpServer {
    let (runtime, _graph) = block(build_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    server
}

#[test]
fn descriptors_gated_on_attachment() {
    // Without a runtime, none of the six tools are listed.
    let mut bare = McpServer::new();
    initialize(&mut bare);
    let listed =
        decode(&bare.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0]);
    let names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"))
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(ToString::to_string))
        .collect();
    for name in [
        "tdw.kg.search",
        "tdw.kg.entity",
        "tdw.kg.traverse",
        "tdw.kg.path",
        "tdw.tags.query",
        "tdw.kg.status",
    ] {
        assert!(!names.contains(&name.to_string()), "{name} must be absent");
    }

    // With a runtime, all six appear with read-only annotations.
    let mut server = server_with_runtime();
    let listed = decode(
        &server.handle_json_rpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)[0],
    );
    let tools = listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools array"));
    for name in [
        "tdw.kg.search",
        "tdw.kg.entity",
        "tdw.kg.traverse",
        "tdw.kg.path",
        "tdw.tags.query",
        "tdw.kg.status",
    ] {
        let descriptor = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} must be listed"));
        assert_eq!(descriptor["annotations"]["readOnlyHint"], true);
    }
}

#[test]
fn search_returns_explained_hits_and_versions() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({ "query": "AAPL momentum", "top_k": 5 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let structured = &response["result"]["structuredContent"];
    let hits = structured["hits"]
        .as_array()
        .unwrap_or_else(|| panic!("hits array"));
    assert!(!hits.is_empty(), "search should find the indexed documents");
    let first = &hits[0];
    assert!(first["id"].is_string());
    assert!(first["entity_id"].is_string());
    // Explanation surfaces channel evidence.
    assert!(first["explanation"]["channels"].is_array());
    // Versions are stamped from with_versions(Some(7), Some(3)).
    assert_eq!(structured["versions"]["rules_version"], 7);
    assert_eq!(structured["versions"]["infer_version"], 3);
    assert!(structured["versions"]["embedder_model"].is_string());
}

#[test]
fn entity_returns_node_and_neighbors() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.entity",
        &json!({ "id": "instrument:AAPL" }),
    );
    assert_eq!(response["result"]["isError"], false);
    let structured = &response["result"]["structuredContent"];
    assert_eq!(structured["node"]["id"], "instrument:AAPL");
    let neighbors = structured["neighbors"]
        .as_array()
        .unwrap_or_else(|| panic!("neighbors array"));
    // AAPL reaches its document node and the MSFT peer.
    assert!(
        neighbors
            .iter()
            .any(|neighbor| neighbor["node"]["id"] == "instrument:MSFT"),
        "AAPL should neighbor MSFT"
    );
    // Each neighbor entry pairs an edge with the reached node.
    assert!(
        neighbors
            .iter()
            .all(|neighbor| neighbor["edge"].is_object())
    );
}

#[test]
fn traverse_caps_hops_and_shows_provenance() {
    let mut server = server_with_runtime();

    // max_hops 4 exceeds the B8 cap of 3 → tool error, not a protocol error.
    let capped = call(
        &mut server,
        "tdw.kg.traverse",
        &json!({ "seeds": ["instrument:AAPL"], "max_hops": 4 }),
    );
    assert!(
        capped.get("error").is_none(),
        "must be a tool error, not protocol"
    );
    assert_eq!(capped["result"]["isError"], true);
    assert!(
        capped["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("max_hops"),
        "error names the hop cap"
    );

    // A valid traverse returns a subgraph with provenance VISIBLE on the edge.
    let response = call(
        &mut server,
        "tdw.kg.traverse",
        &json!({ "seeds": ["instrument:AAPL"], "max_hops": 2 }),
    );
    assert_eq!(response["result"]["isError"], false);
    let structured = &response["result"]["structuredContent"];
    let edges = structured["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("edges array"));
    let peer = edges
        .iter()
        .find(|edge| edge["rel"] == "peer_of")
        .unwrap_or_else(|| panic!("peer_of edge should be traversed"));
    // Rule provenance (derived:*) is serialized as-is and visible.
    assert_eq!(peer["provenance"]["kind"], "rule");
    assert_eq!(peer["provenance"]["rule_id"], "derived:peer");
}

#[test]
fn path_finds_and_misses() {
    let mut server = server_with_runtime();

    let found = call(
        &mut server,
        "tdw.kg.path",
        &json!({ "from": "instrument:AAPL", "to": "instrument:MSFT" }),
    );
    assert_eq!(found["result"]["isError"], false);
    let path = found["result"]["structuredContent"]["path"]
        .as_array()
        .unwrap_or_else(|| panic!("path should be found"));
    assert_eq!(path[0]["rel"], "peer_of");

    // An unreachable target yields a null path (still a success).
    let missed = call(
        &mut server,
        "tdw.kg.path",
        &json!({ "from": "instrument:AAPL", "to": "instrument:NONE" }),
    );
    assert_eq!(missed["result"]["isError"], false);
    assert!(missed["result"]["structuredContent"]["path"].is_null());
}

#[test]
fn tags_query_all_three_modes_and_ambiguous_error() {
    let mut server = server_with_runtime();

    // entity_id + as_of → active tags.
    let active = call(
        &mut server,
        "tdw.tags.query",
        &json!({ "entity_id": "instrument:AAPL", "as_of": NOW }),
    );
    assert_eq!(active["result"]["isError"], false);
    let active_tags = active["result"]["structuredContent"]["active_tags"]
        .as_array()
        .unwrap_or_else(|| panic!("active_tags"));
    assert!(active_tags.iter().any(|tag| tag == "asset:equity:tech"));

    // tag_id alone → descendants.
    let descendants = call(
        &mut server,
        "tdw.tags.query",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert_eq!(descendants["result"]["isError"], false);
    let descendant_ids = descendants["result"]["structuredContent"]["descendants"]
        .as_array()
        .unwrap_or_else(|| panic!("descendants"));
    assert!(descendant_ids.iter().any(|tag| tag == "asset:equity:tech"));

    // tag_id + as_of → entities with the tag.
    let entities = call(
        &mut server,
        "tdw.tags.query",
        &json!({ "tag_id": "asset:equity:tech", "as_of": NOW }),
    );
    assert_eq!(entities["result"]["isError"], false);
    let entity_ids = entities["result"]["structuredContent"]["entities_with_tag"]
        .as_array()
        .unwrap_or_else(|| panic!("entities_with_tag"));
    assert!(entity_ids.iter().any(|entity| entity == "instrument:AAPL"));

    // Ambiguous (entity_id + tag_id) → tool error with a usage message.
    let ambiguous = call(
        &mut server,
        "tdw.tags.query",
        &json!({ "entity_id": "instrument:AAPL", "tag_id": "asset:equity" }),
    );
    assert!(ambiguous.get("error").is_none());
    assert_eq!(ambiguous["result"]["isError"], true);
    assert!(
        ambiguous["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly one"),
        "ambiguous args produce a usage message"
    );
}

#[test]
fn unknown_entity_kind_is_a_tool_error() {
    let mut server = server_with_runtime();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({ "query": "AAPL", "entity_kinds": ["not_a_real_kind"] }),
    );
    assert!(response.get("error").is_none(), "must be a tool error");
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown entity kind")
    );
}

#[test]
fn every_tool_without_runtime_is_a_tool_error_not_protocol() {
    let mut server = McpServer::new();
    initialize(&mut server);
    let cases = [
        ("tdw.kg.search", json!({ "query": "x" })),
        ("tdw.kg.entity", json!({ "id": "instrument:AAPL" })),
        ("tdw.kg.traverse", json!({ "seeds": ["instrument:AAPL"] })),
        ("tdw.kg.path", json!({ "from": "a", "to": "b" })),
        ("tdw.tags.query", json!({ "tag_id": "asset:equity" })),
        ("tdw.kg.status", json!({})),
    ];
    for (name, arguments) in cases {
        let response = call(&mut server, name, &arguments);
        assert!(
            response.get("error").is_none(),
            "{name} without runtime must NOT be a protocol error"
        );
        assert_eq!(
            response["result"]["isError"], true,
            "{name} without runtime must be a tool error"
        );
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .contains("knowledge runtime not attached"),
            "{name} reports the missing runtime"
        );
    }
}

#[test]
fn entity_without_graph_is_a_tool_error() {
    // A vector-only runtime (no graph/tags) serves search but reports the graph
    // tools as unavailable via a tool error.
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let runtime = Arc::new(KnowledgeRuntime::new(embedder, vectors));
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);

    let response = call(
        &mut server,
        "tdw.kg.entity",
        &json!({ "id": "instrument:AAPL" }),
    );
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("knowledge graph not attached")
    );

    let tags = call(
        &mut server,
        "tdw.tags.query",
        &json!({ "tag_id": "asset:equity" }),
    );
    assert_eq!(tags["result"]["isError"], true);
    assert!(
        tags["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("tag engine not attached")
    );
}

/// B8 security review: the sync→async bridge must reuse an ambient tokio
/// runtime — building a fresh one inside `block_in_place` on a multi-thread
/// runtime (the embedded `McpOnly` shape) previously panicked per tool call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn knowledge_tools_survive_an_ambient_runtime() {
    let (runtime, _graph) = build_runtime().await;
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = tokio::task::block_in_place(|| {
        call(
            &mut server,
            "tdw.kg.search",
            &json!({ "query": "Apple AAPL" }),
        )
    });
    assert!(
        response.get("error").is_none(),
        "search inside an ambient runtime must succeed: {response}"
    );
}

/// B8 security review: query strings are attacker-controlled and API
/// embedders bill per token — oversized queries are rejected as tool errors.
#[tokio::test]
async fn search_rejects_oversized_queries() {
    let (runtime, _graph) = build_runtime().await;
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let huge = "a".repeat(8193);
    let response = call(&mut server, "tdw.kg.search", &json!({ "query": huge }));
    let text = response.to_string();
    assert!(
        text.contains("8192"),
        "oversized query must be a tool error naming the cap: {text}"
    );
}

/// K-E2: `tdw.kg.status` returns a snapshot with every observability field
/// populated for the seeded runtime (`with_versions(Some(7), Some(3))`).
#[test]
fn kg_status_returns_all_fields() {
    let mut server = server_with_runtime();
    let response = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(
        response["result"]["isError"], false,
        "status must succeed: {response}"
    );
    let s = &response["result"]["structuredContent"];

    // Vector / document channel — assert exact seeded values, not just presence.
    assert_eq!(
        s["embedder_model"], "local-hash-8",
        "embedder_model seeded: {s}"
    );
    assert!(
        s["vector_collection"]
            .as_str()
            .is_some_and(|v| v.starts_with("tdw_knowledge__")),
        "vector_collection namespaced: {s}"
    );
    assert!(
        s["document_count_note"].is_string(),
        "document_count_note present: {s}"
    );

    // Taxonomy — exactly 53 EntityKind variants (52 + Pattern added by K-R4).
    assert_eq!(
        s["taxonomy_kind_count"].as_u64(),
        Some(53),
        "taxonomy_kind_count is 53: {s}"
    );

    // Versions seeded via with_versions(Some(7), Some(3)).
    assert_eq!(s["versions"]["rules_version"], 7, "rules_version");
    assert_eq!(s["versions"]["infer_version"], 3, "infer_version");
    assert_eq!(
        s["versions"]["embedder_model"], "local-hash-8",
        "versions.embedder_model seeded: {s}"
    );

    // Language-model grade — no resolver/agent_id in the test runtime → stub.
    assert!(
        s["language_model_grade"]
            .as_str()
            .is_some_and(|g| g.contains("stub")),
        "language_model_grade is stub: {s}"
    );
}

/// K-E2: `tdw.kg.status` with graph attached reports graph health and the
/// explicit backend name set via `with_graph_name`.
#[test]
fn kg_status_reports_graph_health_when_attached() {
    let (runtime, _graph) = block(build_runtime());
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(response["result"]["isError"], false);
    let s = &response["result"]["structuredContent"];
    // The in-memory graph is always reachable.
    let health = &s["graph_health"];
    assert!(health.is_object(), "graph_health present: {s}");
    assert_eq!(health["reachable"], true, "in-memory graph reachable");
    // build_runtime() does not call with_graph_name → sentinel "graph-engine".
    assert_eq!(
        health["backend_name"], "graph-engine",
        "sentinel name when not explicitly set: {s}"
    );
}

/// K-E2: `tdw.kg.status` on a vector-only runtime (no graph) returns `null`
/// `graph_health` and `null` proposals.
#[test]
fn kg_status_null_optionals_on_vector_only_runtime() {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let runtime = Arc::new(KnowledgeRuntime::new(embedder, vectors));
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = call(&mut server, "tdw.kg.status", &json!({}));
    assert_eq!(response["result"]["isError"], false);
    let s = &response["result"]["structuredContent"];
    assert!(
        s["graph_health"].is_null(),
        "no graph → graph_health is null: {s}"
    );
    assert!(
        s["proposals"].is_null(),
        "no proposal queue → proposals is null: {s}"
    );
}

/// B8 security review: traverse walks under hard budgets — a hub node whose
/// neighborhood exceeds the per-anchor ceiling is a tool error, not an
/// unbounded response.
#[tokio::test]
async fn traverse_rejects_hub_fanout() {
    let (runtime, graph) = build_runtime().await;
    // A star: hub -> 70 spokes (over the 64 per-anchor ceiling).
    let mut nodes = vec![GraphNode {
        id: "hub:center".to_string(),
        kind: EntityKind::Instrument,
        label: "hub".to_string(),
        aliases: Vec::new(),
        props: Value::Null,
        valid_from: None,
        valid_to: None,
    }];
    let mut edges = Vec::new();
    for index in 0..70 {
        nodes.push(GraphNode {
            id: format!("spoke:{index}"),
            kind: EntityKind::Instrument,
            label: format!("spoke {index}"),
            aliases: Vec::new(),
            props: Value::Null,
            valid_from: None,
            valid_to: None,
        });
        edges.push(GraphEdge {
            from: "hub:center".to_string(),
            to: format!("spoke:{index}"),
            rel: "links_to".to_string(),
            props: Value::Null,
            provenance: tdw_core::Provenance::Ingest {
                source: "fixture".to_string(),
            },
            valid_from: None,
            valid_to: None,
        });
    }
    graph.upsert_nodes(nodes).await.expect("hub nodes");
    graph.upsert_edges(edges).await.expect("hub edges");

    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    let response = call(
        &mut server,
        "tdw.kg.traverse",
        &json!({ "seeds": ["hub:center"], "max_hops": 1 }),
    );
    let text = response.to_string();
    assert!(
        text.contains("traverse budget exceeded"),
        "hub fan-out must be a budget tool error: {text}"
    );
}

// ── K-X3 Trust-dial integration tests (JSON-RPC surface) ────────────────────

/// Build a minimal runtime that indexes one `document_ingested` document
/// (an instrument) and one `user_authored` document (a finding), so the
/// provenance-class filter can be exercised end-to-end via the MCP surface.
fn server_with_trust_fixture() -> McpServer {
    let (runtime, _graph) = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let lexical = Arc::new(InMemoryLexicalEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());

        let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
        let mut indexer = KnowledgeIndexer::new(index)
            .with_lexical(lexical.clone(), LEXICAL_INDEX)
            .with_graph(Arc::new(SharedGraph(graph.clone())));

        // document_ingested: instrument entity
        indexer
            .index_at(
                KnowledgeDocument {
                    id: "trust-doc-ingested".to_string(),
                    body: "equity research note AAPL momentum".to_string(),
                    entity: Entity {
                        entity_id: "instrument:AAPL-trust".to_string(),
                        kind: EntityKind::Instrument,
                        label: "Apple trust fixture".to_string(),
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
            .unwrap_or_else(|e| panic!("index instrument doc: {e}"));

        // user_authored: finding entity (K-X6 kind)
        indexer
            .index_at(
                KnowledgeDocument {
                    id: "trust-doc-finding".to_string(),
                    body: "equity research note AAPL momentum personal finding".to_string(),
                    entity: Entity {
                        entity_id: "finding:trust-finding-001".to_string(),
                        kind: EntityKind::Finding,
                        label: "Trust finding fixture".to_string(),
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
            .unwrap_or_else(|e| panic!("index finding doc: {e}"));

        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_lexical(lexical, LEXICAL_INDEX)
            .with_graph(Arc::new(SharedGraph(graph.clone())));
        (Arc::new(runtime), graph)
    });
    let mut server = McpServer::new().with_knowledge(runtime);
    initialize(&mut server);
    server
}

#[test]
fn trust_dial_default_returns_all_classes_with_unfiltered_scope() {
    // No provenance_classes → all classes, trust_scope.filtered = false.
    let mut server = server_with_trust_fixture();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({ "query": "equity research AAPL momentum", "top_k": 8 }),
    );
    let result = &response["result"]["content"][0]["text"];
    let payload: serde_json::Value = serde_json::from_str(result.as_str().expect("text content"))
        .expect("structured result parses");

    // trust_scope must report unfiltered
    assert_eq!(
        payload["trust_scope"]["filtered"], false,
        "unfiltered search must report filtered=false: {payload}"
    );
    let hit_ids: Vec<&str> = payload["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert!(
        hit_ids.contains(&"trust-doc-ingested"),
        "instrument doc must appear without filter: {hit_ids:?}"
    );
    assert!(
        hit_ids.contains(&"trust-doc-finding"),
        "finding doc must appear without filter: {hit_ids:?}"
    );
}

#[test]
fn trust_dial_document_only_excludes_findings() {
    // provenance_classes = [document_ingested] → finding excluded.
    let mut server = server_with_trust_fixture();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({
            "query": "equity research AAPL momentum",
            "top_k": 8,
            "provenance_classes": ["document_ingested"],
        }),
    );
    let result = &response["result"]["content"][0]["text"];
    let payload: serde_json::Value = serde_json::from_str(result.as_str().expect("text content"))
        .expect("structured result parses");

    assert_eq!(
        payload["trust_scope"]["filtered"], true,
        "filtered search must report filtered=true: {payload}"
    );
    let classes_in_scope = &payload["trust_scope"]["provenance_classes"];
    assert!(
        classes_in_scope.to_string().contains("document_ingested"),
        "scope must name the active class: {classes_in_scope}"
    );
    let hit_ids: Vec<&str> = payload["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert!(
        hit_ids.contains(&"trust-doc-ingested"),
        "instrument doc must pass document_ingested filter: {hit_ids:?}"
    );
    assert!(
        !hit_ids.contains(&"trust-doc-finding"),
        "finding must be excluded by document_ingested filter: {hit_ids:?}"
    );
    // Every hit carries trust_class for explainability
    for hit in payload["hits"].as_array().expect("hits array") {
        assert_eq!(
            hit["trust_class"], "document_ingested",
            "hit must carry trust_class=document_ingested: {hit}"
        );
    }
}

#[test]
fn trust_dial_user_only_returns_only_findings() {
    // provenance_classes = [user_authored] → only finding.
    let mut server = server_with_trust_fixture();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({
            "query": "equity research AAPL momentum",
            "top_k": 8,
            "provenance_classes": ["user_authored"],
        }),
    );
    let result = &response["result"]["content"][0]["text"];
    let payload: serde_json::Value = serde_json::from_str(result.as_str().expect("text content"))
        .expect("structured result parses");

    let hit_ids: Vec<&str> = payload["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .filter_map(|h| h["id"].as_str())
        .collect();
    assert_eq!(
        hit_ids,
        vec!["trust-doc-finding"],
        "only the finding doc must appear under user_authored filter"
    );
    assert_eq!(
        payload["hits"][0]["trust_class"], "user_authored",
        "hit must carry trust_class=user_authored"
    );
}

#[test]
fn trust_dial_zero_hits_reports_honest_scope_not_error() {
    // provenance_classes = [rule_derived] → nothing in fixture → 0 hits, not error.
    let mut server = server_with_trust_fixture();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({
            "query": "equity research AAPL momentum",
            "top_k": 8,
            "provenance_classes": ["rule_derived"],
        }),
    );
    let result = &response["result"]["content"][0]["text"];
    let payload: serde_json::Value = serde_json::from_str(result.as_str().expect("text content"))
        .expect("structured result parses");

    assert_eq!(
        payload["hits"].as_array().expect("hits array").len(),
        0,
        "rule_derived filter must yield 0 hits"
    );
    assert_eq!(
        payload["trust_scope"]["filtered"], true,
        "zero-hit response must still report filtered=true: {payload}"
    );
    let note = payload["trust_scope"]["note"].as_str().unwrap_or_default();
    assert!(
        note.contains("0 hits at this trust level"),
        "zero-hit note must say '0 hits at this trust level': {note}"
    );
}

#[test]
fn trust_dial_unknown_class_is_tool_error() {
    // An unrecognized provenance_class token must be a tool error, not a panic.
    let mut server = server_with_trust_fixture();
    let response = call(
        &mut server,
        "tdw.kg.search",
        &json!({
            "query": "equity",
            "provenance_classes": ["not_a_real_class"],
        }),
    );
    let text = response.to_string();
    assert!(
        text.contains("unknown provenance_class") || text.contains("isError"),
        "unknown class must be a tool error: {text}"
    );
}

/// K-X3 fix #5: production-path assertion — `index_at → document_payload →
/// provenance_class_token` stamps the correct class on the stored vector point.
///
/// This test constructs a fresh `KnowledgeIndexer` (the real construction
/// path, not the shared fixture), indexes one `Instrument` doc and one
/// `Finding` doc, then searches via the `Retriever` and asserts each hit's
/// `trust_class` matches the production stamp. It exercises the full chain:
/// `index_at` → `document_payload` → `provenance_class_token` → vector upsert
/// → retrieval → `effective_trust_class` → `RetrievedHit::trust_class`.
#[tokio::test]
async fn index_at_stamps_provenance_class_on_production_path() {
    use tdw_retrieve::{KnowledgeQuery, QueryFilter, Retriever, TrustClass};

    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());

    // Use the real KnowledgeIndex + KnowledgeIndexer construction (production path).
    let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
    let mut indexer = KnowledgeIndexer::new(index);

    // Instrument → provenance_class_token returns "document_ingested"
    indexer
        .index_at(
            KnowledgeDocument {
                id: "prod-doc-instrument".to_string(),
                body: "production path instrument document ingested stamp".to_string(),
                entity: Entity {
                    entity_id: "instrument:PROD-INSTR".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Prod Instrument".to_string(),
                    aliases: Vec::new(),
                },
                tags: Vec::new(),
                source: None,
                plane: Some("platform".to_string()),
                as_of: Some(NOW.to_string()),
                mentions: Vec::new(),
            },
            NOW,
        )
        .await
        .expect("index instrument doc");

    // Finding → provenance_class_token returns "user_authored"
    indexer
        .index_at(
            KnowledgeDocument {
                id: "prod-doc-finding".to_string(),
                body: "production path finding user authored stamp".to_string(),
                entity: Entity {
                    entity_id: "finding:PROD-FINDING".to_string(),
                    kind: EntityKind::Finding,
                    label: "Prod Finding".to_string(),
                    aliases: Vec::new(),
                },
                tags: Vec::new(),
                source: None,
                plane: Some("platform".to_string()),
                as_of: Some(NOW.to_string()),
                mentions: Vec::new(),
            },
            NOW,
        )
        .await
        .expect("index finding doc");

    // Search via the real Retriever — same engines the indexer wrote to.
    // HashEmbeddingProvider::model_id() == "local-hash-8"; derive the
    // collection name via the same production helper `KnowledgeIndex::new` uses.
    let collection = collection_name("local-hash-8");
    let retriever = Retriever::new(embedder, vectors, &collection);
    let query = KnowledgeQuery::try_new("production path", 16, QueryFilter::default(), None)
        .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");

    let instrument_hit = hits
        .iter()
        .find(|h| h.id == "prod-doc-instrument")
        .expect("instrument doc must appear in results");
    assert_eq!(
        instrument_hit.trust_class,
        Some(TrustClass::DocumentIngested),
        "instrument doc must carry DocumentIngested from production stamp path"
    );

    let finding_hit = hits
        .iter()
        .find(|h| h.id == "prod-doc-finding")
        .expect("finding doc must appear in results");
    assert_eq!(
        finding_hit.trust_class,
        Some(TrustClass::UserAuthored),
        "finding doc must carry UserAuthored from production stamp path"
    );
}
