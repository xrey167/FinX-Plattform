//! Tests for the agent writeback gate (knowledge-system B9).
//!
//! Every test drives the [`ProposalQueue`] over fully in-memory engines (an
//! `InMemoryGraphEngine` plus a `GraphTagEngine` writing into the SAME graph, so
//! tag definitions made through the gate are visible to later validators). The
//! contract under test: admission below `Learning` is refused and admitted at
//! `Learning`/`SelfModifying`; the per-agent pending cap; every automated
//! validator failure; eval-driven promotion (`promote_for_agent`) and the human
//! `approve` path; terminal `reject`; materialization writing ONLY `Ready`
//! proposals with `Provenance::Agent { gated: true }` / `agent:<id>;proposal:<pid>`
//! provenance; and a serde round-trip preserving queue state.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance, Result, Subgraph,
    TraversalFilter,
};
use tdw_knowledge::proposals::{
    PER_AGENT_PENDING_CAP, ProposalKind, ProposalQueue, READY_THRESHOLD,
};
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_tags::{TagDefinition, TagEngine};
use tdw_taxonomy::{Adaptivity, EntityKind, ValidationStatus};

const NOW: &str = "2026-05-22";

/// A shareable [`GraphEngine`] handle: one in-memory graph behind an `Arc`,
/// delegating the whole trait. Lets the gate's graph handle and the
/// graph-backed tag engine read/write the SAME store (mirrors the B8 harness).
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

/// A node fixture (entity/document) so edge/tag/annotation endpoints exist.
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

/// Build a shared in-memory graph (with two seeded entity nodes) and a tag
/// engine over it. Returns the trait-object handles the gate consumes plus the
/// raw graph for assertions.
async fn engines() -> (
    Arc<dyn GraphEngine>,
    Arc<dyn TagEngine>,
    Arc<InMemoryGraphEngine>,
) {
    let raw = Arc::new(InMemoryGraphEngine::default());
    raw.upsert_nodes(vec![node("instrument:AAPL"), node("instrument:MSFT")])
        .await
        .unwrap_or_else(|error| panic!("seed nodes: {error}"));
    let graph: Arc<dyn GraphEngine> = Arc::new(SharedGraph(raw.clone()));
    let tags: Arc<dyn TagEngine> = Arc::new(GraphTagEngine::new(SharedGraph(raw.clone())));
    (graph, tags, raw)
}

fn edge_kind(from: &str, to: &str, rel: &str) -> ProposalKind {
    ProposalKind::Edge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
    }
}

#[tokio::test]
async fn admission_gates_on_learning() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();

    // Below Learning: None and Configured are both refused.
    for adaptivity in [Adaptivity::None, Adaptivity::Configured] {
        let error = queue
            .submit(
                "agent:a",
                adaptivity,
                edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
                &graph,
                &tags,
                NOW,
            )
            .await
            .expect_err("below Learning must be refused");
        assert!(
            error.to_string().contains("below Learning"),
            "admission error names the gate: {error}"
        );
    }

    // Learning and SelfModifying are admitted (distinct payloads so neither is a
    // re-assertion of the other).
    let learning = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("Learning is admitted");
    assert_eq!(learning.status, ValidationStatus::Validated);

    let self_mod = queue
        .submit(
            "agent:b",
            Adaptivity::SelfModifying,
            edge_kind("instrument:MSFT", "instrument:AAPL", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("SelfModifying is admitted");
    assert_eq!(self_mod.status, ValidationStatus::Validated);
}

#[tokio::test]
async fn per_agent_pending_cap_is_enforced() {
    let (graph, tags, raw) = engines().await;
    // Use the DEFAULT cap; seed enough distinct edge targets to fill it + 1.
    assert_eq!(PER_AGENT_PENDING_CAP, 32);
    let mut targets = Vec::new();
    for index in 0..=PER_AGENT_PENDING_CAP {
        let id = format!("instrument:T{index}");
        raw.upsert_nodes(vec![node(&id)])
            .await
            .unwrap_or_else(|error| panic!("seed target {id}: {error}"));
        targets.push(id);
    }
    let mut queue = ProposalQueue::default();

    // The first `cap` distinct valid Edge proposals enqueue.
    for target in targets.iter().take(PER_AGENT_PENDING_CAP) {
        queue
            .submit(
                "agent:cap",
                Adaptivity::Learning,
                edge_kind("instrument:AAPL", target, "mentions"),
                &graph,
                &tags,
                NOW,
            )
            .await
            .unwrap_or_else(|error| panic!("under-cap submit should pass: {error}"));
    }

    // The (cap+1)-th valid proposal trips the per-agent cap.
    let error = queue
        .submit(
            "agent:cap",
            Adaptivity::Learning,
            edge_kind(
                "instrument:AAPL",
                &targets[PER_AGENT_PENDING_CAP],
                "mentions",
            ),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("the cap+1 submission must error");
    assert!(
        error.to_string().contains("cap"),
        "cap error names the limit: {error}"
    );
}

#[tokio::test]
async fn each_validator_failure_is_loud() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();

    // Edge endpoint missing.
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:GONE", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("missing endpoint must fail");
    assert!(error.to_string().contains("does not exist"), "{error}");

    // Define + materialize an edge, then re-assert it → rejected.
    let valid = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("first assertion is valid");
    queue
        .approve(&valid.id, "human", NOW)
        .expect("approve to Ready");
    queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize the edge");
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("re-assertion must fail");
    assert!(error.to_string().contains("already exists"), "{error}");
}

#[tokio::test]
async fn tag_and_annotation_validator_failures_are_loud() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();

    // Unknown tag on TagAssign.
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            ProposalKind::TagAssign {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:undefined".to_string(),
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("unknown tag must fail");
    assert!(error.to_string().contains("unknown tag"), "{error}");

    // Unknown parent on TagDefine.
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            ProposalKind::TagDefine {
                tag_id: "asset:equity:tech".to_string(),
                parent: Some("asset:missing".to_string()),
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("unknown parent must fail");
    assert!(error.to_string().contains("unknown parent"), "{error}");

    // Duplicate tag on TagDefine: define one, then re-define it.
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .await
    .expect("seed an existing tag");
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            ProposalKind::TagDefine {
                tag_id: "asset:equity".to_string(),
                parent: None,
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("duplicate tag define must fail");
    assert!(error.to_string().contains("already defined"), "{error}");

    // Oversized note on Annotation (> MAX_NOTE_CHARS).
    let huge = "n".repeat(5000);
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            ProposalKind::Annotation {
                entity_id: "instrument:AAPL".to_string(),
                note: huge,
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("oversized note must fail");
    assert!(error.to_string().contains("at most"), "{error}");

    // Control character in an Annotation note.
    let error = queue
        .submit(
            "agent:v",
            Adaptivity::Learning,
            ProposalKind::Annotation {
                entity_id: "instrument:AAPL".to_string(),
                note: "bad\u{0007}note".to_string(),
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect_err("control char must fail");
    assert!(error.to_string().contains("control characters"), "{error}");
}

#[tokio::test]
async fn promote_for_agent_respects_threshold_and_scope() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();

    let a = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("agent a proposal");
    let b = queue
        .submit(
            "agent:b",
            Adaptivity::Learning,
            edge_kind("instrument:MSFT", "instrument:AAPL", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("agent b proposal");

    // Below threshold: nothing promotes, and it is not an error.
    let promoted = queue.promote_for_agent("agent:a", READY_THRESHOLD - 0.1, NOW);
    assert!(promoted.is_empty(), "below threshold promotes nothing");
    assert_eq!(
        queue.get(&a.id).expect("a present").status,
        ValidationStatus::Validated
    );

    // At threshold: only agent:a's Validated proposals promote.
    let promoted = queue.promote_for_agent("agent:a", READY_THRESHOLD, NOW);
    assert_eq!(promoted, vec![a.id.clone()]);
    assert_eq!(
        queue.get(&a.id).expect("a present").status,
        ValidationStatus::Ready
    );
    assert_eq!(
        queue.get(&b.id).expect("b present").status,
        ValidationStatus::Validated,
        "agent b is untouched"
    );
}

#[tokio::test]
async fn approve_is_the_alternative_path_to_ready() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();
    let proposal = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("proposal");
    queue
        .approve(&proposal.id, "operator", NOW)
        .expect("approve to Ready");
    assert_eq!(
        queue.get(&proposal.id).expect("present").status,
        ValidationStatus::Ready
    );
}

#[tokio::test]
async fn reject_is_terminal() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();
    let proposal = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("proposal");
    queue
        .reject(&proposal.id, "not useful", NOW)
        .expect("reject");

    // A rejected proposal never promotes.
    let promoted = queue.promote_for_agent("agent:a", 1.0, NOW);
    assert!(promoted.is_empty(), "rejected proposals do not promote");
    // Nor does it materialize.
    let report = queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize");
    assert!(
        report.materialized.is_empty(),
        "rejected proposals do not materialize"
    );
    assert!(queue.get(&proposal.id).expect("present").rejected.is_some());
}

#[tokio::test]
async fn materialize_writes_only_ready_with_provenance() {
    let (graph, tags, raw) = engines().await;
    let mut queue = ProposalQueue::default();

    // One Ready edge (promote it) and one that stays Validated (never promoted).
    let ready_edge = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("ready edge proposal");
    let validated_edge = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:MSFT", "instrument:AAPL", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("validated edge proposal");
    // A Ready tag definition + assignment to assert the assignment provenance.
    let tag_define = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            ProposalKind::TagDefine {
                tag_id: "asset:equity".to_string(),
                parent: None,
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("tag define proposal");

    // Promote ONLY two of the three by approving each individually (the
    // validated_edge is left untouched).
    for id in [&ready_edge.id, &tag_define.id] {
        queue.approve(id, "human", NOW).expect("approve");
    }
    let report = queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize");
    assert_eq!(report.materialized.len(), 2, "only the two Ready proposals");
    assert!(!report.materialized.contains(&validated_edge.id));

    // The materialized edge carries Provenance::Agent { gated: true }.
    let edges = raw
        .edges(Some("peer_of"), 0, 256)
        .await
        .expect("read peer_of edges");
    let written = edges
        .iter()
        .find(|edge| edge.from == "instrument:AAPL" && edge.to == "instrument:MSFT")
        .expect("the Ready edge is written");
    match &written.provenance {
        Provenance::Agent { agent_id, gated } => {
            assert_eq!(agent_id, "agent:a");
            assert!(*gated, "agent edges are gated");
        }
        other => panic!("expected gated agent provenance, got {other:?}"),
    }
    // The Validated (unpromoted) edge MSFT->AAPL is NOT written.
    assert!(
        !edges
            .iter()
            .any(|edge| edge.from == "instrument:MSFT" && edge.to == "instrument:AAPL"),
        "a Validated proposal stays unwritten"
    );

    // The tag is defined, proving the tag-define proposal materialized.
    assert!(
        tags.is_defined("asset:equity").await.expect("is_defined"),
        "the Ready tag define is materialized"
    );
}

#[tokio::test]
async fn tag_assignment_provenance_string() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();
    // Define the tag through the gate, then assign it, then materialize both.
    let define = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            ProposalKind::TagDefine {
                tag_id: "asset:equity".to_string(),
                parent: None,
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("define");
    queue
        .approve(&define.id, "human", NOW)
        .expect("approve define");
    queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize define");

    let assign = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            ProposalKind::TagAssign {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
            },
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("assign");
    queue
        .approve(&assign.id, "human", NOW)
        .expect("approve assign");
    queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize assign");

    // The assignment is active (proving the provenance-stamped write landed).
    let active = tags
        .active_tags("instrument:AAPL", NOW)
        .await
        .expect("active tags");
    assert!(
        active.contains(&"asset:equity".to_string()),
        "the gated assignment is active: {active:?}"
    );
}

#[tokio::test]
async fn queue_serde_round_trip_preserves_state() {
    let (graph, tags, _raw) = engines().await;
    let mut queue = ProposalQueue::default();
    let validated = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("validated proposal");
    let ready = queue
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:MSFT", "instrument:AAPL", "peer_of"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("ready proposal");
    queue.approve(&ready.id, "human", NOW).expect("approve");

    let json = serde_json::to_string(&queue).expect("serialize queue");
    let restored: ProposalQueue = serde_json::from_str(&json).expect("deserialize queue");

    assert_eq!(
        restored
            .get(&validated.id)
            .expect("validated present")
            .status,
        ValidationStatus::Validated
    );
    assert_eq!(
        restored.get(&ready.id).expect("ready present").status,
        ValidationStatus::Ready
    );
    // The restored queue keeps its next_id: a fresh submit gets a new id, never
    // colliding with the round-tripped ones.
    let mut restored = restored;
    let fresh = restored
        .submit(
            "agent:a",
            Adaptivity::Learning,
            edge_kind("instrument:AAPL", "instrument:MSFT", "annotated_by"),
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("fresh submit after round-trip");
    assert_ne!(fresh.id, validated.id);
    assert_ne!(fresh.id, ready.id);
}
