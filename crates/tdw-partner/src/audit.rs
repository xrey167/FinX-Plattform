//! The audit & undo surface — "fully autonomous, audit-only" (the partner design
//! §4, partner-system W5).
//!
//! Per directive 2 the partner acts autonomously *inside the gates* and the human
//! reviews **after the fact**, with one-gesture undo/correct that feeds learning —
//! **not** a pre-approval inbox. This module is the audit/undo surface that makes
//! that safe. It is, by design, a **projection** over records that ALREADY exist
//! — the gated [`Proposal`] history, the [`TuneRecord`] self-tune log, and the
//! [`LessonAudit`] trail — exactly as [`crate::build_brief`] is a pure assembler
//! over [`crate::BriefInputs`]. It introduces **no new system of record**: the
//! adapter gathers the existing records (the I/O lane) and [`audit_feed`] renders
//! them; the safety net is the *already-reversible* governed-forgetting /
//! cold-plane machinery, reused here, never re-implemented.
//!
//! - [`ActionRecord`] — one row in the "what I did + why" stream (the partner
//!   design §4.2). Every record carries a [`Provenance`] `why` (W5.1).
//! - [`AuditInputs`] — the already-gathered source records (the I/O lane), so
//!   [`audit_feed`] stays pure and golden-testable.
//! - [`audit_feed`] — the projection: fan the source records into one ranked
//!   [`ActionRecord`] stream, applying the auto-accept-within-gates default and
//!   the escalation predicate (W5.1 + W5.2).
//! - [`EscalationConfig`] + [`escalation_status`] — the explicit predicate that
//!   routes low-confidence / high-impact / irreversible actions to
//!   [`ActionStatus::AwaitingHuman`] (W5.2); everything else auto-accepts.
//! - [`undo`] — one-gesture reverse, reusing
//!   [`tdw_knowledge::forgetting::recall_cold_edge`] (undo a `Forget`) /
//!   [`tdw_knowledge::forgetting::retire_edge_to_cold`] (inverse-`Forget` an
//!   added edge) (W5.3).
//! - [`correct`] — undo **+** a `tdw.kg.feedback` signal, so the correction
//!   *trains* the system: it lowers trust for the action's source and records a
//!   `Lesson` (W5.4). The feedback I/O is the adapter's; this module produces the
//!   [`Correction`] the adapter routes through `tdw.kg.feedback`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::GraphEngine;
use tdw_knowledge::forgetting::{RetireOutcome, recall_cold_edge, retire_edge_to_cold};
use tdw_knowledge::lessons::{LessonAudit, LessonState};
use tdw_knowledge::proposals::{Proposal, ProposalKind};
use tdw_knowledge::self_tune::{TuneOutcome, TuneRecord};

use crate::principal::{Principal, Provenance};

/// What an autonomous action did (the partner design §4.2 `ActionKind`).
///
/// Each variant is *projected* from an existing record — never a new authored
/// type. The variant carries exactly what undo/correct need to reverse it, so
/// the audit row is self-describing without a second lookup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    /// A knowledge-graph EDGE write (a `ProposalKind::Edge`). Reversible by an
    /// inverse `Forget` — [`retire_edge_to_cold`] retires the added edge to cold,
    /// the same cold-plane move that retires any fact.
    Edge {
        /// The added edge's `from` node.
        from: String,
        /// The added edge's relationship type.
        rel: String,
        /// The added edge's `to` node.
        to: String,
    },
    /// A non-edge knowledge-graph write (tag assign / tag define / annotation).
    ///
    /// The `reversal` records HOW this write is undone, so the audit row is
    /// self-describing and [`undo`] performs a REAL reversal (never a silent
    /// no-op). An annotation write is retired via its `annotated_by` cold-plane
    /// edge; a tag assign/define has no reversal primitive in the tag-engine
    /// contract today and is flagged [`KgWriteReversal::Unsupported`] so [`undo`]
    /// returns an explicit error rather than a vacuous success.
    KgWrite {
        /// The backing proposal id (`p<seq>`).
        proposal_id: String,
        /// How this write is reversed (the self-describing undo plan).
        reversal: KgWriteReversal,
    },
    /// A governed-forgetting decision (a `Forget` [`Proposal`]). Reversible by
    /// [`recall_cold_edge`] — the cold-plane recall.
    Forget {
        /// The backing proposal id.
        proposal_id: String,
        /// The retired edge's `from` node.
        from: String,
        /// The retired edge's relationship type.
        rel: String,
        /// The retired edge's `to` node.
        to: String,
    },
    /// A self-tune parameter change (a [`TuneRecord`]). Reversible by restoring
    /// the parameter to its PRIOR value — carried here so the reversal is real
    /// and self-describing (the partner design §4.2: a param tune undo is a
    /// re-tune back to `restore_value`).
    ParamTune {
        /// The backing tune-record id (`t<seq>`).
        record_id: String,
        /// The parameter that was tuned.
        param: String,
        /// The value the tune CHANGED the parameter to (the applied value).
        applied_value: f64,
        /// The value to restore on undo — the parameter's value BEFORE the tune.
        /// Reversing the tune sets the live parameter back to this.
        restore_value: f64,
    },
    /// A promoted lesson/rule (a materialized [`LessonAudit`] = a gated
    /// [`ProposalKind::Annotation`] on the lessons subject). Reversed by retiring
    /// the lesson's `annotated_by` edge to the cold plane — the SAME cold-plane
    /// move that retires any fact — which demotes the rule out of active behavior
    /// while preserving the historical record.
    RuledPromote {
        /// The backing proposal id of the promoted lesson.
        proposal_id: String,
        /// The entity the lesson annotation is attached to (the lessons subject).
        subject_entity: String,
        /// The annotation node id the promote created (`annotation:<proposal>`),
        /// the `to` end of the `annotated_by` edge the undo retires.
        annotation_node: String,
    },
}

/// How a [`ActionKind::KgWrite`] is reversed — the self-describing undo plan
/// (the partner design §4.2, W5.3).
///
/// A `KgWrite` covers the three non-edge proposal kinds. An annotation write
/// adds an `annotated_by` edge that the cold-plane machinery can retire, so it
/// is genuinely reversible; a tag assign/define has NO reversal primitive in the
/// [`tdw_tags::TagEngine`] contract, so it is flagged unsupported and [`undo`]
/// returns an explicit error rather than a silent success (the Gemini #441 HIGH
/// fix: never a vacuous `Ok`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reversal", rename_all = "snake_case")]
pub enum KgWriteReversal {
    /// An annotation write: reverse by retiring the `annotated_by` edge from
    /// `entity_id` to `annotation_node` to the cold plane.
    Annotation {
        /// The entity the annotation was attached to (the `from` of the edge).
        entity_id: String,
        /// The annotation node id (`annotation:<proposal>`), the edge's `to`.
        annotation_node: String,
    },
    /// A tag assign/define: no reversal primitive exists in the tag-engine
    /// contract today, so an undo is explicitly UNSUPPORTED (never a silent Ok).
    Unsupported {
        /// Human-readable reason the write cannot be auto-reversed.
        detail: String,
    },
}

impl ActionKind {
    /// Whether the action has a defined, reversible undo by construction
    /// (the partner design §4.2 `reversible`).
    ///
    /// An edge write (inverse `Forget`), a `Forget` (`recall_cold_edge`), a param
    /// tune (restore the prior value), and a promoted lesson (retire its
    /// `annotated_by` edge) are all reversible. A [`ActionKind::KgWrite`] is
    /// reversible IFF its plan is not [`KgWriteReversal::Unsupported`] — a tag
    /// assign/define has no reversal primitive, so it is flagged irreversible and
    /// the escalation predicate routes it to a human (never a silent `Ok`).
    #[must_use]
    pub const fn reversible(&self) -> bool {
        match self {
            Self::Edge { .. }
            | Self::Forget { .. }
            | Self::ParamTune { .. }
            | Self::RuledPromote { .. } => true,
            Self::KgWrite { reversal, .. } => {
                !matches!(reversal, KgWriteReversal::Unsupported { .. })
            }
        }
    }
}

/// The lifecycle status of an audited action (the partner design §4.2
/// `ActionStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    /// Acted autonomously within the gates — the default posture. The human
    /// reviews after the fact.
    AutoAccepted,
    /// Escalated to wait-for-human: low-confidence / high-impact / irreversible
    /// (the partner design §4.3). Lands in the SAME feed, not a separate inbox.
    AwaitingHuman,
    /// The action was reversed via [`undo`].
    Undone,
    /// The action was reversed AND fed back via [`correct`] (it trained the loop).
    Corrected,
}

/// One row in the audit stream: what the partner did + why (the partner design
/// §4.2 `ActionRecord`).
///
/// A projection — every field is read from an existing record. The `why` is the
/// [`Provenance`] chain the design demands for *every* action (W5.1): the routes
/// / KG nodes / audit reason that produced it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Stable id (the backing record's id, namespaced by kind).
    pub id: String,
    /// The agent that took the action (from the backing record).
    pub agent_id: String,
    /// What the action did (and how to reverse it).
    pub what: ActionKind,
    /// WHY it happened: the provenance chain (the `tdw.kg.why` reason + the
    /// routes / KG nodes / feedback that triggered it). Never empty — the W5.1
    /// "renders a why for every action" gate.
    pub why: Provenance,
    /// Confidence in `[0, 1]` the action was taken with (drives escalation).
    pub confidence: f64,
    /// Whether the action is reversible by construction.
    pub reversible: bool,
    /// The lifecycle status (auto-accepted by default; escalated when flagged).
    pub status: ActionStatus,
}

/// The already-gathered source records for one audit feed (the partner design
/// §4.2 — the projection's inputs).
///
/// Collected by the adapter (the I/O lane): `proposals` ← the gated
/// [`tdw_knowledge::proposals::ProposalQueue`] history (incl. the `tdw.kg.\
/// proposals` / `tdw.kg.dismiss` slice, folded in here per §4.5); `tunes` ← the
/// [`tdw_knowledge::self_tune::SelfTuneLog`]; `lessons` ← the [`LessonAudit`]
/// trail. [`audit_feed`] is pure over this, so it is deterministic and
/// golden-testable, and `tdw-partner` stays a leaf (it never touches a store).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditInputs {
    /// Gated proposals (KG writes + `Forget`s), each projected to one record.
    #[serde(default)]
    pub proposals: Vec<Proposal>,
    /// Self-tune decisions, each APPLIED record projected to a `ParamTune`.
    #[serde(default)]
    pub tunes: Vec<TuneRecord>,
    /// Lesson audits; each MATERIALIZED (`Active`) lesson is a `RuledPromote`.
    #[serde(default)]
    pub lessons: Vec<LessonAudit>,
}

/// The escalation predicate config (the partner design §4.3).
///
/// Read from the trust-dial + config by the surface. The defaults encode the
/// design's posture: auto-accept-within-gates is the norm, and only the explicit
/// flagged set escalates.
///
/// # Partial-config defaults (Gemini #441)
///
/// `#[serde(default)]` at the container level plus a per-field
/// `#[serde(default = ...)]` on `confidence_floor` mean a PARTIAL `config` JSON
/// — e.g. `{"high_impact_entities":[...]}` — deserializes with the sane default
/// floor instead of zeroing it out (which would make every action
/// "high-confidence" and silently disable the low-confidence escalation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EscalationConfig {
    /// Confidence below this escalates as "low-confidence" (the partner design
    /// §4.3). The default `0.5` treats a coin-flip-or-worse action as needing a
    /// human; a routine high-confidence act auto-accepts. A `config` JSON that
    /// omits this field keeps the default floor (never `0.0`).
    #[serde(default = "default_confidence_floor")]
    pub confidence_floor: f64,
    /// Entity ids deemed "high-impact" (e.g. a core thesis). An action touching
    /// one escalates regardless of confidence (the partner design §4.3). Empty by
    /// default — nothing is high-impact until the config marks it so.
    #[serde(default)]
    pub high_impact_entities: Vec<String>,
}

/// The default low-confidence escalation floor (the partner design §4.3).
///
/// A standalone function so both [`Default`] and the per-field
/// `#[serde(default = ...)]` share ONE source of truth — a partial config can
/// never silently zero the floor.
const fn default_confidence_floor() -> f64 {
    0.5
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            confidence_floor: default_confidence_floor(),
            high_impact_entities: Vec::new(),
        }
    }
}

impl EscalationConfig {
    /// Whether `action` touches a high-impact entity (the partner design §4.3).
    ///
    /// A `Forget` is checked against its `from`/`to` nodes; a KG write/promote is
    /// checked against the proposal's referenced entities via its `why`
    /// provenance KG nodes. Empty config → never high-impact.
    #[must_use]
    fn is_high_impact(&self, action: &ActionRecord) -> bool {
        if self.high_impact_entities.is_empty() {
            return false;
        }
        let touches = |node: &str| self.high_impact_entities.iter().any(|e| e == node);
        // A `match` rather than an `if let ... && ...` let-chain: the let-chain
        // feature is unstable on older toolchains, so the portable form avoids
        // the unstable-feature dependency (Gemini #441 MEDIUM). A `match` (vs a
        // nested `if let`) also keeps clippy::collapsible_if happy.
        let forget_touches_core = match &action.what {
            ActionKind::Forget { from, to, .. } => touches(from) || touches(to),
            _ => false,
        };
        forget_touches_core || action.why.kg_nodes.iter().any(|node| touches(node))
    }
}

/// Decide the status of an action under the escalation predicate (the partner
/// design §4.3, W5.2).
///
/// The default posture is [`ActionStatus::AutoAccepted`]; an action escalates to
/// [`ActionStatus::AwaitingHuman`] iff it is **low-confidence**
/// (`confidence < confidence_floor`), **high-impact** (touches a flagged
/// entity), or **irreversible** (no defined undo — should be rare, flagged
/// loudly). This is the explicit predicate, not a per-action human decision:
/// routine acts auto-accept, flagged acts wait.
#[must_use]
pub fn escalation_status(action: &ActionRecord, config: &EscalationConfig) -> ActionStatus {
    let low_confidence = action.confidence < config.confidence_floor;
    let irreversible = !action.reversible;
    if low_confidence || irreversible || config.is_high_impact(action) {
        ActionStatus::AwaitingHuman
    } else {
        ActionStatus::AutoAccepted
    }
}

/// The default confidence attributed to a gated, materialized action.
///
/// A proposal that cleared the gate and materialized is high-confidence by
/// construction (it passed the validators + the eval threshold), so it sits well
/// above the default floor and auto-accepts. A still-pending proposal is
/// attributed a below-floor confidence so it surfaces as `AwaitingHuman` until it
/// materializes — the honest "I have not committed this yet" signal.
const MATERIALIZED_CONFIDENCE: f64 = 0.95;
const PENDING_CONFIDENCE: f64 = 0.4;

/// Assemble the audit feed (the partner design §4.2/§4.3, W5.1 + W5.2).
///
/// Projects the source records into one ranked [`ActionRecord`] stream, applying
/// the auto-accept-within-gates default and the escalation predicate. **Pure
/// given its inputs.**
///
/// Every record is rendered with a non-empty `why` (W5.1). Ordering is stable and
/// fully determined by the inputs: `AwaitingHuman` rows lead (the human's
/// attention list), then the rest, each group by id — so "what I'm waiting on
/// you for" and "what I did" share ONE surface (the partner design §4.3), not two.
#[must_use]
pub fn audit_feed(
    inputs: &AuditInputs,
    principal: &Principal,
    config: &EscalationConfig,
) -> Vec<ActionRecord> {
    let mut records: Vec<ActionRecord> =
        Vec::with_capacity(inputs.proposals.len() + inputs.tunes.len() + inputs.lessons.len());

    for proposal in &inputs.proposals {
        records.push(project_proposal(proposal));
    }
    for tune in &inputs.tunes {
        // Only an APPLIED tune is an action that happened; a no-op cycle changed
        // nothing, so it is not an audited action.
        if tune.outcome.applied() {
            records.push(project_tune(tune));
        }
    }
    for lesson in &inputs.lessons {
        // Only a materialized (Active) lesson is an installed behavior change.
        if lesson.state == LessonState::Active {
            records.push(project_lesson(lesson));
        }
    }

    // Apply the escalation predicate to set each row's status, then sort so the
    // AwaitingHuman rows lead (one surface for "did" + "waiting on you"), id asc.
    for record in &mut records {
        record.status = escalation_status(record, config);
    }
    records.sort_by(|a, b| {
        awaiting_first(a.status)
            .cmp(&awaiting_first(b.status))
            .then(a.id.cmp(&b.id))
    });
    let _ = principal; // scopes the feed's namespace, not the projection.
    records
}

/// Sort key: `AwaitingHuman` (0) before everything else (1), so the human's
/// attention list leads the shared feed.
const fn awaiting_first(status: ActionStatus) -> u8 {
    match status {
        ActionStatus::AwaitingHuman => 0,
        ActionStatus::AutoAccepted | ActionStatus::Undone | ActionStatus::Corrected => 1,
    }
}

/// Project one [`Proposal`] into an [`ActionRecord`] (W5.1).
///
/// The `why` is built from the proposal's own audit trail: its `history` (the
/// transition log) and the referenced entities become the provenance chain, and
/// a `Forget`'s `reason` is folded in as the `tdw.kg.why`-style audit string.
fn project_proposal(proposal: &Proposal) -> ActionRecord {
    let confidence = if proposal.materialized {
        MATERIALIZED_CONFIDENCE
    } else {
        PENDING_CONFIDENCE
    };
    let (what, kg_nodes, reason) = match &proposal.kind {
        ProposalKind::Forget {
            from,
            rel,
            to,
            reason,
        } => (
            ActionKind::Forget {
                proposal_id: proposal.id.clone(),
                from: from.clone(),
                rel: rel.clone(),
                to: to.clone(),
            },
            vec![from.clone(), to.clone()],
            Some(reason.clone()),
        ),
        ProposalKind::Edge { from, to, rel } => (
            ActionKind::Edge {
                from: from.clone(),
                rel: rel.clone(),
                to: to.clone(),
            },
            vec![from.clone(), to.clone()],
            None,
        ),
        ProposalKind::Annotation { entity_id, .. } => (
            ActionKind::KgWrite {
                proposal_id: proposal.id.clone(),
                // An annotation write adds an `annotated_by` edge from the entity
                // to `annotation:<proposal>`; the undo retires THAT edge to cold,
                // so the reversal is real and self-describing.
                reversal: KgWriteReversal::Annotation {
                    entity_id: entity_id.clone(),
                    annotation_node: annotation_node_id(&proposal.id),
                },
            },
            vec![entity_id.clone()],
            None,
        ),
        ProposalKind::TagAssign { entity_id, tag_id } => (
            ActionKind::KgWrite {
                proposal_id: proposal.id.clone(),
                reversal: KgWriteReversal::Unsupported {
                    detail: format!(
                        "tag assign {tag_id} on {entity_id} has no reversal primitive in the \
                         tag-engine contract; reverse by re-defining/re-assigning through the gate"
                    ),
                },
            },
            vec![entity_id.clone()],
            None,
        ),
        ProposalKind::TagDefine { tag_id, .. } => (
            ActionKind::KgWrite {
                proposal_id: proposal.id.clone(),
                reversal: KgWriteReversal::Unsupported {
                    detail: format!(
                        "tag define {tag_id} has no reversal primitive in the tag-engine contract"
                    ),
                },
            },
            vec![tag_id.clone()],
            None,
        ),
    };
    let why = build_why(&proposal.history, kg_nodes, reason);
    let reversible = what.reversible();
    ActionRecord {
        id: format!("action:proposal:{}", proposal.id),
        agent_id: proposal.agent_id.clone(),
        what,
        why,
        confidence,
        reversible,
        // Provisional; audit_feed sets the real status via the predicate.
        status: ActionStatus::AutoAccepted,
    }
}

/// The annotation node id a materialized [`ProposalKind::Annotation`] creates
/// (`annotation:<proposal>`), matching `write_proposal` in `tdw-knowledge`.
///
/// The undo retires the `annotated_by` edge whose `to` is this node, so the
/// id must be derived identically to the write side.
fn annotation_node_id(proposal_id: &str) -> String {
    format!("annotation:{proposal_id}")
}

/// Project one APPLIED [`TuneRecord`] into a `ParamTune` action (W5.1).
///
/// Only a [`TuneOutcome::Promoted`] reaches here (the feed filters on
/// `outcome.applied()`), so the prior (`old_value`) and applied (`new_value`)
/// values are both present — carried onto the action so [`undo`] can restore the
/// PRIOR value for real, never a silent no-op (Gemini #441 HIGH).
fn project_tune(tune: &TuneRecord) -> ActionRecord {
    // An applied tune is a gated, eval-passed change → high confidence.
    let why = Provenance {
        routes: vec![tune.provenance.clone()],
        kg_nodes: Vec::new(),
        as_of: Some(tune.decided_at.clone()),
    };
    // `project_tune` is only called for an applied (Promoted) outcome; fall back
    // defensively to the same value (a no-op restore) if that ever changes,
    // rather than panicking inside a pure projection.
    let (applied_value, restore_value) = match &tune.outcome {
        TuneOutcome::Promoted {
            old_value,
            new_value,
            ..
        } => (*new_value, *old_value),
        _ => (0.0, 0.0),
    };
    let what = ActionKind::ParamTune {
        record_id: tune.id.clone(),
        param: tune.param.id().to_string(),
        applied_value,
        restore_value,
    };
    let reversible = what.reversible();
    ActionRecord {
        id: format!("action:tune:{}", tune.id),
        agent_id: tdw_knowledge::self_tune::TUNER_AGENT_ID.to_string(),
        what,
        why,
        confidence: MATERIALIZED_CONFIDENCE,
        reversible,
        status: ActionStatus::AutoAccepted,
    }
}

/// Project one Active [`LessonAudit`] into a `RuledPromote` action (W5.1).
fn project_lesson(lesson: &LessonAudit) -> ActionRecord {
    let why = Provenance {
        routes: vec![format!(
            "lesson:{}:{}",
            lesson.lesson.task_class, lesson.lesson.recommended_behavior
        )],
        // The supporting episode ids are the evidence chain.
        kg_nodes: lesson.lesson.evidence.clone(),
        as_of: None,
    };
    let what = ActionKind::RuledPromote {
        proposal_id: lesson.proposal_id.clone(),
        // A lesson is a gated annotation on the lessons subject; the promote
        // added an `annotated_by` edge subject -> annotation:<proposal>. The undo
        // retires THAT edge to cold, demoting the rule out of active behavior.
        subject_entity: tdw_knowledge::lessons::LESSON_SUBJECT_ENTITY.to_string(),
        annotation_node: annotation_node_id(&lesson.proposal_id),
    };
    let reversible = what.reversible();
    ActionRecord {
        id: format!("action:lesson:{}", lesson.proposal_id),
        agent_id: lesson.lesson.agent_id.clone(),
        what,
        why,
        // The lesson's own confidence is its observed success rate.
        confidence: lesson.lesson.confidence,
        reversible,
        status: ActionStatus::AutoAccepted,
    }
}

/// Build a non-empty `why` provenance from a record's audit trail (W5.1).
///
/// The proposal `history` lines are the transition audit; the referenced
/// entities are the KG nodes; a `Forget`'s `reason` is appended as the
/// `tdw.kg.why`-style explanation. The result always carries at least the
/// history, so the W5.1 "every action renders a why" gate holds even for a write
/// with no entity nodes.
fn build_why(history: &[String], kg_nodes: Vec<String>, reason: Option<String>) -> Provenance {
    let mut routes: Vec<String> = history.to_vec();
    if let Some(reason) = reason {
        routes.push(format!("why: {reason}"));
    }
    let as_of = history
        .last()
        .and_then(|line| line.split_whitespace().next())
        .map(ToString::to_string);
    Provenance {
        routes,
        kg_nodes,
        as_of,
    }
}

/// A correction the human supplies when reversing an action (the partner design
/// §4.2 `correct`).
///
/// Carries WHY the action was wrong, so [`correct`] can both undo it AND route a
/// `tdw.kg.feedback` signal that trains the loop (lowers trust for the action's
/// source + records a `Lesson`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    /// The audited action id being corrected.
    pub action_id: String,
    /// The human's reason / the right answer — the feedback payload.
    pub note: String,
}

/// The feedback signal a [`correct`] emits for the surface to route through
/// `tdw.kg.feedback` (the partner design §4.2/§4.4, W5.4).
///
/// `tdw-partner` is a leaf, so it does not call the feedback handler itself; it
/// produces this descriptor and the adapter routes it. `trust_delta` is negative
/// (a correction LOWERS trust for the action's source), which is exactly the
/// signal `self_tune`/`lessons` consume to suppress the corrected behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// The action that was corrected.
    pub action_id: String,
    /// The negative trust adjustment a correction applies (always `< 0`).
    pub trust_delta: f64,
    /// The human's note, recorded as the lesson body.
    pub note: String,
}

/// The trust penalty a single correction applies (the partner design §4.4: a
/// correction LOWERS trust). Bounded and negative.
const CORRECTION_TRUST_DELTA: f64 = -0.1;

/// The outcome of reversing an action — a REAL reversal record, never a silent
/// no-op (the partner design §4.2, W5.3; the Gemini #441 HIGH fix).
///
/// Every variant documents what was actually reverted: a recalled/retired
/// cold-plane edge for the graph reversals, or the restored parameter value for
/// a param tune. A genuinely-irreversible action does NOT reach here — [`undo`]
/// returns [`UndoError::Unsupported`] for it instead of fabricating a success.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UndoOutcome {
    /// The action that was reversed.
    pub action_id: String,
    /// What the reversal actually did (the real reverted edge/record).
    pub reversal: UndoReversal,
}

/// What a successful [`undo`] actually reverted (W5.3).
///
/// This is the proof the undo was REAL: the cold-plane edge that was
/// recalled/retired, or the parameter value that was restored. There is no
/// "did nothing" variant — an action with no real reversal is an
/// [`UndoError::Unsupported`], never a silent `Ok`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reversal", rename_all = "snake_case")]
pub enum UndoReversal {
    /// A cold-plane edge was recalled (undo a `Forget`) or retired (undo an
    /// added edge / annotation / promoted lesson). The reverted edge is carried.
    Edge(RetiredEdge),
    /// A self-tune parameter was restored to its prior value (undo a param tune).
    ParamRestored {
        /// The parameter that was restored.
        param: String,
        /// The value it was restored TO (the value before the tune).
        restored_value: f64,
    },
}

/// An error reversing an action (W5.3).
///
/// Either the underlying cold-plane move failed, or the action is genuinely
/// irreversible — in which case [`undo`] returns [`Self::Unsupported`] LOUDLY
/// rather than a vacuous success (the Gemini #441 HIGH fix: never a silent `Ok`
/// that callers mistake for a real undo).
#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    /// The cold-plane recall/retire failed (e.g. no matching edge).
    #[error(transparent)]
    Knowledge(#[from] tdw_knowledge::KnowledgeError),
    /// The action has no defined reversal (e.g. a tag assign/define, which the
    /// tag-engine contract cannot retire). Reversal must be driven explicitly
    /// through the gate; an automatic undo refuses rather than pretend.
    #[error("action {action_id} is not reversible: {detail}")]
    Unsupported {
        /// The action that could not be reversed.
        action_id: String,
        /// Why it is irreversible.
        detail: String,
    },
}

/// The `(from, rel, to)` edge an undo recalled or retired (W5.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredEdge {
    /// The edge's `from` node.
    pub from: String,
    /// The edge's relationship type.
    pub rel: String,
    /// The edge's `to` node.
    pub to: String,
    /// The RFC 3339 instant the reversal was recorded at.
    pub at: String,
}

impl From<RetireOutcome> for RetiredEdge {
    fn from(outcome: RetireOutcome) -> Self {
        Self {
            from: outcome.from,
            rel: outcome.rel,
            to: outcome.to,
            at: outcome.closed_at,
        }
    }
}

/// One-gesture undo: reverse `action` against the graph, reusing the
/// already-reversible cold-plane machinery (the partner design §4.2, W5.3).
///
/// This is the heart of "audit-only autonomy is SAFE": every reversible action
/// has a defined inverse and undo PERFORMS it (the Gemini #441 HIGH fix — no
/// action returns a vacuous `Ok` that the caller mistakes for a real undo):
/// - a [`ActionKind::Forget`] is reversed by [`recall_cold_edge`] (recall the
///   retired edge back to the active plane);
/// - a [`ActionKind::Edge`] write is reversed by an inverse `Forget`
///   ([`retire_edge_to_cold`]) — the same cold-plane move that retires any fact;
/// - a [`ActionKind::KgWrite`] that is an annotation is reversed by retiring its
///   `annotated_by` edge to cold; a tag assign/define `KgWrite` has NO reversal
///   primitive and returns [`UndoError::Unsupported`] (never a silent success);
/// - a [`ActionKind::ParamTune`] restores the parameter's PRIOR value (carried
///   on the action) — the real re-tune, surfaced as [`UndoReversal::ParamRestored`];
/// - a [`ActionKind::RuledPromote`] retires the promoted lesson's `annotated_by`
///   edge to cold, demoting the rule out of active behavior.
///
/// The historical record is NEVER destroyed — recall/retire preserve the cold
/// audit trail, so a prior `as_of` query is unaffected (the partner design §4.1).
///
/// # Errors
///
/// Returns [`UndoError::Knowledge`] when the cold-plane move fails (e.g. no
/// matching edge to recall/retire), or [`UndoError::Unsupported`] when the action
/// is genuinely irreversible (a tag assign/define) — LOUDLY, never a silent `Ok`.
pub async fn undo(
    graph: &Arc<dyn GraphEngine>,
    action: &ActionRecord,
    now: &str,
) -> Result<UndoOutcome, UndoError> {
    let reversal = match &action.what {
        ActionKind::Forget { from, rel, to, .. } => {
            // Reverse a Forget: recall the cold edge back to active.
            let outcome = recall_cold_edge(graph, from, rel, to, now).await?;
            UndoReversal::Edge(outcome.into())
        }
        ActionKind::Edge { from, rel, to } => {
            // Reverse an added edge: the inverse Forget retires it to cold — the
            // SAME cold-plane move that retires any fact, so the undo is as
            // reversible (and as audited) as a Forget.
            let outcome = retire_edge_to_cold(
                graph,
                from,
                rel,
                to,
                "undo: reverse autonomous write",
                "partner-undo",
                now,
            )
            .await?;
            UndoReversal::Edge(outcome.into())
        }
        ActionKind::KgWrite { reversal, .. } => match reversal {
            // An annotation write is reversed by retiring its `annotated_by`
            // edge to cold — a REAL reversal on the same machinery, not a no-op.
            KgWriteReversal::Annotation {
                entity_id,
                annotation_node,
            } => {
                let outcome = retire_edge_to_cold(
                    graph,
                    entity_id,
                    "annotated_by",
                    annotation_node,
                    "undo: retire autonomous annotation",
                    "partner-undo",
                    now,
                )
                .await?;
                UndoReversal::Edge(outcome.into())
            }
            // A tag assign/define has no reversal primitive: refuse LOUDLY.
            KgWriteReversal::Unsupported { detail } => {
                return Err(UndoError::Unsupported {
                    action_id: action.id.clone(),
                    detail: detail.clone(),
                });
            }
        },
        ActionKind::ParamTune {
            param,
            restore_value,
            ..
        } => {
            // Reverse a self-tune: restore the parameter to its PRIOR value. The
            // value travels on the action (read from the TuneRecord's old_value),
            // so the reversal is real and self-contained — no second lookup, no
            // silent no-op. The daemon applies `restored_value` through the same
            // gated self-tune path the original change took.
            UndoReversal::ParamRestored {
                param: param.clone(),
                restored_value: *restore_value,
            }
        }
        ActionKind::RuledPromote {
            subject_entity,
            annotation_node,
            ..
        } => {
            // Reverse a promoted lesson: retire its `annotated_by` edge to cold,
            // demoting the rule out of active behavior (the historical record is
            // preserved). A REAL reversal, not a no-op.
            let outcome = retire_edge_to_cold(
                graph,
                subject_entity,
                "annotated_by",
                annotation_node,
                "undo: demote promoted lesson",
                "partner-undo",
                now,
            )
            .await?;
            UndoReversal::Edge(outcome.into())
        }
    };
    Ok(UndoOutcome {
        action_id: action.id.clone(),
        reversal,
    })
}

/// Correct an action: undo it **and** emit the training feedback (the partner
/// design §4.2/§4.4, W5.4).
///
/// Returns both the [`UndoOutcome`] (the reversal) and the [`FeedbackSignal`] the
/// adapter routes through `tdw.kg.feedback`. The feedback's `trust_delta` is
/// negative — a correction LOWERS trust for the action's source — and its `note`
/// is recorded as a `Lesson`, so the correction *trains* the loop (a strictly
/// more informative signal than a pre-write rejection, the partner design §4.4).
///
/// # Errors
///
/// Propagates the [`undo`] error when the reversal fails or the action is
/// irreversible ([`UndoError`]); the feedback is only emitted alongside a
/// successful undo, so a correction never trains the loop on a reversal that did
/// not actually happen.
pub async fn correct(
    graph: &Arc<dyn GraphEngine>,
    action: &ActionRecord,
    correction: &Correction,
    now: &str,
) -> Result<(UndoOutcome, FeedbackSignal), UndoError> {
    let undo_outcome = undo(graph, action, now).await?;
    let feedback = FeedbackSignal {
        action_id: action.id.clone(),
        trust_delta: CORRECTION_TRUST_DELTA,
        note: correction.note.clone(),
    };
    Ok((undo_outcome, feedback))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_knowledge::proposals::ProposalQueue;
    use tdw_knowledge::self_tune::{SelfTuneLog, TunableParam, TuneOutcome};
    use tdw_storage_graph::InMemoryGraphEngine;
    use tdw_tags::{InMemoryTagEngine, TagEngine};
    use tdw_taxonomy::Adaptivity;

    fn principal() -> Principal {
        Principal::new("sess", "agent:partner")
    }

    fn forget_proposal(id: &str, materialized: bool) -> Proposal {
        Proposal {
            id: id.to_string(),
            kind: ProposalKind::Forget {
                from: "company:ACME".to_string(),
                rel: "hq_in".to_string(),
                to: "city:old".to_string(),
                reason: "stale low-confidence fact (policies: age, low-conf)".to_string(),
            },
            agent_id: "agent:partner".to_string(),
            status: tdw_taxonomy::ValidationStatus::Validated,
            rejected: None,
            materialized,
            history: vec![
                "2026-06-14 draft by agent:partner".to_string(),
                "2026-06-14 validated (automated validators)".to_string(),
            ],
        }
    }

    fn edge_proposal(id: &str) -> Proposal {
        Proposal {
            id: id.to_string(),
            kind: ProposalKind::Edge {
                from: "a".to_string(),
                to: "b".to_string(),
                rel: "supplier_of".to_string(),
            },
            agent_id: "agent:partner".to_string(),
            status: tdw_taxonomy::ValidationStatus::Validated,
            rejected: None,
            materialized: true,
            history: vec!["2026-06-14 draft by agent:partner".to_string()],
        }
    }

    fn annotation_proposal(id: &str, entity_id: &str) -> Proposal {
        Proposal {
            id: id.to_string(),
            kind: ProposalKind::Annotation {
                entity_id: entity_id.to_string(),
                note: "an autonomous annotation".to_string(),
            },
            agent_id: "agent:partner".to_string(),
            status: tdw_taxonomy::ValidationStatus::Validated,
            rejected: None,
            materialized: true,
            history: vec!["2026-06-14 draft by agent:partner".to_string()],
        }
    }

    fn tag_assign_proposal(id: &str) -> Proposal {
        Proposal {
            id: id.to_string(),
            kind: ProposalKind::TagAssign {
                entity_id: "company:ACME".to_string(),
                tag_id: "sector:tech".to_string(),
            },
            agent_id: "agent:partner".to_string(),
            status: tdw_taxonomy::ValidationStatus::Validated,
            rejected: None,
            materialized: true,
            history: vec!["2026-06-14 draft by agent:partner".to_string()],
        }
    }

    /// Materialize an Annotation proposal into the graph EXACTLY as
    /// `write_proposal` does (annotation node + `annotated_by` edge), so a real
    /// write→undo→retired test runs against the real cold-plane.
    async fn seed_annotation(graph: &Arc<dyn GraphEngine>, proposal_id: &str, entity_id: &str) {
        let annotation_id = annotation_node_id(proposal_id);
        graph
            .upsert_nodes(vec![
                node(entity_id, entity_id),
                node(&annotation_id, "note"),
            ])
            .await
            .expect("seed annotation nodes");
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: entity_id.to_string(),
                to: annotation_id,
                rel: "annotated_by".to_string(),
                props: serde_json::Value::Null,
                provenance: tdw_core::Provenance::Agent {
                    agent_id: "agent:partner".to_string(),
                    gated: true,
                },
                valid_from: Some("2026-06-14T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("seed annotated_by edge");
    }

    // ── W5.1: the feed renders a `why` for EVERY action ──────────────────────

    #[test]
    fn feed_renders_a_why_for_every_action() {
        let mut log = SelfTuneLog::new();
        log.record(
            TunableParam::RrfK,
            TuneOutcome::Promoted {
                old_value: 60.0,
                new_value: 62.0,
                incumbent_score: 0.80,
                candidate_score: 0.86,
            },
            "2026-06-14",
        );
        let inputs = AuditInputs {
            proposals: vec![forget_proposal("p1", true), edge_proposal("p2")],
            tunes: vec![log.last().expect("a record").clone()],
            lessons: Vec::new(),
        };
        let feed = audit_feed(&inputs, &principal(), &EscalationConfig::default());
        assert_eq!(feed.len(), 3, "two proposals + one applied tune: {feed:?}");
        for record in &feed {
            assert!(
                !record.why.routes.is_empty(),
                "every action renders a non-empty why: {record:?}"
            );
        }
        // The Forget's reason is folded into the why chain.
        let forget = feed
            .iter()
            .find(|r| matches!(r.what, ActionKind::Forget { .. }))
            .expect("forget action present");
        assert!(
            forget.why.routes.iter().any(|r| r.starts_with("why: ")),
            "the Forget reason is in the why: {:?}",
            forget.why.routes
        );
    }

    #[test]
    fn no_op_tune_is_not_an_audited_action() {
        let mut log = SelfTuneLog::new();
        log.record(
            TunableParam::RrfK,
            TuneOutcome::NoImprovement {
                candidate_value: 62.0,
                incumbent_score: 0.80,
                candidate_score: 0.79,
            },
            "2026-06-14",
        );
        let inputs = AuditInputs {
            proposals: Vec::new(),
            tunes: vec![log.last().expect("record").clone()],
            lessons: Vec::new(),
        };
        let feed = audit_feed(&inputs, &principal(), &EscalationConfig::default());
        assert!(feed.is_empty(), "a no-op tune changed nothing: {feed:?}");
    }

    // ── W5.2: auto-accept-within-gates default + escalation predicate ─────────

    #[test]
    fn routine_materialized_action_auto_accepts() {
        let inputs = AuditInputs {
            proposals: vec![forget_proposal("p1", true)],
            ..AuditInputs::default()
        };
        let feed = audit_feed(&inputs, &principal(), &EscalationConfig::default());
        assert_eq!(feed[0].status, ActionStatus::AutoAccepted);
    }

    #[test]
    fn low_confidence_action_escalates_to_awaiting_human() {
        // A still-pending proposal carries below-floor confidence → AwaitingHuman.
        let inputs = AuditInputs {
            proposals: vec![forget_proposal("p1", false)],
            ..AuditInputs::default()
        };
        let feed = audit_feed(&inputs, &principal(), &EscalationConfig::default());
        assert_eq!(
            feed[0].status,
            ActionStatus::AwaitingHuman,
            "a low-confidence act waits for a human"
        );
    }

    #[test]
    fn high_impact_entity_escalates_even_when_confident() {
        // The materialized (high-confidence) Forget touches a flagged core entity.
        let config = EscalationConfig {
            confidence_floor: 0.5,
            high_impact_entities: vec!["company:ACME".to_string()],
        };
        let inputs = AuditInputs {
            proposals: vec![forget_proposal("p1", true)],
            ..AuditInputs::default()
        };
        let feed = audit_feed(&inputs, &principal(), &config);
        assert_eq!(
            feed[0].status,
            ActionStatus::AwaitingHuman,
            "a high-impact action escalates despite high confidence"
        );
    }

    #[test]
    fn awaiting_human_rows_lead_the_shared_feed() {
        // One auto-accepted + one awaiting: the awaiting row leads (one surface
        // for "did" + "waiting on you").
        let inputs = AuditInputs {
            proposals: vec![
                forget_proposal("p1", true),  // auto-accept
                forget_proposal("p2", false), // awaiting (low-conf)
            ],
            ..AuditInputs::default()
        };
        let feed = audit_feed(&inputs, &principal(), &EscalationConfig::default());
        assert_eq!(feed[0].status, ActionStatus::AwaitingHuman);
        assert_eq!(feed[0].id, "action:proposal:p2");
        assert_eq!(feed[1].status, ActionStatus::AutoAccepted);
    }

    // ── W5.3: reversibility — write → undo → state restored ──────────────────

    #[tokio::test]
    async fn undo_a_forget_recalls_the_cold_edge_and_restores_state() {
        // Build a real graph, seed an edge, run a gated Forget to cold, then undo
        // (recall) and assert the edge is active again — the write→undo→restored
        // reversibility test the W5.3 gate demands, on the REAL cold-plane.
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

        // Seed nodes + the active edge.
        graph
            .upsert_nodes(vec![
                node("company:ACME", "ACME"),
                node("city:old", "Old City"),
            ])
            .await
            .expect("seed nodes");
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: "company:ACME".to_string(),
                to: "city:old".to_string(),
                rel: "hq_in".to_string(),
                props: serde_json::Value::Null,
                provenance: tdw_core::Provenance::Ingest {
                    source: "seed".to_string(),
                },
                valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("seed edge");

        // Materialize a Forget through the gate (the autonomous action).
        let mut queue = ProposalQueue::default();
        let proposal = queue
            .submit(
                "agent:partner",
                Adaptivity::Learning,
                ProposalKind::Forget {
                    from: "company:ACME".to_string(),
                    rel: "hq_in".to_string(),
                    to: "city:old".to_string(),
                    reason: "stale fact".to_string(),
                },
                &graph,
                &tags,
                "2026-06-14",
            )
            .await
            .expect("forget submits");
        let promoted = queue.promote_for_agent("agent:partner", 1.0, 5, "2026-06-14");
        assert!(
            promoted.contains(&proposal.id),
            "the forget is promoted to ready: {promoted:?}"
        );
        let report = queue
            .materialize_ready(&graph, &tags, "2026-06-14")
            .await
            .expect("materialize");
        assert!(
            report.materialized.contains(&proposal.id),
            "the Forget materialized: {report:?}"
        );

        // Confirm the edge is now cold (closed validity window).
        let cold = graph.edges(Some("hq_in"), 0, 16).await.expect("read edges");
        assert!(
            cold.iter().all(|e| e.valid_to.is_some()),
            "the forgotten edge is cold: {cold:?}"
        );

        // Now UNDO via the audit surface and assert the edge is active again.
        let action = project_proposal(&forget_proposal(&proposal.id, true));
        let outcome = undo(&graph, &action, "2026-06-15")
            .await
            .expect("undo recalls the cold edge");
        let UndoReversal::Edge(edge) = outcome.reversal else {
            panic!("a Forget undo reverts an edge: {:?}", outcome.reversal);
        };
        assert_eq!(edge.from, "company:ACME");
        assert_eq!(edge.to, "city:old");

        let restored = graph.edges(Some("hq_in"), 0, 16).await.expect("read edges");
        assert!(
            restored
                .iter()
                .any(|e| e.to == "city:old" && e.valid_to.is_none()),
            "the edge is ACTIVE again after undo: {restored:?}"
        );
    }

    #[tokio::test]
    async fn undo_an_edge_write_retires_it_to_cold() {
        // The inverse-Forget undo path: an added edge is retired to cold, so a
        // KG WRITE is as reversible as a Forget (write → undo → retired).
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        graph
            .upsert_nodes(vec![node("a", "A"), node("b", "B")])
            .await
            .expect("seed nodes");
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                rel: "supplier_of".to_string(),
                props: serde_json::Value::Null,
                provenance: tdw_core::Provenance::Ingest {
                    source: "partner-write".to_string(),
                },
                valid_from: Some("2026-06-14T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("seed active edge");

        let action = project_proposal(&edge_proposal("p3"));
        assert!(matches!(action.what, ActionKind::Edge { .. }));
        let outcome = undo(&graph, &action, "2026-06-15")
            .await
            .expect("undo retires the added edge");
        assert!(
            matches!(outcome.reversal, UndoReversal::Edge(_)),
            "the edge undo touched the graph: {:?}",
            outcome.reversal
        );

        let edges = graph
            .edges(Some("supplier_of"), 0, 16)
            .await
            .expect("read edges");
        assert!(
            edges.iter().all(|e| e.valid_to.is_some()),
            "the added edge is now cold after undo: {edges:?}"
        );
    }

    #[tokio::test]
    async fn undo_an_annotation_kgwrite_retires_the_annotated_by_edge() {
        // The KgWrite (annotation) undo path the Gemini #441 HIGH fix demands a
        // REAL reversal for: write the annotation, undo, assert the annotated_by
        // edge is now cold (write → undo → state restored).
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        seed_annotation(&graph, "p7", "company:ACME").await;

        let action = project_proposal(&annotation_proposal("p7", "company:ACME"));
        assert!(
            matches!(
                &action.what,
                ActionKind::KgWrite {
                    reversal: KgWriteReversal::Annotation { .. },
                    ..
                }
            ),
            "an annotation projects to a reversible KgWrite: {:?}",
            action.what
        );
        assert!(action.reversible, "an annotation KgWrite is reversible");

        let outcome = undo(&graph, &action, "2026-06-15")
            .await
            .expect("undo retires the annotation edge");
        let UndoReversal::Edge(edge) = outcome.reversal else {
            panic!("an annotation undo reverts an edge: {:?}", outcome.reversal);
        };
        assert_eq!(edge.from, "company:ACME");
        assert_eq!(edge.to, annotation_node_id("p7"));

        let edges = graph
            .edges(Some("annotated_by"), 0, 16)
            .await
            .expect("read edges");
        assert!(
            edges.iter().all(|e| e.valid_to.is_some()),
            "the annotation edge is cold after undo: {edges:?}"
        );
    }

    #[tokio::test]
    async fn undo_a_tag_kgwrite_is_explicitly_unsupported_not_silent_ok() {
        // A tag assign/define has NO reversal primitive: undo must refuse LOUDLY
        // (UndoError::Unsupported), never the silent Ok the Gemini #441 HIGH flag
        // called out. No graph state is needed — the refusal is structural.
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let action = project_proposal(&tag_assign_proposal("p8"));
        assert!(
            !action.reversible,
            "a tag KgWrite is flagged irreversible: {:?}",
            action.what
        );

        let error = undo(&graph, &action, "2026-06-15")
            .await
            .expect_err("a tag write undo is explicitly unsupported");
        match error {
            UndoError::Unsupported { action_id, .. } => {
                assert_eq!(action_id, action.id, "names the refused action");
            }
            other @ UndoError::Knowledge(_) => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn undo_a_param_tune_restores_the_prior_value() {
        // The ParamTune undo path: a promoted tune (60 → 62) must undo by
        // RESTORING 60 — a real re-tune, surfaced as ParamRestored, never a
        // silent no-op (the Gemini #441 HIGH fix).
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let mut log = SelfTuneLog::new();
        log.record(
            TunableParam::RrfK,
            TuneOutcome::Promoted {
                old_value: 60.0,
                new_value: 62.0,
                incumbent_score: 0.80,
                candidate_score: 0.86,
            },
            "2026-06-14",
        );
        let tune = log.last().expect("a record").clone();
        let action = project_tune(&tune);
        assert!(action.reversible, "a param tune is reversible");
        match &action.what {
            ActionKind::ParamTune {
                applied_value,
                restore_value,
                ..
            } => {
                assert!((applied_value - 62.0).abs() < 1e-12);
                assert!((restore_value - 60.0).abs() < 1e-12);
            }
            other => panic!("expected ParamTune, got {other:?}"),
        }

        let outcome = undo(&graph, &action, "2026-06-15")
            .await
            .expect("a param tune undo restores the prior value");
        match outcome.reversal {
            UndoReversal::ParamRestored {
                param,
                restored_value,
            } => {
                assert_eq!(param, "rrf_k");
                assert!(
                    (restored_value - 60.0).abs() < 1e-12,
                    "restored to the prior value 60, got {restored_value}"
                );
            }
            other @ UndoReversal::Edge(_) => panic!("expected ParamRestored, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn undo_a_ruled_promote_demotes_the_lesson_edge() {
        // The RuledPromote undo path: a materialized lesson is a gated annotation
        // on the lessons subject; undo retires its annotated_by edge to cold,
        // demoting the rule (write → undo → demoted), never a silent no-op.
        use tdw_knowledge::lessons::{LESSON_SUBJECT_ENTITY, Lesson};
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        seed_annotation(&graph, "p10", LESSON_SUBJECT_ENTITY).await;

        let lesson_audit = LessonAudit {
            proposal_id: "p10".to_string(),
            lesson: Lesson {
                agent_id: "agent:partner".to_string(),
                task_class: "valuation".to_string(),
                recommended_behavior: "prefer approach Y".to_string(),
                evidence: vec!["ep-1".to_string()],
                confidence: 0.9,
            },
            state: LessonState::Active,
        };
        let action = project_lesson(&lesson_audit);
        assert!(matches!(action.what, ActionKind::RuledPromote { .. }));
        assert!(action.reversible, "a promoted lesson is reversible");

        let outcome = undo(&graph, &action, "2026-06-15")
            .await
            .expect("a promoted-lesson undo retires its edge");
        let UndoReversal::Edge(edge) = outcome.reversal else {
            panic!("a lesson undo reverts an edge: {:?}", outcome.reversal);
        };
        assert_eq!(edge.from, LESSON_SUBJECT_ENTITY);
        assert_eq!(edge.to, annotation_node_id("p10"));

        let edges = graph
            .edges(Some("annotated_by"), 0, 16)
            .await
            .expect("read edges");
        assert!(
            edges.iter().all(|e| e.valid_to.is_some()),
            "the lesson edge is cold (demoted) after undo: {edges:?}"
        );
    }

    // ── W5.4: correct = undo + feedback that lowers trust + records a lesson ──

    #[tokio::test]
    async fn correct_undoes_and_emits_trust_lowering_feedback() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        graph
            .upsert_nodes(vec![
                node("company:ACME", "ACME"),
                node("city:old", "Old City"),
            ])
            .await
            .expect("seed nodes");
        // Seed a COLD edge directly so undo (recall) has something to restore.
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: "company:ACME".to_string(),
                to: "city:old".to_string(),
                rel: "hq_in".to_string(),
                props: serde_json::json!({
                    "forgotten": {
                        "reason": "stale",
                        "forgotten_at": "2026-06-14T00:00:00Z",
                        "provenance": "partner",
                        "prior_valid_to": null
                    }
                }),
                provenance: tdw_core::Provenance::Ingest {
                    source: "seed".to_string(),
                },
                valid_from: Some("2020-01-01T00:00:00Z".to_string()),
                valid_to: Some("2026-06-14T00:00:00Z".to_string()),
            }])
            .await
            .expect("seed cold edge");

        let action = project_proposal(&forget_proposal("p9", true));
        let correction = Correction {
            action_id: action.id.clone(),
            note: "ACME never moved HQ — keep the fact".to_string(),
        };
        let (undo_outcome, feedback) = correct(&graph, &action, &correction, "2026-06-15")
            .await
            .expect("correct undoes + feeds back");

        assert!(
            matches!(undo_outcome.reversal, UndoReversal::Edge(_)),
            "correct reversed the action: {:?}",
            undo_outcome.reversal
        );
        // The correction LOWERS trust (negative delta) and carries the note as
        // the lesson body — the W5.4 "lowers trust + records a Lesson" gate.
        assert!(
            feedback.trust_delta < 0.0,
            "a correction lowers trust: {}",
            feedback.trust_delta
        );
        assert_eq!(feedback.action_id, action.id);
        assert!(feedback.note.contains("never moved HQ"));
    }

    #[test]
    fn irreversible_action_would_escalate() {
        // Defensive: an action flagged irreversible (none exist today) must
        // escalate loudly. We synthesize one to prove the predicate covers it.
        let mut action = project_proposal(&forget_proposal("p1", true));
        action.reversible = false;
        let status = escalation_status(&action, &EscalationConfig::default());
        assert_eq!(
            status,
            ActionStatus::AwaitingHuman,
            "an irreversible action is flagged for a human"
        );
    }

    #[test]
    fn audit_record_round_trips_through_serde() {
        let action = project_proposal(&forget_proposal("p1", true));
        let json = serde_json::to_string(&action).expect("serialize");
        let back: ActionRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(action, back);
    }

    // ── Gemini #441 MEDIUM: partial EscalationConfig keeps the default floor ──

    #[test]
    fn partial_config_deserializes_with_default_confidence_floor() {
        // A partial config that sets ONLY high_impact_entities must keep the
        // default floor (0.5), NOT zero it out — otherwise every action would be
        // "high-confidence" and the low-confidence escalation would silently die.
        let config: EscalationConfig =
            serde_json::from_str(r#"{"high_impact_entities":["thesis:core"]}"#)
                .expect("partial config deserializes");
        assert!(
            (config.confidence_floor - 0.5).abs() < 1e-12,
            "the omitted floor keeps its default, got {}",
            config.confidence_floor
        );
        assert_eq!(config.high_impact_entities, vec!["thesis:core".to_string()]);

        // And an empty config object is the full default.
        let empty: EscalationConfig = serde_json::from_str("{}").expect("empty config");
        assert_eq!(empty, EscalationConfig::default());

        // An explicit floor still overrides.
        let explicit: EscalationConfig =
            serde_json::from_str(r#"{"confidence_floor":0.9}"#).expect("explicit floor");
        assert!((explicit.confidence_floor - 0.9).abs() < 1e-12);
    }

    #[test]
    fn param_tune_action_round_trips_through_serde() {
        // The enriched ParamTune carries the prior/applied values; round-trip so
        // the daemon can serialize the audit row and reverse it later.
        let mut log = SelfTuneLog::new();
        log.record(
            TunableParam::RrfK,
            TuneOutcome::Promoted {
                old_value: 60.0,
                new_value: 62.0,
                incumbent_score: 0.80,
                candidate_score: 0.86,
            },
            "2026-06-14",
        );
        let action = project_tune(log.last().expect("a record"));
        let json = serde_json::to_string(&action).expect("serialize");
        let back: ActionRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(action, back);
    }

    fn node(id: &str, label: &str) -> tdw_core::GraphNode {
        tdw_core::GraphNode {
            id: id.to_string(),
            kind: tdw_taxonomy::EntityKind::Instrument,
            label: label.to_string(),
            aliases: vec![],
            props: serde_json::Value::Null,
            valid_from: None,
            valid_to: None,
        }
    }
}
