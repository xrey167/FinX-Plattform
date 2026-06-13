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

/// A hub root with far more neighbors than the handler's node budget, used to
/// demonstrate (and pin) the bounded fan-out truncation path.
pub const HUB_ROOT: &str = "instrument:HUB";

/// Neighbor count seeded around [`HUB_ROOT`] — comfortably past the handler's
/// `MAX_GRAPH_NODES` budget so a depth-1 fetch must truncate.
pub const HUB_NEIGHBORS: usize = 260;

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

/// Boot the backend and fetch the graph for a root that does NOT resolve.
///
/// Returns the parsed honest-empty payload. Distinct from [`fetch_empty_graph`]
/// (absent root): here a root *is* supplied but the graph has no such node, so
/// the handler returns `node_count = 0` with a "not found" note rather than
/// fabricating one.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_unknown_root_graph() -> std::io::Result<Value> {
    let server = start().await?;
    let response = client::get(
        server.addr(),
        "/widget-data/knowledge/graph?root=instrument:DOES_NOT_EXIST",
        "",
    )
    .await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Boot the backend and fetch an ego-graph with an OVERSIZED depth + `as_of`.
///
/// Returns the parsed payload. Exercises the depth clamp (`depth=99` is clamped
/// to the handler ceiling) and the leakage-safe `as_of` threading (echoed
/// verbatim into the structured block).
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_clamped_depth_graph() -> std::io::Result<Value> {
    let server = start().await?;
    let target =
        format!("/widget-data/knowledge/graph?root={ROOT}&depth=99&as_of=2024-01-01T00:00:00Z");
    let response = client::get(server.addr(), &target, "").await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Boot the backend and issue a graph request with a MALFORMED `depth`.
///
/// Returns the HTTP status. A non-numeric depth is the one caller mistake the
/// handler surfaces as `400` (an absent root is honest-empty, not an error).
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_invalid_depth_status() -> std::io::Result<u16> {
    let server = start().await?;
    let response = client::get(
        server.addr(),
        "/widget-data/knowledge/graph?root=x&depth=soon",
        "",
    )
    .await?;
    server.shutdown().await;
    Ok(response.status)
}

/// Boot the backend over a `KnowledgeRuntime` with NO graph engine attached.
///
/// Fetches a graph for a real-looking root and returns the parsed payload.
/// Proves the honest-empty path when the runtime simply has no graph plane: the
/// handler returns `node_count = 0` with an "unavailable" note, not an error.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_graph_without_engine() -> std::io::Result<Value> {
    let runtime = Arc::new(KnowledgeRuntime::new(
        Arc::new(HashEmbeddingProvider::default()),
        Arc::new(InMemoryVectorEngine::default()),
    ));
    let graph = KnowledgeGraphAdapter::with_clock(runtime, Arc::new(|| "@example".to_string()))
        .into_handler();
    let server = start_workspace_with_graph(WorkspaceConfig::default(), graph).await?;
    let target = format!("/widget-data/knowledge/graph?root={ROOT}");
    let response = client::get(server.addr(), &target, "").await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Seed a graph-backed runtime whose [`HUB_ROOT`] has [`HUB_NEIGHBORS`] direct
/// neighbors — past the handler's node budget — so a depth-1 ego-graph fetch
/// must truncate (bounded fan-out).
async fn hub_runtime() -> Arc<KnowledgeRuntime> {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let mut nodes = vec![GraphNode {
        id: HUB_ROOT.to_string(),
        kind: EntityKind::Instrument,
        label: "Hub".to_string(),
        aliases: Vec::new(),
        props: json!({}),
        valid_from: None,
        valid_to: None,
    }];
    let mut edges = Vec::new();
    for index in 0..HUB_NEIGHBORS {
        let id = format!("instrument:N{index}");
        nodes.push(GraphNode {
            id: id.clone(),
            kind: EntityKind::Instrument,
            label: format!("Node {index}"),
            aliases: Vec::new(),
            props: json!({}),
            valid_from: None,
            valid_to: None,
        });
        edges.push(GraphEdge {
            from: HUB_ROOT.to_string(),
            to: id,
            rel: "related_to".to_string(),
            props: json!({}),
            provenance: Provenance::Ingest {
                source: "example:hub".to_string(),
            },
            valid_from: None,
            valid_to: None,
        });
    }
    graph.upsert_nodes(nodes).await.expect("seed hub nodes");
    graph.upsert_edges(edges).await.expect("seed hub edges");

    Arc::new(
        KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
        .with_graph(graph as Arc<dyn GraphEngine>),
    )
}

/// Boot the backend over the [`hub_runtime`] and fetch the hub's ego-graph.
///
/// Returns the parsed depth-1 payload. The handler's hard fan-out budget must
/// clip the neighborhood and emit an honest truncation note.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_hub_graph() -> std::io::Result<Value> {
    let runtime = hub_runtime().await;
    let graph = KnowledgeGraphAdapter::with_clock(runtime, Arc::new(|| "@example".to_string()))
        .into_handler();
    let server = start_workspace_with_graph(WorkspaceConfig::default(), graph).await?;
    let target = format!("/widget-data/knowledge/graph?root={HUB_ROOT}&depth=1");
    let response = client::get(server.addr(), &target, "").await?;
    server.shutdown().await;
    Ok(response.json())
}
