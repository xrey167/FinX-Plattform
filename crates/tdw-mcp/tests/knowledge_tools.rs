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
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};
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
    // Without a runtime, none of the five tools are listed.
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
    ] {
        assert!(!names.contains(&name.to_string()), "{name} must be absent");
    }

    // With a runtime, all five appear with read-only annotations.
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
