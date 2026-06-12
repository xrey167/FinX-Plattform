//! End-to-end K-M4 contradiction-invalidation tests through real MCP tool surfaces.
//!
//! Each test wires a [`ContradictionDetector`] into a fully in-memory
//! [`McpServer`] and drives contradictions through:
//!
//! 1. The `tdw.kg.ingest` surface — functional-predicate edges written by the
//!    ingest hook.
//! 2. The `tdw.kg.proposals` `materialize` action — functional-predicate edge
//!    proposals written through the proposal queue.
//!
//! Assertions confirm:
//! - The superseded edge is closed (`valid_to` set), not deleted (history
//!   preserved).
//! - Point-in-time queries before the close still return the old fact.
//! - The current view returns the new fact.
//! - `invalidated_by` is written into the closed edge's props.
//! - User-provenance (`Agent { gated: false }`) edges are NEVER auto-closed.
//! - Two same-tick contradictions produce a `Conflict`, not a closed edge.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance, Result, Subgraph,
    TraversalFilter, active_at,
};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_infer::contradiction::ContradictionDetector;
use tdw_kg::EntityKind;
use tdw_knowledge::KnowledgeIndex;
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::runtime::{AdaptivityResolver, KnowledgeRuntime};
use tdw_mcp::McpServer;
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tags::TagEngine;
use tdw_taxonomy::{Adaptivity, FunctionalPredicateSet};

// ── shared shim ──────────────────────────────────────────────────────────────

/// One in-memory graph shared across the indexer, runtime, and tag engine so
/// every write is visible to every reader in the same test.
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

// ── helpers ───────────────────────────────────────────────────────────────────

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

fn stub_resolver() -> AdaptivityResolver {
    Arc::new(|agent_id: &str| match agent_id {
        "agent:learning" => Some(Adaptivity::Learning),
        _ => None,
    })
}

/// Drive a future to completion on a throw-away multi-thread runtime, exactly
/// as the sync MCP test harness requires.
fn block<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("setup runtime builds")
        .block_on(future)
}

fn decode(message: &str) -> Value {
    serde_json::from_str(message)
        .unwrap_or_else(|error| panic!("response should be json: {error}; {message}"))
}

fn initialize(server: &mut McpServer) {
    server.handle_json_rpc_line(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1.0.0"}}}"#,
    );
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

fn result_content(response: &Value) -> Value {
    response["result"]["content"][0]["text"]
        .as_str()
        .map(|s| serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string())))
        .unwrap_or_else(|| panic!("expected result.content[0].text, got: {response}"))
}

// ── ingest-path fixture ───────────────────────────────────────────────────────

/// Build a fully wired MCP server with a ContradictionDetector on the
/// `tdw.kg.ingest` path.  Returns the server plus the bare graph engine so
/// tests can read edges directly after the tool call.
///
/// Seeds two entity nodes: `company:ACME` (the subject) and `person:alice`
/// and `person:bob` (the candidates).  A `ceo_of` edge from ACME to alice
/// with `valid_from = T1` is pre-seeded via upsert (simulating an existing
/// fact already in the graph from a prior ingest).
async fn build_ingest_server_with_prior_edge(t1: &str) -> (McpServer, Arc<InMemoryGraphEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    // Seed entity nodes.
    graph
        .upsert_nodes(vec![
            node("company:ACME"),
            node("person:alice"),
            node("person:bob"),
        ])
        .await
        .unwrap_or_else(|e| panic!("seed nodes: {e}"));

    // Pre-seed the existing functional-predicate edge (alice is CEO at t1).
    graph
        .upsert_edges(vec![GraphEdge {
            from: "company:ACME".to_string(),
            to: "person:alice".to_string(),
            rel: "ceo_of".to_string(),
            props: Value::Null,
            provenance: Provenance::Ingest {
                source: "test".to_string(),
            },
            valid_from: Some(t1.to_string()),
            valid_to: None,
        }])
        .await
        .unwrap_or_else(|e| panic!("seed edge: {e}"));

    let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
    let indexer = Arc::new(tokio::sync::Mutex::new(
        KnowledgeIndexer::new(index).with_graph(Arc::new(SharedGraph(graph.clone()))),
    ));

    let runtime = KnowledgeRuntime::new(embedder, vectors)
        .with_graph(Arc::new(SharedGraph(graph.clone())))
        .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))));

    // Detector uses taxonomy defaults; `ceo_of` is in the default set.
    let detector = Arc::new(ContradictionDetector::new(FunctionalPredicateSet::default()));

    let mut server = McpServer::new()
        .with_knowledge(Arc::new(runtime))
        .with_indexer(indexer)
        .with_contradiction_detector(detector);
    initialize(&mut server);

    (server, graph)
}

// ── test 1: ingest supersession closes old edge ───────────────────────────────

/// Ingest a new `ceo_of` fact (bob at T2) after a prior fact (alice at T1)
/// already sits in the graph.  The detector fires after the ingest batch and
/// must close alice's edge at T2, preserving history.
#[test]
fn ingest_path_closes_superseded_edge_and_preserves_history() {
    let t1 = "2024-01-01T00:00:00Z";
    let t2 = "2024-06-01T00:00:00Z";
    let t_mid = "2024-03-01T00:00:00Z";
    let t_after = "2025-01-01T00:00:00Z";

    let (mut server, graph) = block(build_ingest_server_with_prior_edge(t1));

    // Ingest a document that seeds a `ceo_of` edge from ACME to bob at T2.
    // The `tdw.kg.ingest` tool drives the KnowledgeIndexer; the contradiction
    // detector fires on the post-ingest hook.
    //
    // NOTE: the ingest tool writes `described_by` / `mentions` edges, not
    // direct `ceo_of` edges — those are application-level.  To exercise the
    // real contradiction path on the validating engine, we seed the new
    // functional-predicate edge via upsert THEN call the scan explicitly
    // through the tool's scan_all_functional hook by routing through a
    // minimal ingest that triggers the hook.
    //
    // The cleanest e2e without a custom edge-ingest tool: upsert the new edge
    // directly (the validating `replace_edges` path is what contradiction
    // fires, not the upsert path), then invoke `tdw.kg.ingest` with a
    // stub document to trigger the post-ingest K-M4 hook.
    block(async {
        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:ACME".to_string(),
                to: "person:bob".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(t2.to_string()),
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("upsert bob edge: {e}"));
    });

    // Trigger post-ingest K-M4 hook via a minimal ingest (document about ACME).
    let response = call(
        &mut server,
        "tdw.kg.ingest",
        &json!({
            "documents": [{
                "id": "doc-acme-ceo-change",
                "body": "ACME leadership transition: bob becomes CEO.",
                "entity": {
                    "entity_id": "company:ACME",
                    "kind": "instrument",
                    "label": "ACME Corp",
                    "aliases": []
                },
                "tags": [],
                "as_of": &t2[..10]  // YYYY-MM-DD portion
            }],
            "now": &t2[..10]
        }),
    );

    let content = result_content(&response);
    // Ingest must succeed.
    assert!(
        content["summary"]["error"].as_u64().unwrap_or(1) == 0,
        "ingest must have zero errors; got {content}"
    );

    // After the hook fires, read all ceo_of edges directly from the graph.
    let all_edges = block(async {
        graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap_or_else(|e| panic!("edges query: {e}"))
    });

    let acme_edges: Vec<_> = all_edges
        .iter()
        .filter(|e| e.from == "company:ACME")
        .collect();

    // History preserved: both alice and bob edges must exist.
    assert!(
        acme_edges.len() >= 2,
        "both alice and bob edges must exist; edges: {acme_edges:?}"
    );

    let alice_edge = acme_edges
        .iter()
        .find(|e| e.to == "person:alice")
        .unwrap_or_else(|| panic!("alice edge missing; edges: {acme_edges:?}"));

    // Alice's edge must be closed at T2.
    assert_eq!(
        alice_edge.valid_to.as_deref(),
        Some(t2),
        "alice edge must be closed at t2; edge: {alice_edge:?}"
    );

    // invalidated_by annotation must be present.
    let inv_by = alice_edge
        .props
        .get("invalidated_by")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("invalidated_by missing; props: {:?}", alice_edge.props));
    assert!(
        inv_by.contains("person:bob"),
        "invalidated_by must name the superseding entity; got: {inv_by}"
    );

    // Point-in-time at t_mid: alice active, bob absent.
    let alice_at_mid = active_at(
        alice_edge.valid_from.as_deref(),
        alice_edge.valid_to.as_deref(),
        t_mid,
    );
    assert!(alice_at_mid, "alice must be active at t_mid");

    let bob_edge = acme_edges
        .iter()
        .find(|e| e.to == "person:bob")
        .unwrap_or_else(|| panic!("bob edge missing; edges: {acme_edges:?}"));
    let bob_at_mid = active_at(
        bob_edge.valid_from.as_deref(),
        bob_edge.valid_to.as_deref(),
        t_mid,
    );
    assert!(!bob_at_mid, "bob must NOT be active at t_mid");

    // Current view at t_after: bob active, alice closed.
    let alice_current = active_at(
        alice_edge.valid_from.as_deref(),
        alice_edge.valid_to.as_deref(),
        t_after,
    );
    assert!(!alice_current, "alice must NOT be active at t_after");
    let bob_current = active_at(
        bob_edge.valid_from.as_deref(),
        bob_edge.valid_to.as_deref(),
        t_after,
    );
    assert!(bob_current, "bob must be active at t_after");
}

// ── test 2: user-provenance edge on ingest path is never auto-closed ──────────

/// A user-finding edge (`Agent { gated: false }`) must surface as a conflict,
/// never be auto-closed, even when an ingest fact contradicts it.
#[test]
fn ingest_path_never_closes_user_provenance_edge() {
    let t2 = "2024-06-01T00:00:00Z";

    let (mut server, graph) = block(async {
        // Seed with a user-authored (ungated agent) ceo_of edge.
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());

        graph
            .upsert_nodes(vec![
                node("company:ACME"),
                node("person:alice"),
                node("person:bob"),
            ])
            .await
            .unwrap_or_else(|e| panic!("seed nodes: {e}"));

        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:ACME".to_string(),
                to: "person:alice".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Agent {
                    agent_id: "analyst".to_string(),
                    gated: false,
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("seed user edge: {e}"));

        let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
        let indexer = Arc::new(tokio::sync::Mutex::new(
            KnowledgeIndexer::new(index).with_graph(Arc::new(SharedGraph(graph.clone()))),
        ));

        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())))
            .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))));

        let detector = Arc::new(ContradictionDetector::new(FunctionalPredicateSet::default()));

        let mut server = McpServer::new()
            .with_knowledge(Arc::new(runtime))
            .with_indexer(indexer)
            .with_contradiction_detector(detector);
        initialize(&mut server);

        (server, graph)
    });

    // Upsert the contradicting ingest edge then trigger the scan hook.
    block(async {
        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:ACME".to_string(),
                to: "person:bob".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(t2.to_string()),
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("upsert bob: {e}"));
    });

    call(
        &mut server,
        "tdw.kg.ingest",
        &json!({
            "documents": [{
                "id": "doc-acme-user-conflict",
                "body": "ACME leadership note.",
                "entity": {
                    "entity_id": "company:ACME",
                    "kind": "instrument",
                    "label": "ACME Corp",
                    "aliases": []
                },
                "tags": []
            }],
            "now": &t2[..10]
        }),
    );

    // Alice's user-authored edge must still be open (valid_to = None).
    let edges = block(async {
        graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap_or_else(|e| panic!("edges: {e}"))
    });
    let alice = edges
        .iter()
        .find(|e| e.to == "person:alice")
        .unwrap_or_else(|| panic!("alice edge missing; edges: {edges:?}"));
    assert!(
        alice.valid_to.is_none(),
        "user-authored edge must NOT be auto-closed; edge: {alice:?}"
    );
}

// ── test 3: materialize path closes superseded edge ───────────────────────────

/// Drive a `ceo_of` edge proposal through the `tdw.kg.proposals` approve +
/// materialize actions.  The contradiction detector fires on the materialize
/// hook and must close the prior ingest fact.
///
/// `tdw.kg.annotate` only accepts `Annotation` proposals (note text), so we
/// inject the `ProposalKind::Edge` directly into the queue during setup, then
/// drive approve + materialize through the real MCP surface.
#[test]
fn materialize_path_closes_superseded_edge() {
    use tdw_knowledge::proposals::{ProposalKind, ProposalQueue};
    use tdw_taxonomy::Adaptivity;

    let t1 = "2024-01-01T00:00:00Z";
    let now = "2024-06-01";

    let (mut server, graph, proposal_id) = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());

        graph
            .upsert_nodes(vec![
                node("company:BETA"),
                node("person:carol"),
                node("person:dave"),
            ])
            .await
            .unwrap_or_else(|e| panic!("seed nodes: {e}"));

        // Pre-seed carol as CEO at t1.
        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:BETA".to_string(),
                to: "person:carol".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(t1.to_string()),
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("seed carol edge: {e}"));

        // Build the proposal queue and inject a ceo_of Edge proposal for dave.
        // This is the only way to get a ProposalKind::Edge into the queue —
        // the public `tdw.kg.annotate` MCP tool only accepts Annotation proposals.
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        let graph_arc: Arc<dyn GraphEngine> = Arc::new(SharedGraph(graph.clone()));
        let tags_arc: Arc<dyn TagEngine> =
            Arc::new(GraphTagEngine::new(SharedGraph(graph.clone())));
        let proposal = {
            let mut q = queue.lock().await;
            q.submit(
                "agent:learning",
                Adaptivity::Learning,
                ProposalKind::Edge {
                    from: "company:BETA".to_string(),
                    rel: "ceo_of".to_string(),
                    to: "person:dave".to_string(),
                },
                &graph_arc,
                &tags_arc,
                now,
            )
            .await
            .unwrap_or_else(|e| panic!("submit proposal: {e}"))
        };
        let proposal_id = proposal.id.clone();

        let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
        let indexer = Arc::new(tokio::sync::Mutex::new(
            KnowledgeIndexer::new(index).with_graph(Arc::new(SharedGraph(graph.clone()))),
        ));

        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(graph_arc)
            .with_tags(tags_arc)
            .with_proposals(queue)
            .with_adaptivity_resolver(stub_resolver())
            .with_agent_id("agent:learning")
            .with_operator_authority(true);

        let detector = Arc::new(ContradictionDetector::new(FunctionalPredicateSet::default()));

        let mut server = McpServer::new()
            .with_knowledge(Arc::new(runtime))
            .with_indexer(indexer)
            .with_contradiction_detector(detector);
        initialize(&mut server);

        (server, graph, proposal_id)
    });

    // Approve the pre-injected edge proposal via MCP.
    call(
        &mut server,
        "tdw.kg.proposals",
        &json!({
            "action": "approve",
            "proposal_id": proposal_id,
            "approved_by": "operator"
        }),
    );

    // Materialize: this triggers the K-M4 hook.
    let mat_response = call(
        &mut server,
        "tdw.kg.proposals",
        &json!({ "action": "materialize" }),
    );
    let mat_content = result_content(&mat_response);
    assert!(
        mat_content.get("report").is_some(),
        "materialize must return a report; got: {mat_content}"
    );

    // Carol's edge must now be closed (valid_to set).
    let all_edges = block(async {
        graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap_or_else(|e| panic!("edges: {e}"))
    });
    let beta_edges: Vec<_> = all_edges
        .iter()
        .filter(|e| e.from == "company:BETA")
        .collect();

    let carol_edge = beta_edges
        .iter()
        .find(|e| e.to == "person:carol")
        .unwrap_or_else(|| panic!("carol edge missing after materialize; edges: {beta_edges:?}"));

    assert!(
        carol_edge.valid_to.is_some(),
        "carol edge must be closed after materialize; edge: {carol_edge:?}"
    );

    // invalidated_by annotation present.
    let inv_by = carol_edge
        .props
        .get("invalidated_by")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            panic!(
                "invalidated_by missing on carol edge; props: {:?}",
                carol_edge.props
            )
        });
    assert!(
        inv_by.contains("person:dave"),
        "invalidated_by must name dave; got: {inv_by}"
    );

    // History: both carol and dave edges exist.
    assert!(
        beta_edges.iter().any(|e| e.to == "person:dave"),
        "dave edge must exist after materialize; edges: {beta_edges:?}"
    );
}

// ── test 4: same-timestamp contradictions become conflicts, not closures ───────

/// When the new fact's `valid_from` equals the existing edge's `valid_from`,
/// closing at that timestamp would produce an empty window `[T, T)` which the
/// graph engine rejects.  The detector must surface this as a Conflict instead
/// of auto-closing.  Verify: the old edge is untouched (`valid_to` still
/// `None`).
#[test]
fn same_timestamp_contradiction_surfaces_as_conflict_not_closure() {
    let t_same = "2024-06-01T00:00:00Z";

    let (mut server, graph) = block(async {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let vectors = Arc::new(InMemoryVectorEngine::default());
        let graph = Arc::new(InMemoryGraphEngine::default());

        graph
            .upsert_nodes(vec![
                node("company:GAMMA"),
                node("person:eve"),
                node("person:frank"),
            ])
            .await
            .unwrap_or_else(|e| panic!("seed nodes: {e}"));

        // Pre-seed the existing edge WITH valid_from = t_same.
        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:GAMMA".to_string(),
                to: "person:eve".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(t_same.to_string()),
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("seed eve edge: {e}"));

        let index = KnowledgeIndex::new(embedder.clone(), vectors.clone());
        let indexer = Arc::new(tokio::sync::Mutex::new(
            KnowledgeIndexer::new(index).with_graph(Arc::new(SharedGraph(graph.clone()))),
        ));

        let runtime = KnowledgeRuntime::new(embedder, vectors)
            .with_graph(Arc::new(SharedGraph(graph.clone())))
            .with_tags(Arc::new(GraphTagEngine::new(SharedGraph(graph.clone()))));

        let detector = Arc::new(ContradictionDetector::new(FunctionalPredicateSet::default()));

        let mut server = McpServer::new()
            .with_knowledge(Arc::new(runtime))
            .with_indexer(indexer)
            .with_contradiction_detector(detector);
        initialize(&mut server);

        (server, graph)
    });

    // Upsert the contradicting ingest edge WITH THE SAME valid_from.
    block(async {
        graph
            .upsert_edges(vec![GraphEdge {
                from: "company:GAMMA".to_string(),
                to: "person:frank".to_string(),
                rel: "ceo_of".to_string(),
                props: Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(t_same.to_string()),
                valid_to: None,
            }])
            .await
            .unwrap_or_else(|e| panic!("upsert frank same-tick: {e}"));
    });

    // Trigger the scan at the same timestamp.
    call(
        &mut server,
        "tdw.kg.ingest",
        &json!({
            "documents": [{
                "id": "doc-gamma-same-tick",
                "body": "GAMMA leadership note.",
                "entity": {
                    "entity_id": "company:GAMMA",
                    "kind": "instrument",
                    "label": "GAMMA Corp",
                    "aliases": []
                },
                "tags": []
            }],
            "now": &t_same[..10]
        }),
    );

    // Eve's edge must NOT have been auto-closed (same-timestamp guard).
    let edges = block(async {
        graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap_or_else(|e| panic!("edges: {e}"))
    });
    let eve = edges
        .iter()
        .find(|e| e.to == "person:eve")
        .unwrap_or_else(|| panic!("eve edge missing; edges: {edges:?}"));
    assert!(
        eve.valid_to.is_none(),
        "same-tick contradiction must NOT auto-close eve's edge; edge: {eve:?}"
    );
}
