//! The agent writeback gate (knowledge-system B9).
//!
//! Agents never write into the knowledge substrate directly. Every write is
//! a [`Proposal`] moving `Draft -> Validated -> Ready`:
//!
//! 1. **Admission** — the proposing agent's [`Adaptivity`] must be at least
//!    `Learning`; below that the submission is rejected outright.
//! 2. **`Draft -> Validated`** — automated validators run on enqueue (shape
//!    grammars, endpoints exist, tag defined, no re-assertion of an existing
//!    fact, per-agent pending cap). A validator failure rejects loudly.
//! 3. **`Validated -> Ready`** — eval-driven: `promote_for_agent` promotes
//!    the agent's validated proposals iff its eval `pass_rate` meets the
//!    ready threshold (default 0.8) AND at least [`MIN_EVAL_CASES`] were
//!    executed. Human approval (`approve`) is the alternative path.
//!    `SelfModifying` agents are exempt from HUMAN review only — never from
//!    evals.
//! 4. **Materialization** — only `Ready` proposals write into the graph/tag
//!    engines, with provenance `agent:<id>;proposal:<pid>` (edges carry
//!    [`Provenance::Agent`] with `gated: true`).
//!
//! **Agent-id grammar**: allowed characters are `[A-Za-z0-9:._-]`; control
//! characters, semicolons, and whitespace are all rejected. Semicolons are
//! reserved as field separators in the provenance string
//! `agent:<id>;proposal:<pid>`. Ids are bounded to [`MAX_AGENT_ID_LEN`] bytes.
//! Empty ids are rejected. Call [`validate_agent_id`] before storing any
//! agent-derived id.
//!
//! **Eval-evidence floor**: a vacuous eval (0 cases) has a vacuously perfect
//! pass rate and MUST NOT promote — [`MIN_EVAL_CASES`] is the mandatory floor
//! that closes this B9 bypass.
//!
//! **Proposal-id collision guarantee**: ids are `p<monotone_u64>`. The
//! `next_id` counter is persisted in serde, so round-trips preserve uniqueness.
//! The counter is never reset; ids therefore never alias across restarts.
//!
//! Net: `Learning` grants the right to PROPOSE; passing evals (or a human)
//! grants the right to LAND.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance, validate_graph_edge};
use tdw_tags::{TagAssignment, TagDefinition, TagEngine, validate_definition_shape};
use tdw_taxonomy::{Adaptivity, EntityKind};
// Re-export so `tdw-backend` (which cannot take `tdw-taxonomy` as a prod dep)
// can reference `ValidationStatus` via `tdw_knowledge::proposals::ValidationStatus`.
pub use tdw_taxonomy::ValidationStatus;

use crate::{KnowledgeError, Result};

/// Default eval pass-rate needed for `Validated -> Ready` promotion.
pub const READY_THRESHOLD: f64 = 0.8;
/// Default cap on one agent's pending (non-materialized) proposals.
pub const PER_AGENT_PENDING_CAP: usize = 32;
/// Hard cap on TOTAL pending proposals across ALL agents.
///
/// Because the caller supplies `agent_id`, the per-agent cap alone is evadable
/// by rotating ids; this bounds the queue regardless of identity (B9 review).
pub const MAX_TOTAL_PENDING: usize = 1024;
/// Annotation note ceiling (notes flow into graph props and agent context).
pub const MAX_NOTE_CHARS: usize = 4096;
/// Maximum byte length of an agent id (prevents oversized provenance strings).
pub const MAX_AGENT_ID_LEN: usize = 128;
/// Maximum proposals returned by [`ProposalQueue::list`] per page.
pub const LIST_PAGE_MAX: usize = 256;
/// Default page size for [`ProposalQueue::list`] when no limit is supplied.
pub const LIST_PAGE_DEFAULT: usize = 100;
/// Minimum number of eval cases that must have been executed for
/// `promote_for_agent` to grant promotion.
///
/// A vacuous eval (0 cases) has a vacuously perfect pass rate and MUST NOT
/// promote — this floor is the B9 evidence guard. Authorship/provenance
/// integrity of the cases themselves is B11 scope.
pub const MIN_EVAL_CASES: usize = 5;

/// Validate an agent id against the graph-id grammar plus the additional
/// constraint that semicolons are forbidden.
///
/// Semicolons are used as field separators in provenance strings of the form
/// `agent:<id>;proposal:<pid>`. Allowed characters: `[A-Za-z0-9:._-]`. Control
/// characters, semicolons,
/// whitespace, and empty strings are all rejected. Ids longer than
/// [`MAX_AGENT_ID_LEN`] bytes are also rejected.
///
/// # Errors
///
/// Returns [`KnowledgeError::Storage`] with a descriptive message when the id
/// is invalid.
pub fn validate_agent_id(agent_id: &str) -> Result<()> {
    if agent_id.is_empty() {
        return Err(KnowledgeError::Storage(
            "agent id must not be empty".to_string(),
        ));
    }
    if agent_id.len() > MAX_AGENT_ID_LEN {
        return Err(KnowledgeError::Storage(format!(
            "agent id is too long ({} bytes, max {MAX_AGENT_ID_LEN})",
            agent_id.len()
        )));
    }
    if let Some(bad) = agent_id
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(KnowledgeError::Storage(format!(
            "agent id {agent_id:?} contains invalid character {bad:?} — only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

/// What an agent wants to write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalKind {
    /// A new graph edge.
    Edge {
        from: String,
        to: String,
        rel: String,
    },
    /// A time-bounded tag assignment.
    TagAssign { entity_id: String, tag_id: String },
    /// A new taxonomy node.
    TagDefine {
        tag_id: String,
        parent: Option<String>,
    },
    /// A free-text note attached to an entity (stored as an annotation node
    /// + `annotated_by` edge).
    Annotation { entity_id: String, note: String },
}

/// One gated write request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    /// Queue-assigned id (`p<seq>`).
    pub id: String,
    pub kind: ProposalKind,
    pub agent_id: String,
    pub status: ValidationStatus,
    /// Set when rejected (human or validator); a rejected proposal never
    /// promotes or materializes.
    #[serde(default)]
    pub rejected: Option<String>,
    /// True once written into the engines.
    #[serde(default)]
    pub materialized: bool,
    /// Audit trail: every transition with its `now` date.
    pub history: Vec<String>,
}

impl Proposal {
    /// Pending = occupies the agent's queue capacity.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        !self.materialized && self.rejected.is_none()
    }

    fn validate_restored(&self) -> Result<()> {
        if self.id.is_empty() || !self.id.starts_with('p') || self.id.len() < 2 {
            return Err(KnowledgeError::Storage(format!(
                "restored proposal has malformed id {:?}",
                self.id
            )));
        }
        validate_agent_id(&self.agent_id)?;
        if self.materialized && self.rejected.is_some() {
            return Err(KnowledgeError::Storage(format!(
                "restored proposal {:?} is both materialized and rejected",
                self.id
            )));
        }
        Ok(())
    }
}

/// Report of one materialization sweep.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeReport {
    /// Proposal ids written into the engines.
    pub materialized: Vec<String>,
    /// `(proposal id, reason)` for `Ready` proposals that FAILED
    /// re-validation at write time (the world changed since enqueue) — they
    /// are rejected, not written, so a stale proposal can never clobber a
    /// fact asserted in the meantime.
    pub rejected_at_materialize: Vec<(String, String)>,
}

/// A bounded page of proposals returned by [`ProposalQueue::list`].
#[derive(Debug)]
pub struct ProposalPage<'a> {
    /// The proposals in this page (at most the requested limit).
    pub proposals: Vec<&'a Proposal>,
    /// Total matching proposals before the page limit was applied.
    pub total: usize,
}

/// The gated proposal queue.
///
/// In-process state; persistence round-trips through serde like the B5
/// `IndexManifest` and B7 `DerivationIndex`.
///
/// TRUST MODEL: a deserialized queue is the DAEMON's own persisted state,
/// never an agent-supplied value — an operator who can write the queue's
/// persistence file already controls the substrate. Status fields are trusted
/// on load; agents reach this type ONLY through [`ProposalQueue::submit`],
/// which forces every proposal through admission + validators starting at
/// `Validated`. Never deserialize a queue from an agent-controlled source.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProposalQueue {
    proposals: BTreeMap<String, Proposal>,
    next_id: u64,
    #[serde(default)]
    ready_threshold: Option<f64>,
    #[serde(default)]
    pending_cap: Option<usize>,
}

impl ProposalQueue {
    /// Override the eval promotion threshold (config seam).
    #[must_use]
    pub const fn with_ready_threshold(mut self, threshold: f64) -> Self {
        self.ready_threshold = Some(threshold);
        self
    }

    fn threshold(&self) -> f64 {
        self.ready_threshold.unwrap_or(READY_THRESHOLD)
    }

    fn cap(&self) -> usize {
        self.pending_cap.unwrap_or(PER_AGENT_PENDING_CAP)
    }

    /// Validate structural invariants for a queue deserialized from storage.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] for any structural violation.
    pub fn validate_restored(&self) -> Result<()> {
        for proposal in self.proposals.values() {
            proposal.validate_restored()?;
        }
        Ok(())
    }

    /// Submit a proposal through the FULL gate: admission (`Adaptivity >=
    /// Learning`), per-agent pending cap, and the automated validators.
    /// Success leaves the proposal `Validated`; any failure is a loud error
    /// and nothing is enqueued.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Tag`]/[`KnowledgeError::Storage`]-shaped
    /// errors describing the failing gate step.
    pub async fn submit(
        &mut self,
        agent_id: &str,
        adaptivity: Adaptivity,
        kind: ProposalKind,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<Proposal> {
        // 0. Agent-id grammar validation (B9 Finding 3).
        validate_agent_id(agent_id)?;
        // 1. Admission: below Learning never writes (mirrors run_eval_at's
        // feedback gate).
        if adaptivity < Adaptivity::Learning {
            return Err(KnowledgeError::Storage(format!(
                "agent {agent_id:?} has adaptivity {adaptivity:?} — below Learning, writes are \
                 not admitted"
            )));
        }
        // 2a. Queue-wide cap: bounds total pending regardless of how many
        // distinct agent_ids a caller invents (the per-agent cap alone is
        // evadable by id rotation since agent_id is caller-supplied).
        let total_pending = self
            .proposals
            .values()
            .filter(|proposal| proposal.is_pending())
            .count();
        if total_pending >= MAX_TOTAL_PENDING {
            return Err(KnowledgeError::Storage(format!(
                "proposal queue is full ({total_pending} pending, cap {MAX_TOTAL_PENDING})"
            )));
        }
        // 2b. Per-agent pending cap.
        let pending = self
            .proposals
            .values()
            .filter(|proposal| proposal.agent_id == agent_id && proposal.is_pending())
            .count();
        if pending >= self.cap() {
            return Err(KnowledgeError::Storage(format!(
                "agent {agent_id:?} has {pending} pending proposals (cap {}) — promote, \
                 materialize, or reject before submitting more",
                self.cap()
            )));
        }
        // 3. Automated validators (Draft -> Validated on enqueue).
        validate_kind(&kind, graph, tags).await?;

        self.next_id += 1;
        let id = format!("p{}", self.next_id);
        let proposal = Proposal {
            id: id.clone(),
            kind,
            agent_id: agent_id.to_string(),
            status: ValidationStatus::Validated,
            rejected: None,
            materialized: false,
            history: vec![
                format!("{now} draft by {agent_id}"),
                format!("{now} validated (automated validators)"),
            ],
        };
        self.proposals.insert(id, proposal.clone());
        Ok(proposal)
    }

    /// Eval-driven promotion: every `Validated` proposal by `agent_id` moves
    /// to `Ready` iff `pass_rate` meets the threshold AND `cases_executed`
    /// meets the [`MIN_EVAL_CASES`] floor. Returns the promoted ids (empty
    /// when either condition falls short — that is not an error; the proposals
    /// simply wait).
    #[must_use]
    pub fn promote_for_agent(
        &mut self,
        agent_id: &str,
        pass_rate: f64,
        cases_executed: usize,
        now: &str,
    ) -> Vec<String> {
        if pass_rate < self.threshold() || cases_executed < MIN_EVAL_CASES {
            return Vec::new();
        }
        let mut promoted = Vec::new();
        for proposal in self.proposals.values_mut() {
            if proposal.agent_id == agent_id
                && proposal.status == ValidationStatus::Validated
                && proposal.is_pending()
            {
                proposal.status = ValidationStatus::Ready;
                proposal.history.push(format!(
                    "{now} ready (eval pass_rate {pass_rate:.2}, cases {cases_executed})"
                ));
                promoted.push(proposal.id.clone());
            }
        }
        promoted
    }

    /// Human approval: the alternative `Validated -> Ready` path.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown ids or proposals not in `Validated`.
    pub fn approve(&mut self, proposal_id: &str, approved_by: &str, now: &str) -> Result<()> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| KnowledgeError::Storage(format!("unknown proposal {proposal_id:?}")))?;
        if proposal.rejected.is_some() || proposal.status != ValidationStatus::Validated {
            return Err(KnowledgeError::Storage(format!(
                "proposal {proposal_id:?} is not awaiting approval"
            )));
        }
        proposal.status = ValidationStatus::Ready;
        proposal
            .history
            .push(format!("{now} ready (approved by {approved_by})"));
        Ok(())
    }

    /// Reject a pending proposal with a reason. Terminal.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown ids, already-materialized proposals, or
    /// already-rejected proposals (rejection is terminal — a second rejection
    /// would silently overwrite the first reason in the audit trail).
    pub fn reject(&mut self, proposal_id: &str, reason: &str, now: &str) -> Result<()> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| KnowledgeError::Storage(format!("unknown proposal {proposal_id:?}")))?;
        if proposal.materialized {
            return Err(KnowledgeError::Storage(format!(
                "proposal {proposal_id:?} is already materialized"
            )));
        }
        if proposal.rejected.is_some() {
            return Err(KnowledgeError::Storage(format!(
                "proposal {proposal_id:?} is already rejected — rejection is terminal"
            )));
        }
        proposal.rejected = Some(reason.to_string());
        proposal.history.push(format!("{now} rejected: {reason}"));
        Ok(())
    }

    /// Write every `Ready` proposal into the engines with provenance
    /// `agent:<id>;proposal:<pid>` and mark it materialized. Only `Ready`
    /// proposals write — pending facts are invisible to reads by default.
    ///
    /// # Errors
    ///
    /// Returns the first failing engine error; already-written proposals in
    /// this sweep stay materialized (the write is per-proposal).
    pub async fn materialize_ready(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<MaterializeReport> {
        let ready: Vec<String> = self
            .proposals
            .values()
            .filter(|proposal| proposal.status == ValidationStatus::Ready && proposal.is_pending())
            .map(|proposal| proposal.id.clone())
            .collect();
        let mut report = MaterializeReport::default();
        for id in ready {
            let proposal = self.proposals.get(&id).cloned().ok_or_else(|| {
                KnowledgeError::Storage(format!("proposal {id:?} vanished mid-sweep"))
            })?;
            // RE-VALIDATE at write time: the validators ran at ENQUEUE, but
            // the world may have changed (an endpoint deleted, or — the sharp
            // case — the same triple asserted by another path, which an
            // unconditional upsert would CLOBBER, overwriting that fact's
            // provenance: exactly the B7-review hazard). A proposal that no
            // longer validates is rejected here, never written.
            if let Err(error) = validate_kind(&proposal.kind, graph, tags).await {
                let reason = error.to_string();
                if let Some(proposal) = self.proposals.get_mut(&id) {
                    proposal.rejected = Some(reason.clone());
                    proposal
                        .history
                        .push(format!("{now} rejected at materialize: {reason}"));
                }
                report.rejected_at_materialize.push((id, reason));
                continue;
            }
            write_proposal(&proposal, graph, tags, now).await?;
            if let Some(proposal) = self.proposals.get_mut(&id) {
                proposal.materialized = true;
                proposal.history.push(format!("{now} materialized"));
            }
            report.materialized.push(id);
        }
        Ok(report)
    }

    /// Capped variant of [`materialize_ready`](Self::materialize_ready) for the
    /// K-L4 auto-sweep.
    ///
    /// Processes at most `cap` Ready proposals per call in deterministic
    /// (`BTreeMap` insertion order, i.e. ascending proposal id) order; the rest
    /// wait for the next sweep slot.  This is the **only** correct entry point
    /// for the sweep — calling the uncapped `materialize_ready` from an
    /// automated path would land the entire queue in one tick, defeating the
    /// pacing knob `sweep_cap` that operators tune.
    ///
    /// # Errors
    ///
    /// Same as [`materialize_ready`](Self::materialize_ready).
    pub async fn materialize_ready_capped(
        &mut self,
        cap: usize,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<MaterializeReport> {
        // Collect Ready ids in deterministic BTreeMap order, then truncate to cap.
        let ready: Vec<String> = self
            .proposals
            .values()
            .filter(|proposal| proposal.status == ValidationStatus::Ready && proposal.is_pending())
            .map(|proposal| proposal.id.clone())
            .take(cap)
            .collect();
        let mut report = MaterializeReport::default();
        for id in ready {
            let proposal = self.proposals.get(&id).cloned().ok_or_else(|| {
                KnowledgeError::Storage(format!("proposal {id:?} vanished mid-sweep"))
            })?;
            if let Err(error) = validate_kind(&proposal.kind, graph, tags).await {
                let reason = error.to_string();
                if let Some(proposal) = self.proposals.get_mut(&id) {
                    proposal.rejected = Some(reason.clone());
                    proposal
                        .history
                        .push(format!("{now} rejected at materialize: {reason}"));
                }
                report.rejected_at_materialize.push((id, reason));
                continue;
            }
            write_proposal(&proposal, graph, tags, now).await?;
            if let Some(proposal) = self.proposals.get_mut(&id) {
                proposal.materialized = true;
                proposal.history.push(format!("{now} materialized"));
            }
            report.materialized.push(id);
        }
        Ok(report)
    }

    /// Proposals, optionally filtered by agent. Returns a bounded
    /// [`ProposalPage`] (default [`LIST_PAGE_DEFAULT`], max [`LIST_PAGE_MAX`])
    /// with the total matching count.
    #[must_use]
    pub fn list(&self, agent_id: Option<&str>, limit: Option<usize>) -> ProposalPage<'_> {
        let effective_limit = limit.unwrap_or(LIST_PAGE_DEFAULT).min(LIST_PAGE_MAX);
        let all: Vec<&Proposal> = self
            .proposals
            .values()
            .filter(|proposal| agent_id.is_none_or(|agent| proposal.agent_id == agent))
            .collect();
        let total = all.len();
        let proposals = all.into_iter().take(effective_limit).collect();
        ProposalPage { proposals, total }
    }

    /// One proposal by id.
    #[must_use]
    pub fn get(&self, proposal_id: &str) -> Option<&Proposal> {
        self.proposals.get(proposal_id)
    }

    /// Truthful audit of every lesson (K-R1) currently in the queue.
    ///
    /// A lesson is an [`ProposalKind::Annotation`] whose note carries the
    /// [`crate::lessons::LESSON_NOTE_PREFIX`] marker. The reported
    /// [`crate::lessons::LessonState`] is derived ONLY from the backing
    /// proposal's real status — `Active` iff `materialized`, `Rejected` iff
    /// rejected, otherwise `Pending`. The ledger never claims an installation
    /// that did not actually happen (the `tdw.kg.why` truthfulness contract).
    ///
    /// Notes that fail to parse as a lesson are skipped (defensive: a non-lesson
    /// annotation that happens to share the marker is ignored, not surfaced as a
    /// malformed lesson).
    #[must_use]
    pub fn lessons_audit(&self) -> Vec<crate::lessons::LessonAudit> {
        use crate::lessons::{Lesson, LessonAudit, LessonState};
        let mut out = Vec::new();
        for proposal in self.proposals.values() {
            let ProposalKind::Annotation { note, .. } = &proposal.kind else {
                continue;
            };
            if !Lesson::note_is_lesson(note) {
                continue;
            }
            let Ok(lesson) = Lesson::from_note(note) else {
                continue;
            };
            let state = if proposal.materialized {
                LessonState::Active
            } else if proposal.rejected.is_some() {
                LessonState::Rejected
            } else {
                LessonState::Pending
            };
            out.push(LessonAudit {
                proposal_id: proposal.id.clone(),
                lesson,
                state,
            });
        }
        out
    }

    /// Counts of lessons by lifecycle state for `tdw.kg.status` (K-R1).
    ///
    /// `pending` counts lessons not yet materialized (and not rejected);
    /// `active` counts materialized (installed) lessons. Additive — does not
    /// alter the existing [`pending_counts_by_state`](Self::pending_counts_by_state)
    /// breakdown.
    #[must_use]
    pub fn lesson_counts(&self) -> crate::lessons::LessonCounts {
        use crate::lessons::{LessonCounts, LessonState};
        let mut counts = LessonCounts::default();
        for audit in self.lessons_audit() {
            match audit.state {
                LessonState::Pending => counts.pending += 1,
                LessonState::Active => counts.active += 1,
                LessonState::Rejected | LessonState::Retired => {}
            }
        }
        counts
    }

    /// Exact pending proposal counts broken down by [`ValidationStatus`].
    ///
    /// Unlike [`list`](Self::list), this is a single-pass scan over the full
    /// queue and is **not** subject to the `LIST_PAGE_DEFAULT` / `LIST_PAGE_MAX`
    /// pagination cap, so the counts are always exact regardless of queue depth.
    ///
    /// "Pending" means `!materialized && rejected.is_none()` — the same
    /// predicate as [`Proposal::is_pending`].  Proposals that have been
    /// materialized or rejected do not appear in any count.
    #[must_use]
    pub fn pending_counts_by_state(&self) -> (usize, usize, usize) {
        use tdw_taxonomy::ValidationStatus;
        let (mut draft, mut validated, mut ready) = (0usize, 0usize, 0usize);
        for proposal in self.proposals.values().filter(|p| p.is_pending()) {
            match proposal.status {
                ValidationStatus::Draft => draft += 1,
                ValidationStatus::Validated => validated += 1,
                ValidationStatus::Ready => ready += 1,
            }
        }
        (draft, validated, ready)
    }
}

/// The automated validators (gate step 2). Every check is loud.
async fn validate_kind(
    kind: &ProposalKind,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    match kind {
        ProposalKind::Edge { from, to, rel } => {
            // Shape validity via the shared edge grammar (probe edge).
            let probe = GraphEdge {
                from: from.clone(),
                to: to.clone(),
                rel: rel.clone(),
                props: serde_json::Value::Null,
                provenance: Provenance::Agent {
                    agent_id: "probe".to_string(),
                    gated: true,
                },
                valid_from: None,
                valid_to: None,
            };
            validate_graph_edge(&probe).map_err(storage)?;
            // Endpoints must exist (no stub creation through the gate).
            for endpoint in [from, to] {
                if graph.node(endpoint).await.map_err(storage)?.is_none() {
                    return Err(KnowledgeError::Storage(format!(
                        "edge endpoint {endpoint:?} does not exist"
                    )));
                }
            }
            // No re-assertion of an existing fact (B7 posture: agent writes
            // never clobber or duplicate existing triples).
            let mut offset = 0;
            loop {
                let page = graph.edges(Some(rel), offset, 256).await.map_err(storage)?;
                if page.is_empty() {
                    break;
                }
                offset += page.len();
                if page.iter().any(|edge| &edge.from == from && &edge.to == to) {
                    return Err(KnowledgeError::Storage(format!(
                        "edge {from} -{rel}-> {to} already exists"
                    )));
                }
            }
            Ok(())
        }
        ProposalKind::TagAssign { entity_id, tag_id } => {
            // The tag must be DEFINED — the gate never creates taxonomy as a
            // side effect of assignment.
            if !tag_exists(tags, tag_id).await? {
                return Err(KnowledgeError::Tag(format!("unknown tag {tag_id:?}")));
            }
            if graph.node(entity_id).await.map_err(storage)?.is_none() {
                return Err(KnowledgeError::Storage(format!(
                    "entity {entity_id:?} does not exist"
                )));
            }
            Ok(())
        }
        ProposalKind::TagDefine { tag_id, parent } => {
            validate_definition_shape(&TagDefinition {
                tag_id: tag_id.clone(),
                parent: parent.clone(),
                ttl_days: None,
            })
            .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
            if let Some(parent) = parent
                && !tag_exists(tags, parent).await?
            {
                return Err(KnowledgeError::Tag(format!(
                    "unknown parent tag {parent:?}"
                )));
            }
            if tag_exists(tags, tag_id).await? {
                return Err(KnowledgeError::Tag(format!(
                    "tag {tag_id:?} is already defined"
                )));
            }
            Ok(())
        }
        ProposalKind::Annotation { entity_id, note } => {
            if note.trim().is_empty() || note.chars().count() > MAX_NOTE_CHARS {
                return Err(KnowledgeError::Storage(format!(
                    "annotation note must be non-empty and at most {MAX_NOTE_CHARS} characters"
                )));
            }
            if note
                .chars()
                .any(|character| character.is_control() && character != '\n')
            {
                return Err(KnowledgeError::Storage(
                    "annotation note must not contain control characters".to_string(),
                ));
            }
            if graph.node(entity_id).await.map_err(storage)?.is_none() {
                return Err(KnowledgeError::Storage(format!(
                    "entity {entity_id:?} does not exist"
                )));
            }
            Ok(())
        }
    }
}

/// Tag existence via the engine contract's `is_defined`.
async fn tag_exists(tags: &Arc<dyn TagEngine>, tag_id: &str) -> Result<bool> {
    tags.is_defined(tag_id)
        .await
        .map_err(|error| KnowledgeError::Tag(error.to_string()))
}

/// Write one Ready proposal into the engines.
async fn write_proposal(
    proposal: &Proposal,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
    now: &str,
) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let provenance_text = format!("agent:{};proposal:{}", proposal.agent_id, proposal.id);
    match &proposal.kind {
        ProposalKind::Edge { from, to, rel } => graph
            .upsert_edges(vec![GraphEdge {
                from: from.clone(),
                to: to.clone(),
                rel: rel.clone(),
                props: serde_json::json!({ "proposal": proposal.id }),
                provenance: Provenance::Agent {
                    agent_id: proposal.agent_id.clone(),
                    gated: true,
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .map_err(storage),
        ProposalKind::TagAssign { entity_id, tag_id } => tags
            .assign(TagAssignment {
                entity_id: entity_id.clone(),
                tag_id: tag_id.clone(),
                assigned_at: now.to_string(),
                expires_at: None,
                provenance: provenance_text,
            })
            .await
            .map_err(|error| KnowledgeError::Tag(error.to_string())),
        ProposalKind::TagDefine { tag_id, parent } => tags
            .define(TagDefinition {
                tag_id: tag_id.clone(),
                parent: parent.clone(),
                ttl_days: None,
            })
            .await
            .map_err(|error| KnowledgeError::Tag(error.to_string())),
        ProposalKind::Annotation { entity_id, note } => {
            let annotation_id = format!("annotation:{}", proposal.id);
            graph
                .upsert_nodes(vec![GraphNode {
                    id: annotation_id.clone(),
                    kind: EntityKind::Document,
                    label: format!("annotation {}", proposal.id),
                    aliases: Vec::new(),
                    props: serde_json::json!({
                        "note": note,
                        "agent_id": proposal.agent_id,
                        "proposal": proposal.id,
                    }),
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .map_err(storage)?;
            graph
                .upsert_edges(vec![GraphEdge {
                    from: entity_id.clone(),
                    to: annotation_id,
                    rel: "annotated_by".to_string(),
                    props: serde_json::Value::Null,
                    provenance: Provenance::Agent {
                        agent_id: proposal.agent_id.clone(),
                        gated: true,
                    },
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .map_err(storage)
        }
    }
}
