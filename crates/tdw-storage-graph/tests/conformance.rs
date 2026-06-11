//! Cross-backend behavioral conformance for the graph engine (knowledge-system A2).
//!
//! ONE suite of assertions runs verbatim against every [`GraphEngine`] backend so they
//! cannot silently drift: upsert/read round-trips, directional + rel/kind/as_of-filtered
//! neighborhoods, hop-bounded expansion, shortest paths, and real merge semantics
//! (alias union, edge rewiring, tombstone, audit edge) behave identically wherever the
//! graph lives.
//!
//! Slice A2 wires the in-memory leg; the Bolt leg (Memgraph/Neo4j, slice A4) joins as a
//! second `AnyGraph` variant behind a feature + env gate, mirroring
//! `tdw-outbox/tests/conformance.rs`.

#![forbid(unsafe_code)]

use serde_json::json;
use tdw_core::{
    Direction, GraphEdge, GraphEngine, GraphNode, MergeDecision, Provenance, TraversalFilter,
};
use tdw_storage_graph::InMemoryGraphEngine;

enum AnyGraph {
    Mem(InMemoryGraphEngine),
    #[cfg(feature = "bolt")]
    Bolt(tdw_storage_graph::BoltGraphEngine),
}

impl AnyGraph {
    fn engine(&self) -> &dyn GraphEngine {
        match self {
            Self::Mem(engine) => engine,
            #[cfg(feature = "bolt")]
            Self::Bolt(engine) => engine,
        }
    }
}

fn node(id: &str, kind: tdw_taxonomy::EntityKind, label: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        aliases: Vec::new(),
        props: json!({}),
        valid_from: None,
        valid_to: None,
    }
}

fn edge(from: &str, rel: &str, to: &str) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: json!({}),
        provenance: Provenance::Ingest {
            source: "conformance".to_string(),
        },
        valid_from: None,
        valid_to: None,
    }
}

fn filter(direction: Direction, max_hops: u8) -> TraversalFilter {
    TraversalFilter {
        direction,
        max_hops,
        ..TraversalFilter::default()
    }
}

/// The behavioral contract, asserted identically for every backend.
#[allow(clippy::too_many_lines)]
async fn conformance_suite(any: &AnyGraph) {
    use tdw_taxonomy::EntityKind::{Account, Dataset, Instrument, Venue};
    let g = any.engine();

    // 1. Upsert + read round-trip; node upsert replaces by id.
    g.upsert_nodes(vec![
        node("instrument:AAPL", Instrument, "Apple"),
        node("instrument:MSFT", Instrument, "Microsoft"),
        node("venue:XNAS", Venue, "Nasdaq"),
        node("dataset:ohlcv", Dataset, "OHLCV"),
        node("account:alpha", Account, "Alpha Book"),
    ])
    .await
    .unwrap_or_else(|error| panic!("nodes must upsert: {error}"));
    g.upsert_nodes(vec![node("instrument:AAPL", Instrument, "Apple Inc.")])
        .await
        .unwrap_or_else(|error| panic!("re-upsert must succeed: {error}"));
    let apple = g
        .node("instrument:AAPL")
        .await
        .unwrap_or_else(|error| panic!("node read must succeed: {error}"))
        .unwrap_or_else(|| panic!("node must exist"));
    assert_eq!(apple.label, "Apple Inc.");
    assert!(
        g.node("instrument:NOPE")
            .await
            .unwrap_or_else(|error| panic!("missing read must succeed: {error}"))
            .is_none()
    );

    // 2. Edge upsert requires endpoints; identity (from,to,rel,valid_from) replaces.
    assert!(
        g.upsert_edges(vec![edge("instrument:AAPL", "listed_on", "venue:GHOST")])
            .await
            .is_err(),
        "missing endpoint must be rejected"
    );
    g.upsert_edges(vec![
        edge("instrument:AAPL", "listed_on", "venue:XNAS"),
        edge("instrument:MSFT", "listed_on", "venue:XNAS"),
        edge("instrument:AAPL", "has_prices", "dataset:ohlcv"),
        edge("account:alpha", "holds", "instrument:AAPL"),
    ])
    .await
    .unwrap_or_else(|error| panic!("edges must upsert: {error}"));
    let mut replacement = edge("instrument:AAPL", "listed_on", "venue:XNAS");
    replacement.props = json!({ "primary": true });
    g.upsert_edges(vec![replacement])
        .await
        .unwrap_or_else(|error| panic!("edge replacement must succeed: {error}"));

    // 2b. Edge scan (B7): rel-filtered and unfiltered enumeration in the
    // contract (from, rel, to, valid_from) order; stable pagination; zero
    // limit rejected.
    let listed = g
        .edges(Some("listed_on"), 0, 10)
        .await
        .unwrap_or_else(|error| panic!("edge scan must succeed: {error}"));
    assert_eq!(listed.len(), 2);
    assert_eq!(
        (listed[0].from.as_str(), listed[1].from.as_str()),
        ("instrument:AAPL", "instrument:MSFT")
    );
    assert_eq!(
        listed[0].props,
        json!({ "primary": true }),
        "scan returns the replaced edge's props"
    );
    let all_edges = g
        .edges(None, 0, 100)
        .await
        .unwrap_or_else(|error| panic!("unfiltered scan must succeed: {error}"));
    assert_eq!(all_edges.len(), 4);
    let mut paged = Vec::new();
    let mut offset = 0;
    loop {
        let page = g
            .edges(None, offset, 3)
            .await
            .unwrap_or_else(|error| panic!("paged scan must succeed: {error}"));
        if page.is_empty() {
            break;
        }
        offset += page.len();
        paged.extend(page);
    }
    assert_eq!(
        paged, all_edges,
        "pagination covers every edge exactly once"
    );
    assert!(
        g.edges(None, 0, 0).await.is_err(),
        "zero limit must be rejected"
    );
    assert!(
        g.edges(Some("no_such_rel"), 0, 5)
            .await
            .unwrap_or_else(|error| panic!("empty-rel scan must succeed: {error}"))
            .is_empty()
    );

    // 3. Directional neighborhoods, deterministic order.
    // BOLT-A4: the (rel, neighbor id, valid_from) order asserted below is part of
    // the GraphEngine contract — every MATCH feeding neighbors() MUST carry an
    // explicit ORDER BY; omitting it yields planner-dependent order that fails
    // these assertions non-deterministically.
    let out = g
        .neighbors("instrument:AAPL", &filter(Direction::Out, 1))
        .await
        .unwrap_or_else(|error| panic!("out neighbors must succeed: {error}"));
    assert_eq!(
        out.iter()
            .map(|(e, n)| (e.rel.as_str(), n.id.as_str()))
            .collect::<Vec<_>>(),
        vec![("has_prices", "dataset:ohlcv"), ("listed_on", "venue:XNAS"),]
    );
    assert_eq!(
        out.iter()
            .find(|(e, _)| e.rel == "listed_on")
            .map(|(e, _)| e.props.clone()),
        Some(json!({ "primary": true })),
        "edge replacement must have taken effect"
    );
    let inbound = g
        .neighbors("instrument:AAPL", &filter(Direction::In, 1))
        .await
        .unwrap_or_else(|error| panic!("in neighbors must succeed: {error}"));
    assert_eq!(
        inbound
            .iter()
            .map(|(e, n)| (e.rel.as_str(), n.id.as_str()))
            .collect::<Vec<_>>(),
        vec![("holds", "account:alpha")]
    );
    let both = g
        .neighbors("instrument:AAPL", &filter(Direction::Both, 1))
        .await
        .unwrap_or_else(|error| panic!("both neighbors must succeed: {error}"));
    assert_eq!(both.len(), 3);

    // 4. Rel and kind filters.
    let only_listed = g
        .neighbors(
            "instrument:AAPL",
            &TraversalFilter {
                rels: Some(vec!["listed_on".to_string()]),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("rel-filtered neighbors must succeed: {error}"));
    assert_eq!(only_listed.len(), 1);
    let only_datasets = g
        .neighbors(
            "instrument:AAPL",
            &TraversalFilter {
                kinds: Some(vec![Dataset]),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("kind-filtered neighbors must succeed: {error}"));
    assert_eq!(only_datasets[0].1.id, "dataset:ohlcv");

    // 5. Temporal as_of: a closed-window edge is visible inside its window, invisible
    //    after it, and unfiltered reads (as_of: None) still see it.
    let mut dated = edge("instrument:MSFT", "constituent_of", "dataset:ohlcv");
    dated.valid_from = Some("2024-01-01T00:00:00Z".to_string());
    dated.valid_to = Some("2025-01-01T00:00:00Z".to_string());
    g.upsert_edges(vec![dated])
        .await
        .unwrap_or_else(|error| panic!("dated edge must upsert: {error}"));
    let at = |as_of: &str| TraversalFilter {
        as_of: Some(as_of.to_string()),
        rels: Some(vec!["constituent_of".to_string()]),
        ..filter(Direction::Out, 1)
    };
    let inside = g
        .neighbors("instrument:MSFT", &at("2024-06-01T00:00:00Z"))
        .await
        .unwrap_or_else(|error| panic!("inside-window read must succeed: {error}"));
    assert_eq!(inside.len(), 1);
    let after = g
        .neighbors("instrument:MSFT", &at("2025-06-01T00:00:00Z"))
        .await
        .unwrap_or_else(|error| panic!("after-window read must succeed: {error}"));
    assert!(after.is_empty());
    let unfiltered = g
        .neighbors(
            "instrument:MSFT",
            &TraversalFilter {
                rels: Some(vec!["constituent_of".to_string()]),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("unfiltered read must succeed: {error}"));
    assert_eq!(unfiltered.len(), 1);

    // 6. Hop-bounded expansion: 1 hop from the account reaches the instrument; 2 hops
    //    also reach the venue + dataset through it.
    let one_hop = g
        .expand(&["account:alpha".to_string()], &filter(Direction::Out, 1))
        .await
        .unwrap_or_else(|error| panic!("1-hop expand must succeed: {error}"));
    assert_eq!(
        one_hop
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec!["account:alpha", "instrument:AAPL"]
    );
    let two_hops = g
        .expand(&["account:alpha".to_string()], &filter(Direction::Out, 2))
        .await
        .unwrap_or_else(|error| panic!("2-hop expand must succeed: {error}"));
    assert_eq!(
        two_hops
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "account:alpha",
            "dataset:ohlcv",
            "instrument:AAPL",
            "venue:XNAS"
        ]
    );

    // 7. Shortest path respects direction and hop budget.
    let path = g
        .shortest_path("account:alpha", "venue:XNAS", &filter(Direction::Out, 3))
        .await
        .unwrap_or_else(|error| panic!("path query must succeed: {error}"))
        .unwrap_or_else(|| panic!("path must exist"));
    assert_eq!(
        path.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
        vec!["holds", "listed_on"]
    );
    assert!(
        g.shortest_path("account:alpha", "venue:XNAS", &filter(Direction::Out, 1))
            .await
            .unwrap_or_else(|error| panic!("budgeted path query must succeed: {error}"))
            .is_none(),
        "hop budget below path length must yield None"
    );
    assert!(
        g.shortest_path("venue:XNAS", "account:alpha", &filter(Direction::Out, 3))
            .await
            .unwrap_or_else(|error| panic!("reverse path query must succeed: {error}"))
            .is_none(),
        "no outgoing path from venue back to account"
    );

    // 8. Real merge semantics: alias union, edge rewiring, tombstone, audit edge.
    g.upsert_nodes(vec![GraphNode {
        aliases: vec!["APPLE".to_string()],
        ..node("instrument:APPLE-DUP", Instrument, "Apple (duplicate)")
    }])
    .await
    .unwrap_or_else(|error| panic!("duplicate node must upsert: {error}"));
    g.upsert_edges(vec![edge("account:alpha", "holds", "instrument:APPLE-DUP")])
        .await
        .unwrap_or_else(|error| panic!("duplicate edge must upsert: {error}"));
    assert!(
        g.merge_entities(
            "instrument:APPLE-DUP",
            "instrument:APPLE-DUP",
            &MergeDecision {
                approved_by: "architect".to_string(),
            }
        )
        .await
        .is_err(),
        "self-merge must be rejected"
    );
    let report = g
        .merge_entities(
            "instrument:APPLE-DUP",
            "instrument:AAPL",
            &MergeDecision {
                approved_by: "architect".to_string(),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("merge must succeed: {error}"));
    assert_eq!(
        report.rewired_edges, 0,
        "rewired holds-edge duplicates the surviving one"
    );
    assert_eq!(report.absorbed_aliases, 2, "alias + label absorbed");
    let merged_target = g
        .node("instrument:AAPL")
        .await
        .unwrap_or_else(|error| panic!("target read must succeed: {error}"))
        .unwrap_or_else(|| panic!("target must exist"));
    assert!(merged_target.aliases.contains(&"APPLE".to_string()));
    assert!(
        merged_target
            .aliases
            .contains(&"Apple (duplicate)".to_string())
    );
    let tombstone = g
        .node("instrument:APPLE-DUP")
        .await
        .unwrap_or_else(|error| panic!("tombstone read must succeed: {error}"))
        .unwrap_or_else(|| panic!("tombstone must remain addressable"));
    assert_eq!(tombstone.props["merged_into"], "instrument:AAPL");
    let audit = g
        .neighbors(
            "instrument:APPLE-DUP",
            &TraversalFilter {
                rels: Some(vec!["merged_into".to_string()]),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("audit read must succeed: {error}"));
    assert_eq!(audit.len(), 1);
    assert_eq!(
        audit[0].0.provenance,
        Provenance::Manual {
            approved_by: "architect".to_string(),
        }
    );
    // The duplicate's holds-edge was dropped as a duplicate of the surviving one —
    // the account's outgoing holds neighborhood is exactly the target, once.
    let holds = g
        .neighbors(
            "account:alpha",
            &TraversalFilter {
                rels: Some(vec!["holds".to_string()]),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("holds read must succeed: {error}"));
    assert_eq!(
        holds.iter().map(|(_, n)| n.id.as_str()).collect::<Vec<_>>(),
        vec!["instrument:AAPL"]
    );

    // 9. A merge is one-shot: repeating it on a tombstoned source must error
    //    rather than silently duplicate the audit edge.
    assert!(
        g.merge_entities(
            "instrument:APPLE-DUP",
            "instrument:AAPL",
            &MergeDecision {
                approved_by: "architect".to_string(),
            },
        )
        .await
        .is_err(),
        "merging an already-merged source must be rejected"
    );

    // 10. A self-loop is storable but reaches no *neighbor* in any direction —
    //     the anchor must never appear as its own neighbor.
    g.upsert_edges(vec![edge("instrument:AAPL", "tracks", "instrument:AAPL")])
        .await
        .unwrap_or_else(|error| panic!("self-loop edge must upsert: {error}"));
    let self_view = g
        .neighbors(
            "instrument:AAPL",
            &TraversalFilter {
                rels: Some(vec!["tracks".to_string()]),
                ..filter(Direction::Both, 1)
            },
        )
        .await
        .unwrap_or_else(|error| panic!("self-loop read must succeed: {error}"));
    assert!(
        self_view.is_empty(),
        "a self-loop must not surface the anchor as its own neighbor"
    );
    let self_expand = g
        .expand(
            &["instrument:AAPL".to_string()],
            &filter(Direction::Both, 1),
        )
        .await
        .unwrap_or_else(|error| panic!("self-loop expand must succeed: {error}"));
    assert!(
        self_expand.edges.iter().all(|e| e.rel != "tracks"),
        "expand must not traverse a self-loop"
    );

    // 11. Timestamps must be normalized UTC (lexicographic == chronological);
    //     looser forms are rejected loudly instead of mis-filtering silently.
    assert!(
        g.neighbors(
            "instrument:AAPL",
            &TraversalFilter {
                as_of: Some("2024-01-01".to_string()),
                ..filter(Direction::Out, 1)
            },
        )
        .await
        .is_err(),
        "date-only as_of must be rejected"
    );

    // 12. Contract violations are loud.
    assert!(g.upsert_nodes(Vec::new()).await.is_err());
    assert!(g.upsert_edges(Vec::new()).await.is_err());
    assert!(
        g.neighbors("instrument:AAPL", &filter(Direction::Out, 0))
            .await
            .is_err()
    );
    assert!(g.expand(&[], &filter(Direction::Out, 1)).await.is_err());
}

#[tokio::test]
async fn in_memory_engine_meets_the_graph_contract() {
    conformance_suite(&AnyGraph::Mem(InMemoryGraphEngine::default())).await;
}

/// The Bolt leg (A4): the SAME suite against a real Memgraph/Neo4j server.
/// Compiles with `--features bolt` and self-skips unless `TDW_BOLT_TEST_URL`
/// is set (e.g. `bolt://127.0.0.1:7687`), mirroring the Postgres-leg
/// precedent in `tdw-outbox/tests/conformance.rs`. The target graph is WIPED
/// per run — point it at a disposable test instance only.
#[cfg(feature = "bolt")]
#[tokio::test]
async fn bolt_engine_meets_the_graph_contract() {
    let Ok(uri) = std::env::var("TDW_BOLT_TEST_URL") else {
        eprintln!("TDW_BOLT_TEST_URL unset; skipping bolt conformance leg");
        return;
    };
    let user = std::env::var("TDW_BOLT_TEST_USER").unwrap_or_default();
    let password = std::env::var("TDW_BOLT_TEST_PASSWORD").unwrap_or_default();
    // Memgraph's default database is "memgraph"; Neo4j's is "neo4j".
    // Override via TDW_BOLT_TEST_DB when targeting Neo4j.
    let db = std::env::var("TDW_BOLT_TEST_DB").unwrap_or_else(|_| "memgraph".to_string());
    let engine = tdw_storage_graph::BoltGraphEngine::connect(&uri, &user, &password, &db)
        .await
        .unwrap_or_else(|error| panic!("bolt connect must succeed: {error}"));
    engine
        .wipe_for_tests()
        .await
        .unwrap_or_else(|error| panic!("bolt wipe must succeed: {error}"));
    conformance_suite(&AnyGraph::Bolt(engine)).await;
}
