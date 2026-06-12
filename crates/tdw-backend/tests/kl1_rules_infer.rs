//! K-L1 gate tests: rules-driven inference lives in the daemon.
//!
//! Seven gates (all offline, no Docker/network, deterministic):
//!
//! 1. **DeriveEdge e2e** — ingest a doc, seed a base graph edge matching a
//!    `DeriveEdge` rule, fire inference, assert the derived edge is in the graph
//!    with `Provenance::Rule` and the rule's `rule_id`.
//! 2. **PropagateTag e2e** — seed a tag on an entity, fire inference with a
//!    `PropagateTag` rule, assert the tag is propagated to a connected entity.
//! 3. **Hot-reload: changed rules applied** — swap in a new rule set via
//!    `InferEngine::hot_reload`; verify the version increments and a subsequent
//!    run uses the new rules.
//! 4. **Hot-reload: unstratifiable rejected, old rules survive** — attempt to
//!    `hot_reload` a stratification-violating rule set; verify it is rejected and
//!    the engine still runs with the old rules.
//! 5. **Limits-loud** — configure a `RunLimits` with `max_derived = 1`; verify
//!    that `run_incremental` returns a `DerivedLimitExceeded` error (not silently
//!    truncated) when the rule set would produce more than one derived edge.
//! 6. **Retraction e2e** — exercise `Backend::retract_knowledge_fact`; assert the
//!    derived edge is gone and the base edge is untouched after retraction.
//! 7. **MCP JSON-RPC ingest → `Provenance::Rule`** — drive `tdw.kg.ingest`
//!    through the real MCP JSON-RPC tool surface with an inference engine wired
//!    in; seed a base edge matching a `DeriveEdge` rule and assert the derived
//!    edge appears in the graph with `Provenance::Rule { rule_id }` after the
//!    ingest call completes (honesty gate for issue 2: the production MCP path
//!    fires `run_incremental`, not just the `Backend` direct methods).

use std::sync::Arc;

use serde_json::json;
use tdw_backend::prelude::*;
use tdw_core::{Direction, GraphEdge, GraphNode, Provenance, TraversalFilter};
use tdw_infer::{ChangeSet, EdgePattern, InferEngine, InferError, InferRule, RunLimits};
use tdw_kg::EntityKind;
use tdw_tags::{InMemoryTagEngine, TagAssignment, TagDefinition, TagEngine as _};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal [`KnowledgeDocument`] for entity `entity_id` with the given tags.
fn make_doc(id: &str, entity_id: &str, tags: &[&str]) -> KnowledgeDocument {
    KnowledgeDocument {
        id: id.to_string(),
        body: format!("test doc for {entity_id}"),
        entity: Entity {
            entity_id: entity_id.to_string(),
            kind: EntityKind::Instrument,
            label: entity_id.to_string(),
            aliases: vec![],
        },
        tags: tags.iter().map(|t| (*t).to_string()).collect(),
        source: None,
        plane: None,
        as_of: None,
        mentions: vec![],
    }
}

/// Seed a base `GraphEdge` (and both endpoint nodes) directly into `graph`.
async fn seed_edge(graph: &Arc<dyn tdw_core::GraphEngine>, from: &str, rel: &str, to: &str) {
    graph
        .upsert_nodes(vec![
            GraphNode {
                id: from.to_string(),
                kind: EntityKind::Instrument,
                label: from.to_string(),
                aliases: vec![],
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            },
            GraphNode {
                id: to.to_string(),
                kind: EntityKind::Instrument,
                label: to.to_string(),
                aliases: vec![],
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            },
        ])
        .await
        .expect("upsert_nodes should succeed");
    graph
        .upsert_edges(vec![GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Ingest {
                source: "test".to_string(),
            },
            valid_from: None,
            valid_to: None,
        }])
        .await
        .expect("upsert_edges should succeed");
}

/// Build a [`DeriveEdge`] rule: `A -via-> B  =>  A -derived_type-> B`.
fn derive_rule(rule_id: &str, via: &str, derived_type: &str) -> InferRule {
    InferRule::DeriveEdge {
        rule_id: rule_id.to_string(),
        stratum: 0,
        when: vec![EdgePattern {
            rel: via.to_string(),
        }],
        derived_type: derived_type.to_string(),
    }
}

/// Build a [`PropagateTag`] rule: entities with `tag` propagate it outbound
/// along `along` edges, one hop.
fn propagate_rule(rule_id: &str, tag: &str, along: &str) -> InferRule {
    InferRule::PropagateTag {
        rule_id: rule_id.to_string(),
        stratum: 0,
        tag: tag.to_string(),
        include_descendants: false,
        along: vec![along.to_string()],
        outbound: true,
        max_hops: 1,
    }
}

// ---------------------------------------------------------------------------
// Gate 1: DeriveEdge e2e
// ---------------------------------------------------------------------------

#[tokio::test]
async fn derive_edge_rule_produces_derived_edge_with_rule_provenance() {
    let backend = Backend::in_memory_for_tests().await;
    let graph = backend.graph_engine();
    let now = "2026-01-01";

    // Index a doc so the entity node exists in the graph.
    backend
        .knowledge_index_at(make_doc("doc-a", "instrument:A", &["asset:equity"]), now)
        .await
        .expect("knowledge_index_at should succeed");

    // Seed a base edge: instrument:A -listed_on-> exchange:NYSE
    seed_edge(&graph, "instrument:A", "listed_on", "exchange:NYSE").await;

    // Wire a DeriveEdge rule: listed_on => trades_on
    {
        let infer = backend.infer_engine_handle();
        let mut guard = infer.lock().await;
        guard
            .hot_reload(vec![derive_rule("r-trades-on", "listed_on", "trades_on")])
            .expect("hot_reload should accept valid rule");
    }

    // Fire inference with a ChangeSet that includes listed_on.
    {
        let infer = backend.infer_engine_handle();
        let mut guard = infer.lock().await;
        let tags_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());
        let mut changed = ChangeSet::default();
        changed.edge_types.insert("listed_on".to_string());
        let report = guard
            .run_incremental(&graph, &tags_engine, now, &changed)
            .await
            .expect("run_incremental should succeed");
        assert!(
            report.derived_edges >= 1,
            "at least one edge must be derived; report: {report:?}"
        );
    }

    // Assert the derived edge instrument:A -trades_on-> exchange:NYSE is in the graph.
    let filter = TraversalFilter {
        rels: Some(vec!["trades_on".to_string()]),
        kinds: None,
        as_of: None,
        direction: Direction::Out,
        max_hops: 1,
    };
    let neighbors = graph
        .neighbors("instrument:A", &filter)
        .await
        .expect("neighbors should succeed");

    // neighbors returns Vec<(GraphEdge, GraphNode)>
    let derived = neighbors
        .iter()
        .map(|(edge, _node)| edge)
        .find(|edge| edge.rel == "trades_on" && edge.to == "exchange:NYSE");
    assert!(
        derived.is_some(),
        "derived edge instrument:A -trades_on-> exchange:NYSE must be present; \
         edges: {:?}",
        neighbors.iter().map(|(e, _)| e).collect::<Vec<_>>()
    );

    // Assert Provenance::Rule with our rule_id.
    let edge = derived.unwrap();
    assert!(
        matches!(&edge.provenance, Provenance::Rule { rule_id, .. } if rule_id == "r-trades-on"),
        "derived edge must carry Provenance::Rule {{ rule_id: \"r-trades-on\" }}, got {:?}",
        edge.provenance
    );
}

// ---------------------------------------------------------------------------
// Gate 2: PropagateTag e2e
// ---------------------------------------------------------------------------

#[tokio::test]
async fn propagate_tag_rule_assigns_tag_to_connected_entity() {
    let backend = Backend::in_memory_for_tests().await;
    let graph = backend.graph_engine();
    let now = "2026-01-01";

    // Seed nodes and a base edge: instrument:TSLA -sector_peer_of-> instrument:RIVN
    seed_edge(
        &graph,
        "instrument:TSLA",
        "sector_peer_of",
        "instrument:RIVN",
    )
    .await;

    // Build a standalone InMemoryTagEngine so we control tag state precisely.
    let tags_engine = Arc::new(InMemoryTagEngine::default());
    tags_engine
        .define(TagDefinition {
            tag_id: "sector:ev".to_string(),
            parent: None,
            ttl_days: None,
        })
        .await
        .expect("define sector:ev should succeed");
    tags_engine
        .assign(TagAssignment {
            entity_id: "instrument:TSLA".to_string(),
            tag_id: "sector:ev".to_string(),
            assigned_at: now.to_string(),
            expires_at: None,
            provenance: "test".to_string(),
        })
        .await
        .expect("assign sector:ev to TSLA should succeed");

    // Wire a PropagateTag rule: sector:ev along sector_peer_of.
    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![propagate_rule(
            "r-propagate-ev",
            "sector:ev",
            "sector_peer_of",
        )])
        .expect("hot_reload should accept valid rule");

    // Fire inference with sector:ev in the change set.
    let tags_dyn: Arc<dyn tdw_tags::TagEngine> =
        Arc::clone(&tags_engine) as Arc<dyn tdw_tags::TagEngine>;
    let mut changed = ChangeSet::default();
    changed.tags.insert("sector:ev".to_string());
    let report = engine
        .run_incremental(&graph, &tags_dyn, now, &changed)
        .await
        .expect("run_incremental should succeed");
    assert!(
        report.assigned_tags >= 1,
        "at least one tag must be propagated; report: {report:?}"
    );

    // Assert instrument:RIVN now carries sector:ev.
    let rivn_tags = tags_engine
        .active_tags("instrument:RIVN", now)
        .await
        .expect("active_tags should succeed");
    assert!(
        rivn_tags.iter().any(|t| t == "sector:ev"),
        "instrument:RIVN must carry sector:ev after propagation; tags: {rivn_tags:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: Hot-reload — changed rules applied, version bumps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hot_reload_replaces_rules_and_bumps_version() {
    let mut engine = InferEngine::default();
    assert_eq!(engine.version(), 0, "fresh engine starts at version 0");

    // Load first rule set.
    engine
        .hot_reload(vec![derive_rule("r-v1", "edge_a", "derived_a")])
        .expect("first hot_reload should succeed");
    let v1 = engine.version();
    assert!(v1 > 0, "version must increment after first hot_reload");

    // Swap to a different rule set.
    engine
        .hot_reload(vec![derive_rule("r-v2", "edge_b", "derived_b")])
        .expect("second hot_reload should succeed");
    let v2 = engine.version();
    assert!(
        v2 > v1,
        "version must increment again after second hot_reload"
    );

    // The new rule set produces derived_b — verify by running on a graph with
    // a base edge of type edge_b.
    let graph: Arc<dyn tdw_core::GraphEngine> =
        Arc::new(tdw_storage_graph::InMemoryGraphEngine::default());
    seed_edge(&graph, "node:X", "edge_b", "node:Y").await;

    let tags_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut changed = ChangeSet::default();
    changed.edge_types.insert("edge_b".to_string());
    let report = engine
        .run_incremental(&graph, &tags_engine, "2026-01-01", &changed)
        .await
        .expect("run_incremental with v2 rules should succeed");
    assert!(
        report.derived_edges >= 1,
        "v2 rule must fire on edge_b; report: {report:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 4: Hot-reload — unstratifiable rule set rejected, old rules survive
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hot_reload_rejects_self_recursive_rule_keeping_old_ruleset() {
    let mut engine = InferEngine::default();

    // Load a valid first rule set.
    engine
        .hot_reload(vec![derive_rule("r-good", "base_edge", "derived_good")])
        .expect("initial hot_reload should succeed");
    let v_before = engine.version();

    // Attempt to reload a self-recursive rule (consumes the type it produces —
    // `validate_rule` catches this as a stratification violation).
    let bad_rule = InferRule::DeriveEdge {
        rule_id: "r-bad".to_string(),
        stratum: 0,
        when: vec![EdgePattern {
            rel: "loop_type".to_string(),
        }],
        derived_type: "loop_type".to_string(),
    };
    let result = engine.hot_reload(vec![bad_rule]);
    assert!(
        result.is_err(),
        "hot_reload with self-recursive rule must be rejected"
    );
    assert!(
        matches!(result, Err(InferError::InvalidRule { .. })),
        "error must be InvalidRule, got {:?}",
        result
    );

    // Version must NOT have changed — old rule set survives.
    assert_eq!(
        engine.version(),
        v_before,
        "version must not change after a rejected hot_reload"
    );

    // The old rule still fires correctly.
    let graph: Arc<dyn tdw_core::GraphEngine> =
        Arc::new(tdw_storage_graph::InMemoryGraphEngine::default());
    seed_edge(&graph, "node:P", "base_edge", "node:Q").await;

    let tags_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut changed = ChangeSet::default();
    changed.edge_types.insert("base_edge".to_string());
    let report = engine
        .run_incremental(&graph, &tags_engine, "2026-01-01", &changed)
        .await
        .expect("old rule should still fire after rejected reload");
    assert!(
        report.derived_edges >= 1,
        "old rule must still produce derived_good after rejected reload; report: {report:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 5: Limits-loud — DerivedLimitExceeded is surfaced, not silent
// ---------------------------------------------------------------------------

#[tokio::test]
async fn limits_exceeded_returns_error_not_silent_truncation() {
    // max_derived = 1: the engine errors after deriving more than 1 edge.
    let mut engine = InferEngine::with_limits(RunLimits {
        max_iterations: 32,
        max_derived: 1,
    });

    // Two DeriveEdge rules that both fire on their respective base edges.
    engine
        .hot_reload(vec![
            derive_rule("r-limit1", "hop_a", "derived_hop_a"),
            derive_rule("r-limit2", "hop_b", "derived_hop_b"),
        ])
        .expect("hot_reload should accept two valid rules");

    // Seed two base edges so both rules fire, producing 2 derived edges.
    let graph: Arc<dyn tdw_core::GraphEngine> =
        Arc::new(tdw_storage_graph::InMemoryGraphEngine::default());
    seed_edge(&graph, "node:M", "hop_a", "node:N").await;
    seed_edge(&graph, "node:P", "hop_b", "node:Q").await;

    let tags_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut changed = ChangeSet::default();
    changed.edge_types.insert("hop_a".to_string());
    changed.edge_types.insert("hop_b".to_string());

    let result = engine
        .run_incremental(&graph, &tags_engine, "2026-01-01", &changed)
        .await;

    assert!(
        result.is_err(),
        "run_incremental must return Err when max_derived is exceeded, not Ok"
    );
    assert!(
        matches!(result, Err(InferError::DerivedLimitExceeded { .. })),
        "error must be DerivedLimitExceeded, got {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// Gate 6: Retraction e2e
// ---------------------------------------------------------------------------

#[tokio::test]
async fn retract_removes_derived_edge_leaves_base_edge_intact() {
    let backend = Backend::in_memory_for_tests().await;
    let graph = backend.graph_engine();
    let now = "2026-01-01";

    // Seed a base edge: instrument:BRK -owns-> instrument:GEICO
    seed_edge(&graph, "instrument:BRK", "owns", "instrument:GEICO").await;

    // Wire a DeriveEdge rule: owns => controls
    {
        let infer = backend.infer_engine_handle();
        let mut guard = infer.lock().await;
        guard
            .hot_reload(vec![derive_rule("r-controls", "owns", "controls")])
            .expect("hot_reload should succeed");
    }

    // Fire inference to materialise the derived edge.
    {
        let infer = backend.infer_engine_handle();
        let mut guard = infer.lock().await;
        let tags_engine: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());
        let mut changed = ChangeSet::default();
        changed.edge_types.insert("owns".to_string());
        let report = guard
            .run_incremental(&graph, &tags_engine, now, &changed)
            .await
            .expect("run_incremental should succeed");
        assert!(
            report.derived_edges >= 1,
            "derived edge must be produced before retraction test; report: {report:?}"
        );
    }

    // Confirm the derived edge is present.
    let filter_controls = TraversalFilter {
        rels: Some(vec!["controls".to_string()]),
        kinds: None,
        as_of: None,
        direction: Direction::Out,
        max_hops: 1,
    };
    let before = graph
        .neighbors("instrument:BRK", &filter_controls)
        .await
        .expect("neighbors should succeed");
    assert!(
        before.iter().any(|(e, _)| e.rel == "controls"),
        "derived controls edge must exist before retraction; edges: {:?}",
        before.iter().map(|(e, _)| e).collect::<Vec<_>>()
    );

    // Retract by passing the BASE edge key as the seed: retract() traverses the
    // support closure, so the base edge key seeds the frontier and the derived
    // edge (whose support contains the base key) is removed transitively.
    let fact_key = tdw_infer::edge_key("instrument:BRK", "owns", "instrument:GEICO");
    let report = backend
        .retract_knowledge_fact(&fact_key)
        .await
        .expect("retract_knowledge_fact should succeed");
    assert!(
        report.unremovable_tags.is_empty(),
        "retraction must not leave unremovable tags; got {:?}",
        report.unremovable_tags
    );

    // The derived edge must be gone.
    let after_derived = graph
        .neighbors("instrument:BRK", &filter_controls)
        .await
        .expect("neighbors should succeed");
    assert!(
        after_derived.iter().all(|(e, _)| e.rel != "controls"),
        "derived controls edge must be removed after retraction; edges: {:?}",
        after_derived.iter().map(|(e, _)| e).collect::<Vec<_>>()
    );

    // The base edge (owns) must still be present.
    let filter_owns = TraversalFilter {
        rels: Some(vec!["owns".to_string()]),
        kinds: None,
        as_of: None,
        direction: Direction::Out,
        max_hops: 1,
    };
    let base_after = graph
        .neighbors("instrument:BRK", &filter_owns)
        .await
        .expect("neighbors should succeed");
    assert!(
        base_after.iter().any(|(e, _)| e.rel == "owns"),
        "base owns edge must remain intact after retraction; edges: {:?}",
        base_after.iter().map(|(e, _)| e).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Gate 7: MCP JSON-RPC ingest path fires inference → Provenance::Rule
// ---------------------------------------------------------------------------

/// Drive `tdw.kg.ingest` through the real MCP JSON-RPC surface with an
/// inference engine wired in. This is the production path (issue 2 honesty
/// gate): `McpServer::dispatch_knowledge_ingest_tool` must call
/// `run_incremental` after each ingest batch.
#[tokio::test]
async fn mcp_ingest_tool_fires_inference_and_derives_edge_with_rule_provenance() {
    let backend = Backend::in_memory_for_tests().await;
    let graph = backend.graph_engine();
    let now = "2026-01-01";

    // Seed a base edge BEFORE ingest so the rule fires on the described_by /
    // mentions ChangeSet the K-E3 indexer produces. The DeriveEdge rule fires
    // on "described_by" (the standard edge written by the indexer).
    // Note: KnowledgeDocument.id must be a plain identifier (alphanumeric + _ - only),
    // so use "doc-probe" not "doc:probe".
    seed_edge(&graph, "instrument:INFER-MCP", "described_by", "doc-probe").await;

    // Wire the DeriveEdge rule: described_by => confirmed_by
    {
        let infer = backend.infer_engine_handle();
        let mut guard = infer.lock().await;
        guard
            .hot_reload(vec![derive_rule(
                "r-mcp-honesty",
                "described_by",
                "confirmed_by",
            )])
            .expect("hot_reload should accept valid rule");
    }

    // Build an MCP server with the full K-L1 surface wired up.
    // The runtime + indexer + infer handles are all shared with the Backend.
    let runtime = backend.knowledge_runtime_handle();
    let indexer = backend.knowledge_indexer_handle();
    let infer = backend.infer_engine_handle();
    let mut server = McpServer::new()
        .with_knowledge(runtime)
        .with_indexer(indexer)
        .with_infer_engine(infer);

    // Initialize the MCP session (3-step handshake per the MCP spec).
    let init_line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"kl1-test","version":"1.0.0"}}}"#;
    let init_msgs = server.handle_json_rpc_line(init_line);
    assert_eq!(
        init_msgs.len(),
        1,
        "initialize must return exactly one message"
    );

    // The client sends `notifications/initialized` (a notification — no id,
    // no response) to complete the handshake before issuing any tool calls.
    // Without this the server's `initialized` flag stays false and every
    // `tools/call` returns JSON-RPC -32002 "server is not initialized".
    let notif_msgs = server.handle_json_rpc_line(
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
    );
    assert!(
        notif_msgs.is_empty(),
        "notifications/initialized must produce no response messages"
    );

    // Ingest a document for entity instrument:INFER-MCP.
    // The K-E3 indexer writes a described_by edge and the inference engine
    // fires, deriving confirmed_by from the DeriveEdge rule above.
    let ingest_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "tdw.kg.ingest",
            "arguments": {
                "documents": [{
                    "id": "doc-probe",
                    "body": "probe document for MCP inference gate",
                    "entity": {
                        "entity_id": "instrument:INFER-MCP",
                        "kind": "instrument",
                        "label": "INFER-MCP",
                        "aliases": []
                    },
                    "tags": ["test:tag"]
                }],
                "now": now
            }
        }
    });
    let ingest_msgs = server.handle_json_rpc_line(&ingest_request.to_string());
    assert_eq!(
        ingest_msgs.len(),
        1,
        "ingest must return exactly one message"
    );

    // Parse the response and confirm the document landed (not duplicate/error).
    let response: serde_json::Value =
        serde_json::from_str(&ingest_msgs[0]).expect("response must be valid JSON");
    // The MCP tool result is nested: result.content[0].text contains the JSON body.
    // If that layer is absent, surface the full response to aid debugging.
    let content_arr = response["result"]["content"]
        .as_array()
        .unwrap_or_else(|| panic!("result.content must be an array; response: {response}"));
    assert!(
        !content_arr.is_empty(),
        "result.content must not be empty; response: {response}"
    );
    let content_str = content_arr[0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("result.content[0].text must be a string; response: {response}"));
    let report: serde_json::Value =
        serde_json::from_str(content_str).expect("content text must be valid JSON");
    assert_eq!(
        report["summary"]["landed"],
        json!(1),
        "exactly one document must land; report: {report}"
    );

    // Assert the derived edge instrument:INFER-MCP -confirmed_by-> doc:probe
    // is present in the graph with Provenance::Rule { rule_id: "r-mcp-honesty" }.
    let filter = TraversalFilter {
        rels: Some(vec!["confirmed_by".to_string()]),
        kinds: None,
        as_of: None,
        direction: Direction::Out,
        max_hops: 1,
    };
    let neighbors = graph
        .neighbors("instrument:INFER-MCP", &filter)
        .await
        .expect("neighbors should succeed");

    let derived = neighbors
        .iter()
        .map(|(edge, _node)| edge)
        .find(|edge| edge.rel == "confirmed_by" && edge.to == "doc-probe");
    assert!(
        derived.is_some(),
        "MCP ingest must trigger inference and produce confirmed_by edge; \
         edges: {:?}",
        neighbors.iter().map(|(e, _)| e).collect::<Vec<_>>()
    );

    let edge = derived.unwrap();
    assert!(
        matches!(
            &edge.provenance,
            Provenance::Rule { rule_id, .. } if rule_id == "r-mcp-honesty"
        ),
        "derived edge must carry Provenance::Rule {{ rule_id: \"r-mcp-honesty\" }}, \
         got {:?}",
        edge.provenance
    );
}
