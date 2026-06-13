//! End-to-end tests for the read-only knowledge graph-visualization widget
//! (K-M6) served over the `OpenBB` Workspace bridge.
//!
//! These drive the real `tdw_app_server::serve_workspace_http_with_graph`
//! listener with a `RestApiState` (catalog fetch seam) PLUS a
//! `KnowledgeGraphAdapter` over a seeded in-memory graph-backed
//! `KnowledgeRuntime`, so `GET /widget-data/knowledge/graph` resolves a real
//! bounded ego-graph offline (no network, no credentials).
//!
//! Coverage:
//! - a seeded root serves a bounded ego-graph (the root + its neighbors),
//! - an absent / blank root yields an HONEST EMPTY graph (200, `node_count = 0`,
//!   no fabricated nodes) rather than an error,
//! - a non-existent root likewise yields an honest empty graph,
//! - `widgets.json` advertises the curated `knowledge_graph` widget.

#![cfg(feature = "workspace-route")]

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tdw_app_server::{CancellationToken, WorkspaceConfig, serve_workspace_http_with_graph};
use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance};
use tdw_embed_local::HashEmbeddingProvider;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_service_api::{AppState, KnowledgeGraphAdapter, RestApiState};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_taxonomy::EntityKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Build a graph-backed `KnowledgeRuntime` seeded with a small ego-graph:
/// `instrument:AAPL` `--listed_on-->` `exchange:NASDAQ`, and
/// `instrument:AAPL` `--has_ceo-->` `person:tim_cook`.
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
            node("instrument:AAPL", EntityKind::Instrument, "Apple Inc."),
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
            source: "test:seed".to_string(),
        },
        valid_from: None,
        valid_to: None,
    };
    graph
        .upsert_edges(vec![
            edge("instrument:AAPL", "listed_on", "exchange:NASDAQ"),
            edge("instrument:AAPL", "has_ceo", "person:tim_cook"),
        ])
        .await
        .expect("seed edges");

    let runtime = KnowledgeRuntime::new(
        Arc::new(HashEmbeddingProvider::default()),
        Arc::new(InMemoryVectorEngine::default()),
    )
    .with_graph(graph as Arc<dyn GraphEngine>);
    Arc::new(runtime)
}

/// Spin up the workspace listener with the K-M6 graph handler attached.
async fn with_graph_server<F, Fut>(runtime: Arc<KnowledgeRuntime>, body: F)
where
    F: FnOnce(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let state = AppState::in_memory_for_tests().await;
    let handler = RestApiState::new(state).into_handler();
    // Injected clock: deterministic generated_at stamp (no SystemTime::now).
    let graph =
        KnowledgeGraphAdapter::with_clock(runtime, Arc::new(|| "@test".to_string())).into_handler();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let server = tokio::spawn(async move {
        serve_workspace_http_with_graph(
            listener,
            handler,
            Some(graph),
            WorkspaceConfig::default(),
            cancel_srv,
        )
        .await
        .ok();
    });

    body(addr).await;

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
}

async fn raw_get(addr: std::net::SocketAddr, target: &str) -> Vec<u8> {
    let mut conn = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    conn.write_all(request.as_bytes()).await.expect("write");
    conn.flush().await.expect("flush");
    let mut resp = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), conn.read_to_end(&mut resp)).await;
    resp
}

fn status(resp: &[u8]) -> u16 {
    std::str::from_utf8(resp)
        .unwrap_or("")
        .split(' ')
        .nth(1)
        .unwrap_or("0")
        .parse()
        .unwrap_or(0)
}

fn body_json(resp: &[u8]) -> Value {
    let text = std::str::from_utf8(resp).unwrap_or("");
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    serde_json::from_str(body).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn seeded_root_serves_bounded_ego_graph() {
    let runtime = seeded_runtime().await;
    with_graph_server(runtime, |addr| async move {
        let resp = raw_get(
            addr,
            "/widget-data/knowledge/graph?root=instrument:AAPL&depth=1",
        )
        .await;
        assert_eq!(
            status(&resp),
            200,
            "resp: {}",
            String::from_utf8_lossy(&resp)
        );
        let body = body_json(&resp);
        let graph = &body["graph"];
        // Root + two neighbors = 3 nodes; two edges.
        assert_eq!(graph["node_count"], 3, "ego-graph: root + 2 neighbors");
        assert_eq!(graph["edge_count"], 2);
        // The Markdown payload (the markdown widget's dataKey) is non-empty and
        // references the root.
        let markdown = body["results"].as_str().expect("results markdown");
        assert!(
            markdown.contains("instrument:AAPL"),
            "markdown names the root"
        );
        assert!(markdown.contains("Knowledge Graph"));
        // Generated_at comes from the injected clock, not the system clock.
        assert_eq!(graph["generated_at"], "@test");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_root_yields_honest_empty_graph() {
    let runtime = seeded_runtime().await;
    with_graph_server(runtime, |addr| async move {
        let resp = raw_get(addr, "/widget-data/knowledge/graph").await;
        assert_eq!(status(&resp), 200, "honest-empty is 200, not an error");
        let graph = &body_json(&resp)["graph"];
        assert_eq!(graph["node_count"], 0);
        assert_eq!(graph["edge_count"], 0);
        assert!(
            graph["nodes"].as_array().expect("nodes array").is_empty(),
            "no fabricated nodes for an absent root"
        );
        assert!(
            graph["note"].as_str().unwrap_or("").contains("no root"),
            "honest note explains the empty graph"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_root_yields_honest_empty_graph() {
    let runtime = seeded_runtime().await;
    with_graph_server(runtime, |addr| async move {
        let resp = raw_get(
            addr,
            "/widget-data/knowledge/graph?root=instrument:DOES_NOT_EXIST",
        )
        .await;
        assert_eq!(status(&resp), 200);
        let graph = &body_json(&resp)["graph"];
        assert_eq!(
            graph["node_count"], 0,
            "no fabricated node for an unknown root"
        );
        assert!(
            graph["note"].as_str().unwrap_or("").contains("not found"),
            "honest note: root not found"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn widgets_json_advertises_the_graph_widget() {
    let runtime = seeded_runtime().await;
    with_graph_server(runtime, |addr| async move {
        let resp = raw_get(addr, "/widgets.json").await;
        assert_eq!(status(&resp), 200);
        let widget = &body_json(&resp)["knowledge_graph"];
        assert!(widget.is_object(), "knowledge_graph widget advertised");
        assert_eq!(widget["type"], "markdown");
        assert_eq!(widget["endpoint"], "/widget-data/knowledge/graph");
    })
    .await;
}
