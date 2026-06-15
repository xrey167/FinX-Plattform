//! Integration tests for K-R4 pattern mining (determinism, bounds, idempotency).
//!
//! Uses `InMemoryGraphEngine` with a fixed fixture graph so all assertions are
//! deterministic (no I/O, no external state).

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

use std::sync::Arc;

use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance};
use tdw_patterns::{MiningLimits, PatternEngine, PatternIndex, mining::PatternError};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_taxonomy::EntityKind;

const NOW: &str = "2025-06-01T00:00:00Z";
const WINDOW: &str = "2025-06-01";

/// Build a small fixture graph:
///
/// ```text
/// instrument:AAPL --listed_on--> venue:NASDAQ
/// instrument:MSFT --listed_on--> venue:NASDAQ
/// instrument:GOOG --listed_on--> venue:NASDAQ
/// instrument:AAPL --supplier_of--> instrument:MSFT
/// instrument:MSFT --supplier_of--> instrument:GOOG
/// ```
///
/// Expected motifs with support ≥ 2:
/// - `"listed_on"` (support 3)
/// - `"supplier_of"` (support 2)
/// - `"listed_on::supplier_of"` (both AAPL and MSFT are `from` in both) → support 2
async fn build_fixture_graph() -> Arc<InMemoryGraphEngine> {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let prov = Provenance::Ingest {
        source: "test".to_string(),
    };

    let node = |id: &str, kind: EntityKind| GraphNode {
        id: id.to_string(),
        kind,
        label: id.to_string(),
        aliases: vec![],
        props: serde_json::Value::Null,
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

    let edge = |from: &str, rel: &str, to: &str| GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: serde_json::Value::Null,
        provenance: prov.clone(),
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };

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

    graph
}

// ── determinism test ──────────────────────────────────────────────────────────

#[tokio::test]
async fn determinism_same_fixture_produces_same_patterns() {
    let graph = build_fixture_graph().await;
    let engine = PatternEngine::default();

    let mut index_a = PatternIndex::default();
    engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index_a,
            NOW,
            WINDOW,
        )
        .await
        .expect("first run");

    let mut index_b = PatternIndex::default();
    engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index_b,
            NOW,
            WINDOW,
        )
        .await
        .expect("second run");

    // Same canonical keys, same support counts.
    let keys_a: Vec<_> = index_a.iter().map(|(k, _)| k.clone()).collect();
    let keys_b: Vec<_> = index_b.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(keys_a, keys_b, "pattern keys differ between runs");

    for (canonical, rec_a) in index_a.iter() {
        let rec_b = index_b
            .get(canonical)
            .expect("pattern must exist in second run");
        assert_eq!(
            rec_a.support, rec_b.support,
            "support mismatch for {canonical}"
        );
    }
}

// ── expected patterns test ────────────────────────────────────────────────────

#[tokio::test]
async fn fixture_produces_expected_patterns() {
    let graph = build_fixture_graph().await;
    let engine = PatternEngine::default();
    let mut index = PatternIndex::default();

    let report = engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("mining run");

    // At least the single-label motifs must be present.
    assert!(index.contains("listed_on"), "listed_on motif missing");
    assert!(index.contains("supplier_of"), "supplier_of motif missing");

    // Supports.
    assert_eq!(index.get("listed_on").expect("listed_on").support, 3);
    assert_eq!(index.get("supplier_of").expect("supplier_of").support, 2);

    // Report sanity.
    assert!(report.patterns_created >= 2);
    assert!(report.motifs_examined >= 2);
}

// ── support counts distinct source entities, not (from,to) pairs ──────────────

/// Regression: support must equal the number of *distinct source entities* that
/// exhibit a motif — not the number of `(from, to)` instance pairs. A single
/// entity with several `to` targets under one label contributes support 1.
///
/// Before the fix, `support` was the de-duped pair count, so one entity with two
/// edges of the same label reached `min_support = 2` on its own and persisted a
/// false-positive pattern to the graph. The shared fixture never exercised this
/// because every entity there has out-degree 1 per label.
#[tokio::test]
async fn support_counts_distinct_sources_not_pairs() {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let prov = Provenance::Ingest {
        source: "test".to_string(),
    };

    let node = |id: &str| GraphNode {
        id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: vec![],
        props: serde_json::Value::Null,
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };
    graph
        .upsert_nodes(vec![
            node("instrument:AAPL"),
            node("instrument:MSFT"),
            node("instrument:GOOG"),
        ])
        .await
        .expect("upsert nodes");

    let edge = |from: &str, rel: &str, to: &str| GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: serde_json::Value::Null,
        provenance: prov.clone(),
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };
    // AAPL alone exhibits `supplier_of` twice (→ MSFT, → GOOG): one distinct
    // source entity, two instance pairs.
    graph
        .upsert_edges(vec![
            edge("instrument:AAPL", "supplier_of", "instrument:MSFT"),
            edge("instrument:AAPL", "supplier_of", "instrument:GOOG"),
        ])
        .await
        .expect("upsert edges");

    let limits = |min_support| MiningLimits {
        max_candidates: 8_000,
        max_instance_scan: 20_000,
        max_iterations: 50_000,
        max_provenance_edges: 64,
        max_motif_edges: 1,
        min_support,
    };

    // min_support = 2: a single-entity motif must NOT be recorded.
    let mut index = PatternIndex::default();
    PatternEngine::with_limits(limits(2))
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("mining run");
    assert!(
        !index.contains("supplier_of"),
        "one entity with two same-label edges must not reach min_support=2 (false-positive pattern)"
    );

    // min_support = 1: recorded with support 1 (distinct sources), not 2 (pairs).
    let mut index1 = PatternIndex::default();
    PatternEngine::with_limits(limits(1))
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn GraphEngine>),
            &mut index1,
            NOW,
            WINDOW,
        )
        .await
        .expect("mining run");
    assert_eq!(
        index1
            .get("supplier_of")
            .expect("supplier_of present at min_support=1")
            .support,
        1,
        "support must count the single distinct source entity, not the two (from,to) pairs"
    );
}

// ── idempotent re-mine ────────────────────────────────────────────────────────

#[tokio::test]
async fn idempotent_remine_updates_not_duplicates() {
    let graph = build_fixture_graph().await;
    let engine = PatternEngine::default();
    let mut index = PatternIndex::default();

    let report1 = engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("first run");

    let count_after_first = index.len();
    assert!(report1.patterns_created >= 2);

    // Second run: all patterns already exist → updated, not created.
    let report2 = engine
        .run_pattern_mining_at(
            &(graph.clone() as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("second run");

    assert_eq!(
        index.len(),
        count_after_first,
        "index grew on second run (duplicate)"
    );
    assert_eq!(
        report2.patterns_created, 0,
        "second run must not create new patterns"
    );
    assert!(
        report2.patterns_updated >= 2,
        "second run must update existing patterns"
    );
}

// ── bounds: candidate cap ─────────────────────────────────────────────────────

#[tokio::test]
async fn candidate_budget_exceeded_is_hard_error() {
    let graph = build_fixture_graph().await;
    // Tiny candidate cap so even the small fixture trips it.
    let limits = MiningLimits {
        max_candidates: 1,
        max_instance_scan: 20_000,
        max_iterations: 50_000,
        max_provenance_edges: 64,
        max_motif_edges: 1,
        min_support: 1,
    };
    let engine = PatternEngine::with_limits(limits);
    let mut index = PatternIndex::default();

    let err = engine
        .run_pattern_mining_at(
            &(graph as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect_err("must error on candidate budget exceeded");

    assert!(
        matches!(err, PatternError::CandidateBudgetExceeded { .. }),
        "unexpected error variant: {err}"
    );
}

// ── bounds: instance scan cap ─────────────────────────────────────────────────

#[tokio::test]
async fn instance_scan_budget_exceeded_is_hard_error() {
    let graph = build_fixture_graph().await;
    let limits = MiningLimits {
        max_candidates: 8_000,
        max_instance_scan: 1, // trip on first scan
        max_iterations: 50_000,
        max_provenance_edges: 64,
        max_motif_edges: 1,
        min_support: 1,
    };
    let engine = PatternEngine::with_limits(limits);
    let mut index = PatternIndex::default();

    let err = engine
        .run_pattern_mining_at(
            &(graph as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect_err("must error on instance-scan budget exceeded");

    assert!(
        matches!(err, PatternError::InstanceBudgetExceeded { .. }),
        "unexpected error variant: {err}"
    );
}

// ── temporal leakage regression ───────────────────────────────────────────────

#[tokio::test]
async fn post_as_of_edges_never_appear_in_patterns() {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let prov = Provenance::Ingest {
        source: "test".to_string(),
    };

    let node = |id: &str| GraphNode {
        id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: vec![],
        props: serde_json::Value::Null,
        valid_from: Some("2020-01-01T00:00:00Z".to_string()),
        valid_to: None,
    };
    graph
        .upsert_nodes(vec![
            node("instrument:A"),
            node("instrument:B"),
            node("instrument:C"),
        ])
        .await
        .expect("upsert nodes");

    // Edge active BEFORE now: should mine.
    graph
        .upsert_edges(vec![GraphEdge {
            from: "instrument:A".to_string(),
            to: "instrument:B".to_string(),
            rel: "past_rel".to_string(),
            props: serde_json::Value::Null,
            provenance: prov.clone(),
            valid_from: Some("2020-01-01T00:00:00Z".to_string()),
            valid_to: None,
        }])
        .await
        .expect("upsert past edge");

    // Edge active AFTER now (future edge): must NOT appear in patterns.
    graph
        .upsert_edges(vec![GraphEdge {
            from: "instrument:A".to_string(),
            to: "instrument:C".to_string(),
            rel: "future_rel".to_string(),
            props: serde_json::Value::Null,
            provenance: prov,
            valid_from: Some("2099-01-01T00:00:00Z".to_string()),
            valid_to: None,
        }])
        .await
        .expect("upsert future edge");

    let engine = PatternEngine::default();
    let mut index = PatternIndex::default();

    // Mine at NOW (2025-06-01) — well before the future edge.
    engine
        .run_pattern_mining_at(
            &(graph as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            "2025-06-01T00:00:00Z",
            WINDOW,
        )
        .await
        .expect("mining run");

    // The future_rel motif must NOT appear.
    assert!(
        !index.contains("future_rel"),
        "future_rel must not appear in patterns (temporal leakage)"
    );
}

// ── index round-trip ──────────────────────────────────────────────────────────

#[tokio::test]
async fn pattern_index_round_trips() {
    let graph = build_fixture_graph().await;
    let engine = PatternEngine::default();
    let mut index = PatternIndex::default();

    engine
        .run_pattern_mining_at(
            &(graph as Arc<dyn tdw_core::GraphEngine>),
            &mut index,
            NOW,
            WINDOW,
        )
        .await
        .expect("mining run");

    assert!(!index.is_empty());
    let json = index.to_json().expect("serialize");
    let restored = PatternIndex::from_json(&json).expect("deserialize");
    assert_eq!(restored, index);
}
