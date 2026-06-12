//! Skill lifecycle + eval tournaments (knowledge-system-2 K-R2, Phase 4
//! "Real Learning").
//!
//! Skills are learned and refined, but **every lifecycle transition flows
//! through the SAME gated B9 pipeline** — never a direct write. A skill moves
//! `Candidate -> (eval tournament) -> Active | Retired`, and each transition is
//! a [`Proposal`](crate::proposals::Proposal) submitted through
//! [`ProposalQueue::submit`](crate::proposals::ProposalQueue::submit) with
//! provenance `Agent { gated: true }`. Nothing here writes into the graph or
//! tag engines directly; the queue's materialization sweep is the only landing
//! path, exactly as for B9 facts.
//!
//! ## The tournament (FAIL-CLOSED)
//!
//! [`SkillTournament::run`] makes `N` candidate skills compete on a **golden
//! task set** scored deterministically (a fixed seed; no clock, no RNG state
//! that survives a call). The promotion decision is a REAL out-of-sample score
//! and is **fail-closed**: an empty or insufficient golden set
//! (`< MIN_TOURNAMENT_TASKS`) promotes NOTHING and retires NOTHING — it returns
//! a [`TournamentOutcome::Insufficient`] verdict. This is the exact trap
//! K-R5/K-R1 fell into: a vacuous gate that "passes" on no evidence. The floor
//! here mirrors [`MIN_EVAL_CASES`](crate::proposals::MIN_EVAL_CASES) in spirit.
//!
//! ## Truthful audit
//!
//! A skill is reported `promoted/active` ONLY if it actually won the tournament
//! AND its promotion proposal was enqueued. The [`SkillsAudit`] records every
//! transition with the deterministic score that justified it, so
//! `tdw.kg.why` on the skill node (the annotation the proposal materializes)
//! and the audit agree. There is no path that flips a skill to `Active` without
//! a recorded winning score.
//!
//! ## Retirement / rollback
//!
//! A regressing `Active` skill is retired the same way: [`SkillTournament`]
//! scores it against the golden set, and a score below the retire floor enqueues
//! a retire proposal through the gate. Retirement is never a silent local flip.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::GraphEngine;
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

use crate::proposals::{Proposal, ProposalKind, ProposalQueue};
use crate::{KnowledgeError, Result};

/// Minimum number of golden tasks required for a tournament to render a verdict.
///
/// Below this the tournament is **insufficient** and promotes nothing — the
/// fail-closed floor (cf. [`MIN_EVAL_CASES`](crate::proposals::MIN_EVAL_CASES)).
pub const MIN_TOURNAMENT_TASKS: usize = 3;

/// Default minimum mean score a candidate must reach to be eligible for
/// promotion. Scores are in `[0.0, 1.0]`.
pub const PROMOTE_THRESHOLD: f64 = 0.6;

/// Default score at or below which an `Active` skill is retired.
pub const RETIRE_THRESHOLD: f64 = 0.3;

/// Maximum length of a skill id (mirrors the agent-id bound so provenance
/// strings stay bounded).
pub const MAX_SKILL_ID_LEN: usize = 128;

/// Where a skill sits in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLifecycle {
    /// Newly learned/refined; not yet promoted. Competes in tournaments.
    Candidate,
    /// Won a tournament and was installed (the promotion proposal was enqueued
    /// through B9). The active, selectable skill.
    Active,
    /// Regressed or lost; retired through the gate. Terminal for this version.
    Retired,
}

impl SkillLifecycle {
    /// The lowercase token used in audit/provenance strings.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Active => "active",
            Self::Retired => "retired",
        }
    }
}

/// One golden task: a deterministic prompt plus the substrings to credit.
///
/// The skill's output must contain every `expect_contains` substring. Scoring is
/// a containment ratio — honest and deterministic, never an LLM judge (mirrors
/// `tdw-eval-runner::score_case`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenTask {
    /// Stable task id (audit/debug).
    pub task_id: String,
    /// The prompt fed to the skill under evaluation.
    pub prompt: String,
    /// Substrings the skill's response must contain. Empty = any non-empty
    /// response scores 1.0.
    pub expect_contains: Vec<String>,
}

/// A skill under management, with its lifecycle state and behavior.
///
/// The `produce` closure is the skill's deterministic behavior over a prompt —
/// injected so the tournament executes the REAL skill (per the requirement that
/// the eval EXECUTES skills), with no network.
pub struct ManagedSkill {
    id: String,
    lifecycle: SkillLifecycle,
    /// Deterministic skill behavior: prompt -> response. Injected so the
    /// tournament scores the real artifact, not a stub. Must be referentially
    /// transparent for determinism (no clock, no RNG).
    produce: Arc<dyn Fn(&str) -> String + Send + Sync>,
}

impl ManagedSkill {
    /// Build a managed skill in the `Candidate` state.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] when `id` is empty or too long.
    pub fn new_candidate(
        id: impl Into<String>,
        produce: Arc<dyn Fn(&str) -> String + Send + Sync>,
    ) -> Result<Self> {
        let id = id.into();
        validate_skill_id(&id)?;
        Ok(Self {
            id,
            lifecycle: SkillLifecycle::Candidate,
            produce,
        })
    }

    /// The skill id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The current lifecycle state.
    #[must_use]
    pub const fn lifecycle(&self) -> SkillLifecycle {
        self.lifecycle
    }
}

impl std::fmt::Debug for ManagedSkill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedSkill")
            .field("id", &self.id)
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

/// Validate a skill id against the same grammar B9 uses for agent ids.
///
/// The grammar is `[A-Za-z0-9:._-]`, non-empty, bounded. Skill ids become part
/// of the `agent:<id>;proposal:<pid>` provenance only indirectly (the proposing
/// agent id is separate), but bounding them keeps audit strings sane.
///
/// # Errors
///
/// Returns [`KnowledgeError::Storage`] when the id is empty, too long, or
/// contains a character outside the grammar.
pub fn validate_skill_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(KnowledgeError::Storage(
            "skill id must not be empty".to_string(),
        ));
    }
    if id.len() > MAX_SKILL_ID_LEN {
        return Err(KnowledgeError::Storage(format!(
            "skill id is too long ({} bytes, max {MAX_SKILL_ID_LEN})",
            id.len()
        )));
    }
    if let Some(bad) = id
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(KnowledgeError::Storage(format!(
            "skill id {id:?} contains invalid character {bad:?} — only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

/// Per-skill score in a tournament: the mean containment ratio over the golden
/// task set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillScore {
    /// The scored skill's id.
    pub skill_id: String,
    /// Mean score over the golden tasks, in `[0.0, 1.0]`.
    pub mean_score: f64,
    /// Number of golden tasks scored (always the full set; recorded for audit).
    pub tasks_scored: usize,
}

/// The verdict of one tournament run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TournamentOutcome {
    /// FAIL-CLOSED: fewer than [`MIN_TOURNAMENT_TASKS`] golden tasks. Nothing is
    /// promoted or retired. Carries the deficient task count for audit.
    Insufficient {
        /// How many golden tasks were supplied (below the floor).
        tasks: usize,
        /// The floor that was not met.
        floor: usize,
    },
    /// A real verdict was rendered. Carries the full scoreboard plus the
    /// promote/retire decisions derived from it.
    Decided {
        /// Per-skill scores, sorted descending by `mean_score` then ascending by
        /// id (deterministic tie-break).
        scoreboard: Vec<SkillScore>,
        /// The winning skill id, when one cleared [`SkillTournament::promote_threshold`].
        /// `None` when no candidate met the bar (still a real verdict, not a
        /// vacuous pass).
        winner: Option<String>,
        /// Active skills whose score fell at/below the retire floor.
        retired: Vec<String>,
    },
}

/// One recorded lifecycle transition for the truthful audit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SkillAuditEntry {
    /// The skill that transitioned.
    pub skill_id: String,
    /// State before.
    pub from: SkillLifecycle,
    /// State after.
    pub to: SkillLifecycle,
    /// The deterministic mean score that justified the transition.
    pub score: f64,
    /// The id of the B9 proposal that carries this transition (proof it went
    /// through the gate, not a direct write).
    pub proposal_id: String,
    /// Injected `now` (date or RFC3339) at transition time.
    pub at: String,
}

/// Append-only truthful audit of every lifecycle transition (the `skills_audit`).
///
/// A skill is reported promoted/active ONLY via an entry here whose `to` is
/// `Active` and whose `score` cleared the promote threshold — there is no other
/// path. Mirrors the proposal `history` audit posture.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillsAudit {
    entries: Vec<SkillAuditEntry>,
}

impl SkillsAudit {
    /// Record a transition. Called only after the gated proposal is enqueued.
    pub fn record(&mut self, entry: SkillAuditEntry) {
        self.entries.push(entry);
    }

    /// All recorded transitions, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[SkillAuditEntry] {
        &self.entries
    }

    /// Transitions for one skill, oldest first.
    #[must_use]
    pub fn for_skill<'a>(&'a self, skill_id: &str) -> Vec<&'a SkillAuditEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.skill_id == skill_id)
            .collect()
    }
}

/// Skill counts by lifecycle state, surfaced additively in `tdw.kg.status`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCounts {
    /// Skills awaiting/competing in a tournament.
    pub candidate: usize,
    /// Promoted, selectable skills.
    pub active: usize,
    /// Retired skills.
    pub retired: usize,
}

/// The registry of managed skills plus the gated lifecycle machine.
///
/// The registry NEVER writes to the graph/tag engines itself. Lifecycle
/// transitions are enqueued as B9 proposals; the queue's materialization is the
/// only landing path. The registry keeps the in-process lifecycle state in sync
/// with what was *proposed* and records the audit.
#[derive(Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, ManagedSkill>,
    audit: SkillsAudit,
}

impl std::fmt::Debug for SkillRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillRegistry")
            .field("skills", &self.skills.len())
            .field("audit_entries", &self.audit.entries.len())
            .finish()
    }
}

impl SkillRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a candidate skill. Overwrites any prior skill of the same id
    /// (re-learning a skill resets it to candidate).
    ///
    /// # Errors
    ///
    /// Returns an error when the skill id is invalid.
    pub fn register_candidate(
        &mut self,
        id: impl Into<String>,
        produce: Arc<dyn Fn(&str) -> String + Send + Sync>,
    ) -> Result<()> {
        let skill = ManagedSkill::new_candidate(id, produce)?;
        self.skills.insert(skill.id.clone(), skill);
        Ok(())
    }

    /// Current lifecycle of a skill, if registered.
    #[must_use]
    pub fn lifecycle(&self, skill_id: &str) -> Option<SkillLifecycle> {
        self.skills.get(skill_id).map(ManagedSkill::lifecycle)
    }

    /// Exact counts by lifecycle state (single pass; not paginated).
    #[must_use]
    pub fn counts(&self) -> SkillCounts {
        let mut counts = SkillCounts::default();
        for skill in self.skills.values() {
            match skill.lifecycle {
                SkillLifecycle::Candidate => counts.candidate += 1,
                SkillLifecycle::Active => counts.active += 1,
                SkillLifecycle::Retired => counts.retired += 1,
            }
        }
        counts
    }

    /// The truthful audit.
    #[must_use]
    pub const fn audit(&self) -> &SkillsAudit {
        &self.audit
    }

    /// Ids of skills currently in a given state, sorted.
    #[must_use]
    pub fn ids_in(&self, state: SkillLifecycle) -> Vec<String> {
        self.skills
            .values()
            .filter(|skill| skill.lifecycle == state)
            .map(|skill| skill.id.clone())
            .collect()
    }

    /// Run a tournament over the candidate skills and the golden task set, then
    /// enqueue the resulting promote/retire transitions through the B9 gate.
    ///
    /// This is the ONLY way a skill becomes `Active` or `Retired`. The flow:
    ///
    /// 1. Score every candidate (and, for retirement, every active) skill over
    ///    the golden set deterministically.
    /// 2. FAIL-CLOSED: an insufficient golden set promotes/retires NOTHING.
    /// 3. The winner (if any cleared the promote threshold) gets a promote
    ///    proposal; active skills at/below the retire floor get retire proposals.
    /// 4. Each enqueued transition updates the in-process lifecycle and records
    ///    a truthful audit entry referencing the proposal id.
    ///
    /// `proposer_id` is the gated agent the proposals are attributed to;
    /// `adaptivity` must be at least `Learning` (the queue enforces this). The
    /// proposals are [`ProposalKind::Annotation`] on the skill's entity node, so
    /// `tdw.kg.why` explains the transition once materialized. The skill node
    /// must already exist in the graph (the B9 annotation validator requires it).
    ///
    /// # Errors
    ///
    /// Returns the first gate error (admission, validators, cap). When the
    /// tournament is insufficient, returns `Ok` with no transitions — that is a
    /// verdict, not an error.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_tournament_gated(
        &mut self,
        tournament: &SkillTournament,
        golden: &[GoldenTask],
        skill_node_id: impl Fn(&str) -> String + Send + Sync,
        proposer_id: &str,
        adaptivity: Adaptivity,
        queue: &Arc<tokio::sync::Mutex<ProposalQueue>>,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<TournamentOutcome> {
        let candidates: Vec<&ManagedSkill> = self
            .skills
            .values()
            .filter(|s| s.lifecycle == SkillLifecycle::Candidate)
            .collect();
        let actives: Vec<&ManagedSkill> = self
            .skills
            .values()
            .filter(|s| s.lifecycle == SkillLifecycle::Active)
            .collect();

        let outcome = tournament.evaluate(&candidates, &actives, golden);
        let TournamentOutcome::Decided {
            ref winner,
            ref retired,
            ..
        } = outcome
        else {
            // Insufficient → fail-closed: nothing transitions.
            return Ok(outcome);
        };

        // Collect the deterministic scores so the audit records the exact value
        // that justified each transition.
        let scoreboard = if let TournamentOutcome::Decided { scoreboard, .. } = &outcome {
            scoreboard.clone()
        } else {
            Vec::new()
        };
        let score_of = |skill_id: &str| -> f64 {
            scoreboard
                .iter()
                .find(|s| s.skill_id == skill_id)
                .map_or(0.0, |s| s.mean_score)
        };

        // Promote the winner (Candidate -> Active) through the gate.
        if let Some(winner_id) = winner.clone() {
            let score = score_of(&winner_id);
            let proposal = self
                .enqueue_transition(
                    &winner_id,
                    SkillLifecycle::Active,
                    score,
                    &skill_node_id,
                    proposer_id,
                    adaptivity,
                    queue,
                    graph,
                    tags,
                    now,
                )
                .await?;
            self.apply_transition(&winner_id, SkillLifecycle::Active, score, &proposal.id, now);
        }

        // Retire regressing active skills (Active -> Retired) through the gate.
        for retire_id in retired.clone() {
            let score = score_of(&retire_id);
            let proposal = self
                .enqueue_transition(
                    &retire_id,
                    SkillLifecycle::Retired,
                    score,
                    &skill_node_id,
                    proposer_id,
                    adaptivity,
                    queue,
                    graph,
                    tags,
                    now,
                )
                .await?;
            self.apply_transition(
                &retire_id,
                SkillLifecycle::Retired,
                score,
                &proposal.id,
                now,
            );
        }

        Ok(outcome)
    }

    /// Enqueue ONE lifecycle transition as a gated B9 annotation proposal. This
    /// is the single choke point — there is no direct-write alternative.
    #[allow(clippy::too_many_arguments)]
    async fn enqueue_transition(
        &self,
        skill_id: &str,
        to: SkillLifecycle,
        score: f64,
        skill_node_id: &(impl Fn(&str) -> String + Send + Sync),
        proposer_id: &str,
        adaptivity: Adaptivity,
        queue: &Arc<tokio::sync::Mutex<ProposalQueue>>,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<Proposal> {
        let from = self
            .skills
            .get(skill_id)
            .map_or(SkillLifecycle::Candidate, ManagedSkill::lifecycle);
        let entity_id = skill_node_id(skill_id);
        // The audit note is structured so tdw.kg.why surfaces a truthful,
        // machine-readable record once materialized.
        let note = serde_json::json!({
            "skills_audit": {
                "skill_id": skill_id,
                "from": from.token(),
                "to": to.token(),
                "score": score,
                "at": now,
            }
        })
        .to_string();
        let mut queue = queue.lock().await;
        queue
            .submit(
                proposer_id,
                adaptivity,
                ProposalKind::Annotation { entity_id, note },
                graph,
                tags,
                now,
            )
            .await
    }

    /// Update in-process lifecycle + record the truthful audit AFTER the gated
    /// proposal was enqueued. Never called on a failed enqueue.
    fn apply_transition(
        &mut self,
        skill_id: &str,
        to: SkillLifecycle,
        score: f64,
        proposal_id: &str,
        now: &str,
    ) {
        let Some(skill) = self.skills.get_mut(skill_id) else {
            return;
        };
        let from = skill.lifecycle;
        skill.lifecycle = to;
        self.audit.record(SkillAuditEntry {
            skill_id: skill_id.to_string(),
            from,
            to,
            score,
            proposal_id: proposal_id.to_string(),
            at: now.to_string(),
        });
    }
}

/// The deterministic, fail-closed tournament.
///
/// `evaluate` executes each skill over the golden set (calling the skill's
/// injected `produce` closure — the REAL artifact, no stub), scores it with a
/// containment ratio, and renders a verdict. Determinism: there is no clock and
/// no RNG; given the same skills and golden set the scoreboard is byte-identical
/// every run (the "fixed seed" requirement — scoring is a pure function of the
/// inputs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkillTournament {
    promote_threshold: f64,
    retire_threshold: f64,
    min_tasks: usize,
}

impl Default for SkillTournament {
    fn default() -> Self {
        Self {
            promote_threshold: PROMOTE_THRESHOLD,
            retire_threshold: RETIRE_THRESHOLD,
            min_tasks: MIN_TOURNAMENT_TASKS,
        }
    }
}

impl SkillTournament {
    /// A tournament with explicit thresholds. `promote_threshold` must be
    /// strictly greater than `retire_threshold` so the two decisions can never
    /// overlap; otherwise the defaults are used (the constructor is total, never
    /// panics — an out-of-order pair silently falls back to defaults so a
    /// misconfiguration cannot create a degenerate gate).
    #[must_use]
    pub fn new(promote_threshold: f64, retire_threshold: f64, min_tasks: usize) -> Self {
        if promote_threshold <= retire_threshold
            || !(0.0..=1.0).contains(&promote_threshold)
            || !(0.0..=1.0).contains(&retire_threshold)
            || min_tasks == 0
        {
            return Self::default();
        }
        Self {
            promote_threshold,
            retire_threshold,
            min_tasks,
        }
    }

    /// The promotion bar.
    #[must_use]
    pub const fn promote_threshold(&self) -> f64 {
        self.promote_threshold
    }

    /// The retirement floor.
    #[must_use]
    pub const fn retire_threshold(&self) -> f64 {
        self.retire_threshold
    }

    /// Score `candidates` (for promotion) and `actives` (for retirement) over
    /// the golden set and render a verdict.
    ///
    /// FAIL-CLOSED: `golden.len() < min_tasks` ⇒ [`TournamentOutcome::Insufficient`]
    /// — nothing is promoted or retired. An empty candidate+active set with a
    /// sufficient golden set yields a `Decided` verdict with `winner: None` and
    /// `retired: []` (a real, honest "no transition" outcome).
    #[must_use]
    pub fn evaluate(
        &self,
        candidates: &[&ManagedSkill],
        actives: &[&ManagedSkill],
        golden: &[GoldenTask],
    ) -> TournamentOutcome {
        if golden.len() < self.min_tasks {
            return TournamentOutcome::Insufficient {
                tasks: golden.len(),
                floor: self.min_tasks,
            };
        }

        // Score every competing skill (candidates + actives) deterministically.
        let mut scoreboard: Vec<SkillScore> = candidates
            .iter()
            .chain(actives.iter())
            .map(|skill| SkillScore {
                skill_id: skill.id.clone(),
                mean_score: mean_score(skill, golden),
                tasks_scored: golden.len(),
            })
            .collect();
        // Deterministic order: descending score, then ascending id (stable
        // tie-break so the winner pick never depends on map/iteration luck).
        scoreboard.sort_by(|a, b| {
            b.mean_score
                .partial_cmp(&a.mean_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.skill_id.cmp(&b.skill_id))
        });

        // Winner: the highest-scoring CANDIDATE that cleared the promote bar.
        let candidate_ids: std::collections::BTreeSet<&str> =
            candidates.iter().map(|s| s.id.as_str()).collect();
        let winner = scoreboard
            .iter()
            .find(|s| {
                candidate_ids.contains(s.skill_id.as_str())
                    && s.mean_score >= self.promote_threshold
            })
            .map(|s| s.skill_id.clone());

        // Retire: ACTIVE skills at/below the retire floor.
        let active_ids: std::collections::BTreeSet<&str> =
            actives.iter().map(|s| s.id.as_str()).collect();
        let mut retired: Vec<String> = scoreboard
            .iter()
            .filter(|s| {
                active_ids.contains(s.skill_id.as_str()) && s.mean_score <= self.retire_threshold
            })
            .map(|s| s.skill_id.clone())
            .collect();
        retired.sort();

        TournamentOutcome::Decided {
            scoreboard,
            winner,
            retired,
        }
    }
}

/// Mean containment score for one skill over the golden set, in `[0.0, 1.0]`.
fn mean_score(skill: &ManagedSkill, golden: &[GoldenTask]) -> f64 {
    if golden.is_empty() {
        return 0.0;
    }
    let total: f64 = golden.iter().map(|task| score_task(skill, task)).sum();
    // golden.len() is bounded by the caller's golden set; far within f64 exact range.
    #[allow(clippy::cast_precision_loss)]
    let n = golden.len() as f64;
    total / n
}

/// Score one golden task: the fraction of `expect_contains` substrings present
/// in the skill's response. No `expect_contains` ⇒ 1.0 iff the response is
/// non-empty, else 0.0. Deterministic and honest — a containment check, never
/// an LLM judge.
fn score_task(skill: &ManagedSkill, task: &GoldenTask) -> f64 {
    let response = (skill.produce)(&task.prompt);
    if task.expect_contains.is_empty() {
        return if response.trim().is_empty() { 0.0 } else { 1.0 };
    }
    let hits = task
        .expect_contains
        .iter()
        .filter(|needle| response.contains(needle.as_str()))
        .count();
    // hits and len are small bounded counts; exact in f64.
    #[allow(clippy::cast_precision_loss)]
    let ratio = hits as f64 / task.expect_contains.len() as f64;
    ratio
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tdw_core::GraphEngine;
    use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
    use tdw_taxonomy::{Adaptivity, EntityKind};

    use super::*;
    use crate::proposals::ProposalQueue;

    const NOW: &str = "2026-06-12";

    fn echo_skill(id: &str, canned: &'static str) -> Arc<dyn Fn(&str) -> String + Send + Sync> {
        let _ = id;
        Arc::new(move |_prompt: &str| canned.to_string())
    }

    fn golden(n: usize) -> Vec<GoldenTask> {
        (0..n)
            .map(|i| GoldenTask {
                task_id: format!("t{i}"),
                prompt: format!("prompt {i}"),
                expect_contains: vec!["GOOD".to_string()],
            })
            .collect()
    }

    /// Build a graph with a node for each skill id (the B9 annotation validator
    /// requires the entity to exist) and a fresh tag engine + queue.
    async fn fixture(
        skill_ids: &[&str],
    ) -> (
        Arc<dyn GraphEngine>,
        Arc<dyn TagEngine>,
        Arc<tokio::sync::Mutex<ProposalQueue>>,
    ) {
        let graph_concrete = InMemoryGraphEngine::default();
        for id in skill_ids {
            graph_concrete
                .upsert_nodes(vec![tdw_core::GraphNode {
                    id: skill_node(id),
                    kind: EntityKind::Document,
                    label: format!("skill {id}"),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .expect("seed skill node");
        }
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_concrete);
        let tags: Arc<dyn TagEngine> =
            Arc::new(GraphTagEngine::new(InMemoryGraphEngine::default()));
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        (graph, tags, queue)
    }

    fn skill_node(skill_id: &str) -> String {
        format!("skill:{skill_id}")
    }

    #[test]
    fn fail_closed_on_insufficient_golden_set() {
        let t = SkillTournament::default();
        let winner =
            ManagedSkill::new_candidate("a", echo_skill("a", "GOOD")).expect("test op succeeds");
        let cands = [&winner];
        // Two tasks < MIN_TOURNAMENT_TASKS (3): insufficient, promotes nothing.
        let outcome = t.evaluate(&cands, &[], &golden(2));
        assert_eq!(
            outcome,
            TournamentOutcome::Insufficient { tasks: 2, floor: 3 }
        );
    }

    #[test]
    fn empty_golden_set_is_insufficient_not_vacuous_pass() {
        let t = SkillTournament::default();
        let winner =
            ManagedSkill::new_candidate("a", echo_skill("a", "GOOD")).expect("test op succeeds");
        let outcome = t.evaluate(&[&winner], &[], &[]);
        assert!(matches!(outcome, TournamentOutcome::Insufficient { .. }));
    }

    #[test]
    fn winner_is_highest_scoring_candidate_over_bar() {
        let t = SkillTournament::default();
        let good = ManagedSkill::new_candidate("good", echo_skill("good", "GOOD answer"))
            .expect("test op succeeds");
        let bad = ManagedSkill::new_candidate("bad", echo_skill("bad", "nope"))
            .expect("test op succeeds");
        let outcome = t.evaluate(&[&good, &bad], &[], &golden(5));
        let TournamentOutcome::Decided { winner, .. } = outcome else {
            panic!("expected decided");
        };
        assert_eq!(winner, Some("good".to_string()));
    }

    #[test]
    fn no_candidate_over_bar_yields_no_winner_but_real_verdict() {
        let t = SkillTournament::default();
        let bad = ManagedSkill::new_candidate("bad", echo_skill("bad", "nope"))
            .expect("test op succeeds");
        let outcome = t.evaluate(&[&bad], &[], &golden(5));
        let TournamentOutcome::Decided {
            winner, retired, ..
        } = outcome
        else {
            panic!("expected decided");
        };
        assert_eq!(winner, None, "below-bar candidate must not win");
        assert!(retired.is_empty());
    }

    #[tokio::test]
    async fn lifecycle_transition_enqueues_proposal_not_direct_write() {
        let (graph, tags, queue) = fixture(&["good"]).await;
        let mut reg = SkillRegistry::new();
        reg.register_candidate("good", echo_skill("good", "GOOD answer"))
            .expect("test op succeeds");

        let t = SkillTournament::default();
        let outcome = reg
            .run_tournament_gated(
                &t,
                &golden(5),
                skill_node,
                "skill-curator",
                Adaptivity::Learning,
                &queue,
                &graph,
                &tags,
                NOW,
            )
            .await
            .expect("gated tournament");

        assert!(matches!(outcome, TournamentOutcome::Decided { .. }));
        // The transition went through the queue as a Validated proposal — NOT a
        // direct graph write. The skill node has no materialized annotation yet.
        // Extract the proposal snapshot in a scoped block so the lock is released
        // before the remaining (lock-free) assertions.
        let proposals: Vec<Proposal> = {
            let guard = queue.lock().await;
            guard
                .list(None, None)
                .proposals
                .into_iter()
                .cloned()
                .collect()
        };
        assert_eq!(
            proposals.len(),
            1,
            "exactly one transition proposal enqueued"
        );
        let proposal = &proposals[0];
        assert!(
            matches!(proposal.kind, ProposalKind::Annotation { .. }),
            "transition is an annotation proposal"
        );
        assert!(!proposal.materialized, "proposal is pending, not written");
        let proposal_id = &proposal.id;
        // And the audit truthfully reports the promotion with the winning score.
        let entries = reg.audit().for_skill("good");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].to, SkillLifecycle::Active);
        assert_eq!(&entries[0].proposal_id, proposal_id);
        assert!(entries[0].score >= t.promote_threshold());
        assert_eq!(reg.lifecycle("good"), Some(SkillLifecycle::Active));
    }

    #[tokio::test]
    async fn empty_tournament_data_promotes_nothing_fail_closed() {
        let (graph, tags, queue) = fixture(&["good"]).await;
        let mut reg = SkillRegistry::new();
        reg.register_candidate("good", echo_skill("good", "GOOD answer"))
            .expect("test op succeeds");

        let t = SkillTournament::default();
        // Empty golden set: insufficient → NO promotion, NO proposal.
        let outcome = reg
            .run_tournament_gated(
                &t,
                &[],
                skill_node,
                "skill-curator",
                Adaptivity::Learning,
                &queue,
                &graph,
                &tags,
                NOW,
            )
            .await
            .expect("insufficient is a verdict, not an error");

        assert!(matches!(outcome, TournamentOutcome::Insufficient { .. }));
        let total = {
            let q = queue.lock().await;
            q.list(None, None).total
        };
        assert_eq!(total, 0, "no proposal enqueued");
        assert_eq!(
            reg.lifecycle("good"),
            Some(SkillLifecycle::Candidate),
            "skill stays candidate — nothing promoted on no evidence"
        );
        assert!(reg.audit().entries().is_empty(), "no audit entry");
    }

    #[tokio::test]
    async fn passing_tournament_promotes_winner_and_audit_is_truthful() {
        let (graph, tags, queue) = fixture(&["alpha", "beta"]).await;
        let mut reg = SkillRegistry::new();
        // alpha is good, beta is bad.
        reg.register_candidate("alpha", echo_skill("alpha", "GOOD result"))
            .expect("test op succeeds");
        reg.register_candidate("beta", echo_skill("beta", "irrelevant"))
            .expect("test op succeeds");

        let t = SkillTournament::default();
        let outcome = reg
            .run_tournament_gated(
                &t,
                &golden(5),
                skill_node,
                "skill-curator",
                Adaptivity::Learning,
                &queue,
                &graph,
                &tags,
                NOW,
            )
            .await
            .expect("gated tournament");

        let TournamentOutcome::Decided {
            winner, scoreboard, ..
        } = outcome
        else {
            panic!("expected decided");
        };
        assert_eq!(winner, Some("alpha".to_string()));
        // alpha promoted to active; beta stays candidate (lost, but not retired
        // — only ACTIVE skills are retired).
        assert_eq!(reg.lifecycle("alpha"), Some(SkillLifecycle::Active));
        assert_eq!(reg.lifecycle("beta"), Some(SkillLifecycle::Candidate));
        // Audit reports alpha active with the exact winning score from the board.
        let alpha_score = scoreboard
            .iter()
            .find(|s| s.skill_id == "alpha")
            .map(|s| s.mean_score)
            .expect("test op succeeds");
        let entry = reg.audit().for_skill("alpha")[0];
        assert!((entry.score - alpha_score).abs() < f64::EPSILON);
        // Only the truthful winner is reported active.
        let counts = reg.counts();
        assert_eq!(counts.active, 1);
        assert_eq!(counts.candidate, 1);
        assert_eq!(counts.retired, 0);
    }

    #[tokio::test]
    async fn regressing_active_skill_is_retired_through_gate() {
        let (graph, tags, queue) = fixture(&["stale"]).await;
        let mut reg = SkillRegistry::new();
        // A skill that scores 0 (regressed): register then force it active by a
        // first winning tournament would be circular, so seed it active via a
        // passing run, then regress its behavior.
        reg.register_candidate("stale", echo_skill("stale", "GOOD"))
            .expect("test op succeeds");
        let t = SkillTournament::default();
        reg.run_tournament_gated(
            &t,
            &golden(5),
            skill_node,
            "curator",
            Adaptivity::Learning,
            &queue,
            &graph,
            &tags,
            NOW,
        )
        .await
        .expect("test op succeeds");
        assert_eq!(reg.lifecycle("stale"), Some(SkillLifecycle::Active));

        // Now the skill regresses: re-register the SAME id with bad behavior...
        // but re-register resets to candidate. Instead, model regression by a
        // tournament where the active skill now scores 0. We simulate by
        // replacing its produce via a fresh registry entry kept Active.
        // Simplest faithful model: directly evaluate retirement path.
        let bad = ManagedSkill::new_candidate("stale", echo_skill("stale", "broken"))
            .expect("test op succeeds");
        // Manually mark active to model an in-place regression of the live skill.
        let mut active_reg = SkillRegistry::new();
        active_reg.skills.insert("stale".to_string(), bad);
        if let Some(s) = active_reg.skills.get_mut("stale") {
            s.lifecycle = SkillLifecycle::Active;
        }
        let outcome = active_reg
            .run_tournament_gated(
                &t,
                &golden(5),
                skill_node,
                "curator",
                Adaptivity::Learning,
                &queue,
                &graph,
                &tags,
                NOW,
            )
            .await
            .expect("test op succeeds");
        let TournamentOutcome::Decided { retired, .. } = outcome else {
            panic!("decided");
        };
        assert_eq!(retired, vec!["stale".to_string()]);
        assert_eq!(active_reg.lifecycle("stale"), Some(SkillLifecycle::Retired));
        let entry = active_reg.audit().for_skill("stale")[0];
        assert_eq!(entry.from, SkillLifecycle::Active);
        assert_eq!(entry.to, SkillLifecycle::Retired);
    }

    #[tokio::test]
    async fn below_learning_adaptivity_is_rejected_by_gate() {
        let (graph, tags, queue) = fixture(&["good"]).await;
        let mut reg = SkillRegistry::new();
        reg.register_candidate("good", echo_skill("good", "GOOD answer"))
            .expect("test op succeeds");
        let t = SkillTournament::default();
        // Adaptivity::None is below Learning — the B9 admission gate rejects the
        // promote proposal loudly; nothing is promoted.
        let result = reg
            .run_tournament_gated(
                &t,
                &golden(5),
                skill_node,
                "good",
                Adaptivity::None,
                &queue,
                &graph,
                &tags,
                NOW,
            )
            .await;
        assert!(result.is_err(), "below-Learning admission must be rejected");
        assert_eq!(
            reg.lifecycle("good"),
            Some(SkillLifecycle::Candidate),
            "rejected transition does not flip lifecycle"
        );
    }

    #[test]
    fn determinism_same_inputs_same_scoreboard() {
        let t = SkillTournament::default();
        let a =
            ManagedSkill::new_candidate("a", echo_skill("a", "GOOD")).expect("test op succeeds");
        let b =
            ManagedSkill::new_candidate("b", echo_skill("b", "GOOD")).expect("test op succeeds");
        let g = golden(4);
        let first = t.evaluate(&[&a, &b], &[], &g);
        let second = t.evaluate(&[&a, &b], &[], &g);
        assert_eq!(first, second, "scoring is a pure function of the inputs");
    }

    #[test]
    fn skill_id_grammar_is_enforced() {
        assert!(validate_skill_id("good-skill.v1").is_ok());
        assert!(validate_skill_id("").is_err());
        assert!(validate_skill_id("bad id").is_err());
        assert!(validate_skill_id("bad;id").is_err());
    }
}
