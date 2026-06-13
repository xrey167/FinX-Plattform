//! Example 80 — the read-only knowledge graph-visualization widget (K-M6).
//!
//! Where examples 10/20 serve catalog data, this serves the *knowledge graph*: a
//! seeded in-memory graph-backed `KnowledgeRuntime` is wrapped in the same
//! `KnowledgeGraphAdapter` the live daemon uses, and the workspace backend
//! answers `GET /widget-data/knowledge/graph?root=...` with a bounded ego-graph
//! rendered as Markdown. Fully offline: the graph, vector engine, and embedder
//! are all in-memory.
//!
//! [`fetch_ego_graph`] seeds the runtime, boots the server, issues one request,
//! and returns the parsed payload so a `main()` can print it and a test can
//! assert on it. [`fetch_empty_graph`] proves the honest-empty path.

use std::sync::Arc;

use serde_json::{Value, json};
use tdw_app_server::WorkspaceConfig;
use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_service_api::KnowledgeGraphAdapter;
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::EntityKind;

use crate::client;
use crate::server::{RunningServer, start_workspace_with_graph};

/// The root entity the example anchors its ego-graph on.
pub const ROOT: &str = "instrument:AAPL";

/// Seed a graph-backed `KnowledgeRuntime`: `instrument:AAPL` listed on NASDAQ
/// and with Tim Cook as CEO — a three-node, two-edge ego-graph.
async fn seeded_runtime() -> Arc<KnowledgeRuntime> {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let node = |id: &str, kind: EntityKind, label: &str| GraphNode {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        aliases: Vec::new(),
        props: json!({}),
        valid_from: None,
        valid_to: None,
    };
    graph
        .upsert_nodes(vec![
            node(ROOT, EntityKind::Instrument, "Apple Inc."),
            node("exchange:NASDAQ", EntityKind::Venue, "NASDAQ"),
            node("person:tim_cook", EntityKind::Personality, "Tim Cook"),
        ])
        .await
        .expect("seed nodes");
    let edge = |from: &str, rel: &str, to: &str| GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: json!({}),
        provenance: Provenance::Ingest {
            source: "example:seed".to_string(),
        },
        valid_from: None,
        valid_to: None,
    };
    graph
        .upsert_edges(vec![
            edge(ROOT, "listed_on", "exchange:NASDAQ"),
            edge(ROOT, "has_ceo", "person:tim_cook"),
        ])
        .await
        .expect("seed edges");

    Arc::new(
        KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
        .with_graph(graph as Arc<dyn GraphEngine>),
    )
}

/// Boot the graph-backed workspace backend on an ephemeral port (deterministic
/// `generated_at` stamp via an injected clock).
///
/// # Errors
///
/// Returns any error binding the listener.
pub async fn start() -> std::io::Result<RunningServer> {
    let runtime = seeded_runtime().await;
    let graph = KnowledgeGraphAdapter::with_clock(runtime, Arc::new(|| "@example".to_string()))
        .into_handler();
    start_workspace_with_graph(WorkspaceConfig::default(), graph).await
}

/// Boot the backend and fetch the ego-graph for the seeded root, returning the
/// parsed payload (the `results` Markdown + the structured `graph` block).
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_ego_graph() -> std::io::Result<Value> {
    let server = start().await?;
    let target = format!("/widget-data/knowledge/graph?root={ROOT}&depth=1");
    let response = client::get(server.addr(), &target, "").await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Boot the backend and fetch the graph for an ABSENT root, returning the parsed
/// honest-empty payload (`node_count = 0`, no fabricated nodes).
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_empty_graph() -> std::io::Result<Value> {
    let server = start().await?;
    let response = client::get(server.addr(), "/widget-data/knowledge/graph", "").await?;
    server.shutdown().await;
    Ok(response.json())
}
