//! Lessons-as-proposals — gated agent behavior change (knowledge-system-2 K-R1).
//!
//! A **lesson** is generalized behavioral guidance induced from agent episodes:
//! an episodic outcome where an approach failed or succeeded for a task-class
//! becomes a candidate lesson like *"for task-class X, prefer approach Y"*.
//!
//! The cardinal rule of this module is the same as the rest of the
//! knowledge-system writeback model (B9): **a lesson is NEVER a direct write.**
//! Every lesson is induced into a [`Lesson`], lowered into a
//! [`ProposalKind::Annotation`](crate::proposals::ProposalKind), and enqueued
//! through the existing [`ProposalQueue`](crate::proposals::ProposalQueue)
//! — exactly the same propose → eval → promote → materialize pipeline an agent
//! write travels. A lesson therefore:
//!
//! 1. **Proposes** via [`ProposalQueue::submit`](crate::proposals::ProposalQueue::submit)
//!    (admission gate: `Adaptivity >= Learning`; automated validators).
//! 2. **Promotes** to `Ready` ONLY through
//!    [`promote_for_agent`](crate::proposals::ProposalQueue::promote_for_agent),
//!    which is driven by a REAL out-of-sample eval and is **fail-closed**: with
//!    no eval data (`< MIN_EVAL_CASES`) a lesson is never promoted. This is the
//!    same gate K-R5 uses and the same trap the K-R5 review caught — a vacuous
//!    eval must not install a behavior change.
//! 3. **Materializes** (becomes behavior-influencing) only when its proposal is
//!    written into the graph with provenance `Agent { gated: true }`.
//!
//! **Truthful audit (`tdw.kg.why`).** A lesson is reported `Active` by
//! [`ProposalQueue::lessons_audit`](crate::proposals::ProposalQueue::lessons_audit)
//! ONLY when its backing proposal is actually materialized. A lesson whose
//! proposal is still `Validated`/`Ready` reports `Pending`; a rejected one
//! reports `Rejected`. The audit never claims an installation that did not
//! happen — the K-R5 CRITICAL.
//!
//! **Rollback.** A materialized lesson is retired by closing/invalidating its
//! annotation edge through the same temporal-close machinery used for any other
//! agent-gated fact (the proposal's annotation carries a normal `annotated_by`
//! edge). Offline/keyless installs are unaffected: with no production eval the
//! lesson simply never promotes, so behavior is unchanged.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEngine, GraphNode};
use tdw_tags::TagEngine;
use tdw_taxonomy::{Adaptivity, EntityKind};

use crate::proposals::{Proposal, ProposalKind, ProposalQueue};
use crate::{KnowledgeError, Result};

/// Graph relationship under which an episode node is linked to the lessons
/// subject.
///
/// The induction worker scans `edges(Some(EPISODE_REL), ..)` and reads the
/// episode payload from each source node's props — episodes are persisted as
/// graph nodes (the K-M2 episodic-memory convention), never minted here.
pub const EPISODE_REL: &str = "episode_of";

/// Tag/marker prefix embedded in a lesson annotation note so a lesson proposal
/// is distinguishable from any other annotation when scanning the queue.
///
/// The full note is a single line: `LESSON_NOTE_PREFIX` followed by the
/// serialized [`Lesson`] JSON. Keeping it on one structured line means the
/// existing annotation validator (non-empty, no control chars except `\n`,
/// length bound) accepts it unchanged.
pub const LESSON_NOTE_PREFIX: &str = "lesson:v1 ";

/// The synthetic graph entity every lesson annotation hangs off.
///
/// The B9 annotation validator requires the annotated entity to already exist,
/// so the induction worker upserts this node once before enqueuing lessons.
/// One shared subject keeps lessons enumerable (`neighbors` of this node) and
/// avoids minting a node per lesson.
pub const LESSON_SUBJECT_ENTITY: &str = "knowledge:lessons";

/// Minimum number of supporting episodes before a candidate lesson is induced.
///
/// A single episode is an anecdote, not a lesson; requiring corroboration keeps
/// the induction from proposing on noise. This is the induction floor — it is
/// independent of (and weaker than) the eval-promotion floor `MIN_EVAL_CASES`,
/// which still gates whether an induced lesson is ever installed.
pub const MIN_EPISODE_SUPPORT: usize = 3;

/// One agent episode/outcome — the induction input.
///
/// This is the generalized shape of an episodic memory (K-M2) or an agent eval
/// outcome: a single attempt at a task-class with a recorded approach and a
/// binary outcome. The induction groups these by `(agent_id, task_class,
/// approach)` to find approaches that reliably succeed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    /// Stable id of the underlying episodic memory / eval run (provenance).
    pub episode_id: String,
    /// The agent that produced this episode.
    pub agent_id: String,
    /// The task-class this episode belongs to (the lesson's trigger).
    pub task_class: String,
    /// The approach the agent took on this episode.
    pub approach: String,
    /// Whether the approach succeeded.
    pub succeeded: bool,
}

/// A generalized, induced behavioral lesson.
///
/// Carries everything the writeback model and the audit trail need: the
/// trigger (`task_class`), the recommended behavior change
/// (`recommended_behavior`), the supporting episode ids (`evidence`), a
/// `confidence` in `[0, 1]`, and the `agent_id` whose eval gate will decide
/// promotion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lesson {
    /// The agent this lesson is for (its eval gate governs promotion).
    pub agent_id: String,
    /// The trigger: when the agent faces this task-class, the lesson applies.
    pub task_class: String,
    /// The recommended behavior change (e.g. `"prefer approach Y"`).
    pub recommended_behavior: String,
    /// Episode ids that support this lesson (provenance — the evidence).
    pub evidence: Vec<String>,
    /// Confidence in `[0, 1]` — the observed success rate of the recommended
    /// approach across the supporting episodes.
    pub confidence: f64,
}

impl Lesson {
    /// Lower this lesson into a gated [`ProposalKind::Annotation`].
    ///
    /// The lesson is serialized into a single structured note line and attached
    /// to [`LESSON_SUBJECT_ENTITY`]. Routing through the annotation proposal
    /// kind means the lesson travels the FULL B9 gate — there is deliberately no
    /// bespoke "lesson" proposal kind that could bypass the existing validators
    /// or the eval-driven promotion.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] when the lesson cannot be serialized.
    pub fn to_proposal_kind(&self) -> Result<ProposalKind> {
        let body = serde_json::to_string(self).map_err(|error| {
            KnowledgeError::Storage(format!("lesson serialize failed: {error}"))
        })?;
        Ok(ProposalKind::Annotation {
            entity_id: LESSON_SUBJECT_ENTITY.to_string(),
            note: format!("{LESSON_NOTE_PREFIX}{body}"),
        })
    }

    /// True when an annotation note carries the lesson marker.
    #[must_use]
    pub fn note_is_lesson(note: &str) -> bool {
        note.starts_with(LESSON_NOTE_PREFIX)
    }

    /// Parse a lesson back out of a marked annotation note.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] when the note is not a lesson note
    /// or does not deserialize.
    pub fn from_note(note: &str) -> Result<Self> {
        let body = note.strip_prefix(LESSON_NOTE_PREFIX).ok_or_else(|| {
            KnowledgeError::Storage("annotation note is not a lesson note".to_string())
        })?;
        serde_json::from_str(body)
            .map_err(|error| KnowledgeError::Storage(format!("lesson parse failed: {error}")))
    }
}

/// Deterministically induce candidate lessons from a batch of episodes.
///
/// Episodes are grouped by `(agent_id, task_class, approach)`. For each group
/// with at least [`MIN_EPISODE_SUPPORT`] episodes whose success rate is at
/// least `min_confidence`, one [`Lesson`] is emitted recommending that approach
/// for that task-class, with `confidence` = success rate and `evidence` = the
/// supporting episode ids.
///
/// Pure and deterministic: no clock, no randomness, output ordered by
/// `(agent_id, task_class, approach)` so repeated runs are byte-identical.
/// Induction does NOT write anything — it only produces candidates the caller
/// then enqueues as proposals.
#[must_use]
pub fn induce_lessons(episodes: &[Episode], min_confidence: f64) -> Vec<Lesson> {
    use std::collections::BTreeMap;

    // Group key is (agent_id, task_class, approach); value accumulates the
    // supporting episode ids and the success count.
    let mut groups: BTreeMap<(&str, &str, &str), (Vec<String>, usize)> = BTreeMap::new();
    for episode in episodes {
        let entry = groups
            .entry((
                episode.agent_id.as_str(),
                episode.task_class.as_str(),
                episode.approach.as_str(),
            ))
            .or_default();
        entry.0.push(episode.episode_id.clone());
        if episode.succeeded {
            entry.1 += 1;
        }
    }

    let mut lessons = Vec::new();
    for ((agent_id, task_class, approach), (evidence, successes)) in groups {
        let support = evidence.len();
        if support < MIN_EPISODE_SUPPORT {
            continue;
        }
        // support > 0 here (>= MIN_EPISODE_SUPPORT >= 1), so the division is safe.
        #[allow(clippy::cast_precision_loss)]
        let confidence = successes as f64 / support as f64;
        if confidence < min_confidence {
            continue;
        }
        lessons.push(Lesson {
            agent_id: agent_id.to_string(),
            task_class: task_class.to_string(),
            recommended_behavior: format!("prefer approach {approach}"),
            evidence,
            confidence,
        });
    }
    lessons
}

/// Truthful lifecycle state of a lesson, as reported by `tdw.kg.why`.
///
/// The state is derived ONLY from the backing proposal's real status — never
/// asserted optimistically. A lesson is `Active` (behavior-influencing) iff its
/// proposal was actually materialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonState {
    /// Enqueued but not yet materialized (still `Draft`/`Validated`/`Ready`).
    /// Not behavior-influencing.
    Pending,
    /// Proposal materialized — the lesson is installed and behavior-influencing.
    Active,
    /// Proposal rejected (validators, eval never passed, or operator) — the
    /// lesson is NOT installed.
    Rejected,
    /// The lesson was active and has since been retired/invalidated (rollback).
    Retired,
}

/// A truthful audit record for one lesson, for `tdw.kg.why`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LessonAudit {
    /// The proposal id backing this lesson.
    pub proposal_id: String,
    /// The lesson payload.
    pub lesson: Lesson,
    /// Truthful lifecycle state derived from the proposal.
    pub state: LessonState,
}

/// Counts of lessons by lifecycle state, surfaced additively in
/// `tdw.kg.status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LessonCounts {
    /// Lessons enqueued but not yet materialized (not behavior-influencing).
    pub pending: usize,
    /// Lessons whose proposal materialized (installed, behavior-influencing).
    pub active: usize,
}

/// Outcome of one induction pass, for logging/observability.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InductionReport {
    /// Episodes read from the graph this pass.
    pub episodes_read: usize,
    /// Candidate lessons induced this pass.
    pub lessons_induced: usize,
    /// Lessons accepted as new proposals (passed admission + validators).
    pub lessons_enqueued: usize,
    /// Lessons the gate rejected at enqueue (e.g. duplicate of an already-
    /// pending lesson) — NOT an error; counted so operators see the gate working.
    pub lessons_rejected: usize,
}

/// Read agent episodes persisted in the graph (the induction input).
///
/// Episodes are nodes whose `props` carry `{agent_id, task_class, approach,
/// succeeded}` and which link to [`LESSON_SUBJECT_ENTITY`] via an
/// [`EPISODE_REL`] edge. This is read-only: it never writes, mirroring the
/// "induction does not write" contract. A node missing any required prop is
/// skipped (defensive — a malformed episode is ignored, not fatal).
///
/// `max_scan` bounds the edge scan so a pathological graph cannot stall the
/// worker.
///
/// # Errors
///
/// Returns [`KnowledgeError::Storage`] only on an engine error; missing/
/// malformed episode nodes are skipped silently.
pub async fn read_episodes_from_graph(
    graph: &Arc<dyn GraphEngine>,
    max_scan: usize,
) -> Result<Vec<Episode>> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let mut episodes = Vec::new();
    let mut offset = 0usize;
    let page = 256usize;
    while offset < max_scan {
        let edges = graph
            .edges(Some(EPISODE_REL), offset, page)
            .await
            .map_err(storage)?;
        if edges.is_empty() {
            break;
        }
        offset += edges.len();
        for edge in edges {
            // Only edges INTO the lessons subject are episode links.
            if edge.to != LESSON_SUBJECT_ENTITY {
                continue;
            }
            let Some(node) = graph.node(&edge.from).await.map_err(storage)? else {
                continue;
            };
            if let Some(episode) = episode_from_node(&node) {
                episodes.push(episode);
            }
        }
    }
    Ok(episodes)
}

/// Parse an [`Episode`] from a graph node's props; `None` when any field is
/// missing or the wrong type.
fn episode_from_node(node: &GraphNode) -> Option<Episode> {
    let props = node.props.as_object()?;
    Some(Episode {
        episode_id: node.id.clone(),
        agent_id: props.get("agent_id")?.as_str()?.to_string(),
        task_class: props.get("task_class")?.as_str()?.to_string(),
        approach: props.get("approach")?.as_str()?.to_string(),
        succeeded: props.get("succeeded")?.as_bool()?,
    })
}

/// Ensure the shared [`LESSON_SUBJECT_ENTITY`] node exists so the annotation
/// validator (which requires the annotated entity to exist) accepts lesson
/// proposals. Idempotent: upserts a stable node.
///
/// # Errors
///
/// Returns [`KnowledgeError::Storage`] on an engine error.
pub async fn ensure_lesson_subject(graph: &Arc<dyn GraphEngine>) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    if graph
        .node(LESSON_SUBJECT_ENTITY)
        .await
        .map_err(storage)?
        .is_some()
    {
        return Ok(());
    }
    graph
        .upsert_nodes(vec![GraphNode {
            id: LESSON_SUBJECT_ENTITY.to_string(),
            kind: EntityKind::Document,
            label: "agent lessons".to_string(),
            aliases: Vec::new(),
            props: serde_json::json!({ "synthetic": true, "purpose": "K-R1 lessons subject" }),
            valid_from: None,
            valid_to: None,
        }])
        .await
        .map_err(storage)
}

/// Submit a batch of induced lessons through the FULL B9 gate.
///
/// Each lesson is lowered to a [`ProposalKind::Annotation`] and submitted via
/// [`ProposalQueue::submit`] at the lesson's own `agent_id` and the supplied
/// `adaptivity`. Submission can fail (admission below `Learning`, duplicate of
/// an already-pending lesson, queue full) — those are NOT errors: they are the
/// gate working, counted in [`InductionReport::lessons_rejected`]. The lesson is
/// never written directly; promotion remains the agent's eval gate
/// (`promote_for_agent`), fail-closed on no eval data.
///
/// # Errors
///
/// Returns [`KnowledgeError`] only on an engine-level failure inside the
/// validators (not on a gate rejection, which is recorded in the report).
pub async fn submit_lessons(
    queue: &mut ProposalQueue,
    lessons: &[Lesson],
    adaptivity: Adaptivity,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
    now: &str,
) -> InductionReport {
    let mut report = InductionReport {
        lessons_induced: lessons.len(),
        ..InductionReport::default()
    };
    for lesson in lessons {
        let Ok(kind) = lesson.to_proposal_kind() else {
            report.lessons_rejected += 1;
            continue;
        };
        match queue
            .submit(&lesson.agent_id, adaptivity, kind, graph, tags, now)
            .await
        {
            Ok(_) => report.lessons_enqueued += 1,
            // A gate rejection (duplicate / cap / admission) is the gate
            // working as designed — count it, never panic or direct-write.
            Err(_) => report.lessons_rejected += 1,
        }
    }
    report
}

/// Convenience: one full induction pass — read episodes, induce, ensure the
/// subject node, and submit. Returns the [`InductionReport`].
///
/// This is the body the production worker calls on each cron tick. It performs
/// NO direct writes: episodes are read-only, the subject node is an idempotent
/// upsert, and every lesson goes through [`submit_lessons`] (the B9 gate).
///
/// # Errors
///
/// Returns [`KnowledgeError`] on an engine error while reading episodes or
/// ensuring the subject node.
pub async fn run_induction_pass(
    queue: &mut ProposalQueue,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
    adaptivity: Adaptivity,
    min_confidence: f64,
    max_scan: usize,
    now: &str,
) -> Result<InductionReport> {
    let episodes = read_episodes_from_graph(graph, max_scan).await?;
    let lessons = induce_lessons(&episodes, min_confidence);
    if lessons.is_empty() {
        return Ok(InductionReport {
            episodes_read: episodes.len(),
            ..InductionReport::default()
        });
    }
    ensure_lesson_subject(graph).await?;
    let mut report = submit_lessons(queue, &lessons, adaptivity, graph, tags, now).await;
    report.episodes_read = episodes.len();
    Ok(report)
}

/// True when a proposal is a lesson proposal (annotation carrying the marker).
#[must_use]
pub fn proposal_is_lesson(proposal: &Proposal) -> bool {
    matches!(&proposal.kind, ProposalKind::Annotation { note, .. } if Lesson::note_is_lesson(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(id: &str, agent: &str, class: &str, approach: &str, ok: bool) -> Episode {
        Episode {
            episode_id: id.to_string(),
            agent_id: agent.to_string(),
            task_class: class.to_string(),
            approach: approach.to_string(),
            succeeded: ok,
        }
    }

    #[test]
    fn induction_requires_minimum_support() {
        // Two episodes is below MIN_EPISODE_SUPPORT (3) — no lesson.
        let episodes = vec![
            ep("e1", "agent:a", "screen", "momentum", true),
            ep("e2", "agent:a", "screen", "momentum", true),
        ];
        assert!(induce_lessons(&episodes, 0.6).is_empty());
    }

    #[test]
    fn induction_emits_lesson_with_evidence_and_confidence() {
        let episodes = vec![
            ep("e1", "agent:a", "screen", "momentum", true),
            ep("e2", "agent:a", "screen", "momentum", true),
            ep("e3", "agent:a", "screen", "momentum", false),
        ];
        let lessons = induce_lessons(&episodes, 0.6);
        assert_eq!(lessons.len(), 1);
        let lesson = &lessons[0];
        assert_eq!(lesson.agent_id, "agent:a");
        assert_eq!(lesson.task_class, "screen");
        assert_eq!(lesson.recommended_behavior, "prefer approach momentum");
        assert_eq!(lesson.evidence, vec!["e1", "e2", "e3"]);
        assert!((lesson.confidence - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn induction_drops_low_confidence_groups() {
        // 1/3 success rate < 0.6 min_confidence → dropped despite enough support.
        let episodes = vec![
            ep("e1", "agent:a", "screen", "random", true),
            ep("e2", "agent:a", "screen", "random", false),
            ep("e3", "agent:a", "screen", "random", false),
        ];
        assert!(induce_lessons(&episodes, 0.6).is_empty());
    }

    #[test]
    fn induction_is_deterministic_and_ordered() {
        let episodes = vec![
            ep("z1", "agent:b", "value", "dcf", true),
            ep("z2", "agent:b", "value", "dcf", true),
            ep("z3", "agent:b", "value", "dcf", true),
            ep("a1", "agent:a", "screen", "momentum", true),
            ep("a2", "agent:a", "screen", "momentum", true),
            ep("a3", "agent:a", "screen", "momentum", true),
        ];
        let lessons = induce_lessons(&episodes, 0.5);
        assert_eq!(lessons.len(), 2);
        // BTreeMap ordering: agent:a before agent:b.
        assert_eq!(lessons[0].agent_id, "agent:a");
        assert_eq!(lessons[1].agent_id, "agent:b");
        assert_eq!(induce_lessons(&episodes, 0.5), lessons);
    }

    #[test]
    fn lesson_round_trips_through_annotation_note() {
        let lesson = Lesson {
            agent_id: "agent:a".to_string(),
            task_class: "screen".to_string(),
            recommended_behavior: "prefer approach momentum".to_string(),
            evidence: vec!["e1".to_string(), "e2".to_string(), "e3".to_string()],
            confidence: 2.0 / 3.0,
        };
        let kind = lesson.to_proposal_kind().expect("lower to annotation");
        let ProposalKind::Annotation { entity_id, note } = kind else {
            panic!("lesson must lower to an Annotation proposal, never a direct write");
        };
        assert_eq!(entity_id, LESSON_SUBJECT_ENTITY);
        assert!(Lesson::note_is_lesson(&note));
        let parsed = Lesson::from_note(&note).expect("parse lesson back");
        assert_eq!(parsed, lesson);
    }

    #[test]
    fn from_note_rejects_non_lesson_notes() {
        assert!(Lesson::from_note("just a plain annotation").is_err());
    }
}
