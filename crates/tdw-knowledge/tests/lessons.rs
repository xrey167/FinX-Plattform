//! Tests for lessons-as-proposals — gated agent behavior change (K-R1).
//!
//! The contract under test, end to end over fully in-memory engines:
//!
//! (a) A lesson induced from agent episodes ENQUEUES as a gated proposal — it is
//!     NOT a direct write into the graph. Until materialization, no lesson
//!     annotation exists in the substrate.
//! (b) With NO eval data the lesson is NOT promoted (fail-closed): a vacuous /
//!     under-floor eval never grants promotion (the K-R5 CRITICAL).
//! (c) With passing eval data (>= MIN_EVAL_CASES at/above the threshold) the
//!     lesson promotes, materializes, and `tdw.kg.why`-style audit reports it
//!     truthfully as `Active` — only after it actually landed.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, MergeDecision, MergeReport, Provenance, Result, Subgraph,
    TraversalFilter,
};
use tdw_knowledge::lessons::{
    EPISODE_REL, Episode, LESSON_SUBJECT_ENTITY, Lesson, LessonState, induce_lessons,
    run_induction_pass,
};
use tdw_knowledge::proposals::{MIN_EVAL_CASES, ProposalQueue};
use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

const NOW: &str = "2026-05-22";

/// A shareable [`GraphEngine`] handle: one in-memory graph behind an `Arc`,
/// delegating the whole trait (mirrors the B9 proposals test harness).
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

/// One episode node: id + props {agent_id, task_class, approach, succeeded}.
fn episode_node(id: &str, agent: &str, class: &str, approach: &str, ok: bool) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: tdw_taxonomy::EntityKind::Document,
        label: id.to_string(),
        aliases: Vec::new(),
        props: json!({
            "agent_id": agent,
            "task_class": class,
            "approach": approach,
            "succeeded": ok,
        }),
        valid_from: None,
        valid_to: None,
    }
}

/// A bare subject node (so the EPISODE_REL edges have an existing endpoint).
fn subject_stub() -> GraphNode {
    GraphNode {
        id: LESSON_SUBJECT_ENTITY.to_string(),
        kind: tdw_taxonomy::EntityKind::Document,
        label: "lessons".to_string(),
        aliases: Vec::new(),
        props: Value::Null,
        valid_from: None,
        valid_to: None,
    }
}

/// Build an in-memory graph seeded with `n` episodes for `(agent, class,
/// approach)` (all succeeding) plus the subject node and their episode edges.
async fn graph_with_episodes(
    agent: &str,
    class: &str,
    approach: &str,
    n: usize,
) -> (
    Arc<dyn GraphEngine>,
    Arc<dyn TagEngine>,
    Arc<InMemoryGraphEngine>,
) {
    let raw = Arc::new(InMemoryGraphEngine::default());
    let mut nodes = vec![subject_stub()];
    let mut edges = Vec::new();
    for i in 0..n {
        let id = format!("episode-{i}");
        nodes.push(episode_node(&id, agent, class, approach, true));
        edges.push(GraphEdge {
            from: id,
            to: LESSON_SUBJECT_ENTITY.to_string(),
            rel: EPISODE_REL.to_string(),
            props: Value::Null,
            provenance: Provenance::System {
                detail: "test-episode".to_string(),
            },
            valid_from: None,
            valid_to: None,
        });
    }
    raw.upsert_nodes(nodes)
        .await
        .unwrap_or_else(|e| panic!("seed episode nodes: {e}"));
    raw.upsert_edges(edges)
        .await
        .unwrap_or_else(|e| panic!("seed episode edges: {e}"));
    let graph: Arc<dyn GraphEngine> = Arc::new(SharedGraph(raw.clone()));
    let tags: Arc<dyn TagEngine> = Arc::new(GraphTagEngine::new(SharedGraph(raw.clone())));
    (graph, tags, raw)
}

/// Count annotation nodes attached to the lessons subject (i.e. MATERIALIZED
/// lessons in the graph). Before any materialization this MUST be zero — proving
/// induction did not direct-write.
async fn materialized_lesson_annotations(raw: &Arc<InMemoryGraphEngine>) -> usize {
    let mut count = 0;
    let mut offset = 0;
    loop {
        let edges = raw
            .edges(Some("annotated_by"), offset, 256)
            .await
            .unwrap_or_else(|e| panic!("scan annotated_by: {e}"));
        if edges.is_empty() {
            break;
        }
        offset += edges.len();
        count += edges
            .iter()
            .filter(|e| e.from == LESSON_SUBJECT_ENTITY)
            .count();
    }
    count
}

#[tokio::test]
async fn lesson_enqueues_as_proposal_not_a_direct_write() {
    // (a) Induction reads episodes and ENQUEUES a lesson proposal. Nothing is
    // written into the graph as a lesson annotation — the only landing path is
    // materialization, which requires promotion first.
    let agent = "agent:a";
    let (graph, tags, raw) = graph_with_episodes(agent, "screen", "momentum", 4).await;
    let mut queue = ProposalQueue::default();

    let report = run_induction_pass(
        &mut queue,
        &graph,
        &tags,
        Adaptivity::Learning,
        0.6,
        10_000,
        NOW,
    )
    .await
    .expect("induction pass runs");

    // One lesson induced and enqueued as a PROPOSAL.
    assert_eq!(report.episodes_read, 4);
    assert_eq!(report.lessons_enqueued, 1);

    // The proposal exists in the queue but is NOT materialized…
    let audit = queue.lessons_audit();
    assert_eq!(audit.len(), 1, "exactly one lesson proposal enqueued");
    assert_eq!(audit[0].state, LessonState::Pending);
    assert_eq!(audit[0].lesson.agent_id, agent);
    assert_eq!(audit[0].lesson.evidence.len(), 4);

    // …and crucially, NO lesson annotation has been written into the graph.
    assert_eq!(
        materialized_lesson_annotations(&raw).await,
        0,
        "induction must NEVER direct-write a lesson into the substrate"
    );
    // Status counts report it as pending, not active.
    let counts = queue.lesson_counts();
    assert_eq!(counts.pending, 1);
    assert_eq!(counts.active, 0);
}

#[tokio::test]
async fn no_eval_data_does_not_promote_fail_closed() {
    // (b) FAIL-CLOSED: with no eval data (zero cases — a vacuous "perfect" eval)
    // the lesson is NEVER promoted. This is the K-R5 CRITICAL the gate must hold.
    let agent = "agent:a";
    let (graph, tags, raw) = graph_with_episodes(agent, "screen", "momentum", 4).await;
    let mut queue = ProposalQueue::default();
    run_induction_pass(
        &mut queue,
        &graph,
        &tags,
        Adaptivity::Learning,
        0.6,
        10_000,
        NOW,
    )
    .await
    .expect("induction pass runs");

    // Attempt promotion with ZERO eval cases and a vacuously perfect pass_rate.
    let promoted = queue.promote_for_agent(agent, 1.0, 0, NOW);
    assert!(
        promoted.is_empty(),
        "no eval data (0 cases) must NOT promote — fail-closed"
    );

    // Even just under the floor must not promote.
    let promoted = queue.promote_for_agent(agent, 1.0, MIN_EVAL_CASES - 1, NOW);
    assert!(
        promoted.is_empty(),
        "below MIN_EVAL_CASES must NOT promote — fail-closed"
    );

    // Materialization of a non-Ready queue writes nothing.
    queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize sweep runs");
    assert_eq!(
        materialized_lesson_annotations(&raw).await,
        0,
        "an un-promoted lesson must never materialize"
    );

    // Audit truthfully reports Pending, never Active.
    let audit = queue.lessons_audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].state, LessonState::Pending);
    assert_eq!(queue.lesson_counts().active, 0);
}

#[tokio::test]
async fn passing_eval_promotes_and_why_reports_truthfully() {
    // (c) With passing eval data (>= MIN_EVAL_CASES at the threshold) the lesson
    // promotes, materializes, and the audit reports it as Active ONLY after it
    // actually landed.
    let agent = "agent:a";
    let (graph, tags, raw) = graph_with_episodes(agent, "screen", "momentum", 4).await;
    let mut queue = ProposalQueue::default();
    run_induction_pass(
        &mut queue,
        &graph,
        &tags,
        Adaptivity::Learning,
        0.6,
        10_000,
        NOW,
    )
    .await
    .expect("induction pass runs");

    // Real, passing eval evidence: pass_rate 1.0 over MIN_EVAL_CASES cases.
    let promoted = queue.promote_for_agent(agent, 1.0, MIN_EVAL_CASES, NOW);
    assert_eq!(promoted.len(), 1, "a passing eval promotes the lesson");

    // Before materialization the audit must still NOT claim Active.
    assert_eq!(
        queue.lessons_audit()[0].state,
        LessonState::Pending,
        "promoted-but-not-materialized is still Pending — never claim a landing that did not happen"
    );

    // Materialize the Ready lesson.
    let report = queue
        .materialize_ready(&graph, &tags, NOW)
        .await
        .expect("materialize sweep runs");
    assert_eq!(report.materialized.len(), 1);

    // Now — and only now — the lesson is Active, and a real annotation exists.
    let audit = queue.lessons_audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].state, LessonState::Active);
    assert_eq!(queue.lesson_counts().active, 1);
    assert_eq!(queue.lesson_counts().pending, 0);
    assert_eq!(
        materialized_lesson_annotations(&raw).await,
        1,
        "the promoted lesson is now installed in the substrate"
    );

    // Provenance audit: the materialized proposal carries Agent{gated:true}.
    let proposal = queue.get(&audit[0].proposal_id).expect("proposal present");
    assert!(proposal.materialized);
    assert!(proposal.history.iter().any(|h| h.contains("materialized")));
}

#[tokio::test]
async fn induction_under_support_proposes_nothing() {
    // A single anecdote (below MIN_EPISODE_SUPPORT) induces no lesson, so nothing
    // enqueues — the gate never sees noise.
    let (graph, tags, _raw) = graph_with_episodes("agent:a", "screen", "momentum", 1).await;
    let mut queue = ProposalQueue::default();
    let report = run_induction_pass(
        &mut queue,
        &graph,
        &tags,
        Adaptivity::Learning,
        0.6,
        10_000,
        NOW,
    )
    .await
    .expect("induction pass runs");
    assert_eq!(report.episodes_read, 1);
    assert_eq!(report.lessons_enqueued, 0);
    assert!(queue.lessons_audit().is_empty());
}

#[tokio::test]
async fn pure_induction_is_deterministic() {
    // The induction function itself is pure: same episodes → same lessons.
    let episodes: Vec<Episode> = (0..3)
        .map(|i| Episode {
            episode_id: format!("e{i}"),
            agent_id: "agent:a".to_string(),
            task_class: "screen".to_string(),
            approach: "momentum".to_string(),
            succeeded: true,
        })
        .collect();
    let a = induce_lessons(&episodes, 0.6);
    let b = induce_lessons(&episodes, 0.6);
    assert_eq!(a, b);
    assert_eq!(a.len(), 1);
    let lesson: &Lesson = &a[0];
    assert!((lesson.confidence - 1.0).abs() < 1e-9);
}
