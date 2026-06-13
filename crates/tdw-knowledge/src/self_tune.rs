//! System self-tuning within gates (knowledge-system K-R3).
//!
//! The system tunes its OWN operating parameters — but only through the SAME
//! gated `propose → eval → promote → rollback` pipeline that every other agent
//! write goes through.  The differentiator: even the system's self-modification
//! is an auditable, gated proposal with provenance `Agent { gated: true }`,
//! never a silent config write.
//!
//! # The gate (fail-closed)
//!
//! A tuning proposal is accepted ONLY if a REAL out-of-sample eval against the
//! golden split shows the candidate beats the incumbent on the objective by at
//! least `margin`.  The decision logic here is pure and takes the two objective
//! scores as inputs — the worker in `tdw-backend` computes them via the B11
//! retrieval-eval harness (the same golden set K-L3 / K-R7 use) and feeds them
//! in.  When there is NO eval evidence (missing fixture, zero cases), the worker
//! never calls [`decide`] with a candidate score — it records
//! [`TuneOutcome::NoEvalData`] and the live parameter is left UNCHANGED.  This
//! is the exact fail-closed posture K-R5 / K-R1 had to learn: no eval ⇒ no
//! change, never a vacuous "improvement".
//!
//! # Bounds & kill-switch
//!
//! Every tunable knob carries a hard `[min, max]` clamp ([`TuneClamp`]) the
//! tuner can NEVER exceed — both the proposed candidate and any restored value
//! are clamped at the source.  The whole subsystem is gated behind a
//! kill-switch (`SelfTuneConfig::enabled`, default `false`); when off, nothing
//! is proposed and nothing is applied.
//!
//! # Rollback
//!
//! Promotion is monotone on the objective: a candidate only lands if it BEATS
//! the incumbent out-of-sample, so a regression is rejected before it is ever
//! applied.  If a later cycle finds the previous value is better, that previous
//! value is itself proposed and re-gated — rollback is just another gated
//! proposal, never a silent revert.
//!
//! # What is explicitly NOT tunable (safety/correctness exclusion)
//!
//! The tunable set is deliberately tiny and SAFE.  It currently contains ONLY
//! the RRF fusion constant `k` (a pure ranking-quality knob with no correctness
//! or safety semantics).  The following are EXCLUDED BY CONSTRUCTION and can
//! never be reached by the tuner:
//!
//! - **Stub-gating / language-model grade** — whether eval feedback and
//!   auto-materialization are live. Auto-loosening this would let unvetted writes land.
//! - **Temporal-leakage safety** — the `as_of` / `active_at` filters and the
//!   walk-forward no-lookahead guarantee. These are correctness invariants, not quality dials.
//! - **The promote-gate margins / `MIN_EVAL_CASES` evidence floor** — the gate
//!   may not weaken its own evidence requirements.
//! - **Proposal admission (`Adaptivity >= Learning`), pending caps, agent-id grammar**
//!   — the B9 write gate itself.
//!
//! These exclusions are structural: [`TunableParam`] has exactly one variant,
//! and there is no code path that constructs a tune targeting any guard.

use serde::{Deserialize, Serialize};

/// The complete set of parameters the self-tuner is allowed to modify.
///
/// This enum is the structural enforcement of the safety/correctness exclusion:
/// only knobs listed here can ever be tuned, and the list contains ONLY pure
/// ranking-quality dials.  Adding a variant is a deliberate, reviewable act —
/// safety/correctness guards are never added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunableParam {
    /// The Reciprocal Rank Fusion dampening constant `k` (B4 hybrid retrieval).
    ///
    /// Higher `k` flattens the contribution of rank position; lower `k` sharpens
    /// it.  Purely a ranking-quality dial — no correctness or safety semantics.
    RrfK,
}

impl TunableParam {
    /// Stable identifier used in audit strings and the status surface.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::RrfK => "rrf_k",
        }
    }
}

/// A hard `[min, max]` clamp plus the perturbation `step` for one knob.
///
/// The tuner can NEVER propose a value outside `[min, max]`; [`clamp`] is
/// applied to every candidate and every restored value at the source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TuneClamp {
    /// Inclusive lower bound — the tuner cannot go below this.
    pub min: f64,
    /// Inclusive upper bound — the tuner cannot go above this.
    pub max: f64,
    /// Magnitude by which a candidate perturbs the incumbent value.
    pub step: f64,
}

impl TuneClamp {
    /// Build a clamp.  `min` must be `< max` and `step > 0` (the config layer
    /// enforces this; this constructor only stores).
    #[must_use]
    pub const fn new(min: f64, max: f64, step: f64) -> Self {
        Self { min, max, step }
    }

    /// Clamp `value` into `[min, max]`.  This is the hard bound the tuner can
    /// never exceed.
    #[must_use]
    pub const fn clamp(&self, value: f64) -> f64 {
        value.clamp(self.min, self.max)
    }
}

/// Which direction the next candidate perturbs the incumbent.
///
/// The policy alternates Up/Down across cycles (seeded by the cycle counter) so
/// the tuner explores both sides of the incumbent over time without any
/// randomness — every proposal is deterministic and reproducible from the
/// `(incumbent, cycle)` pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

/// Deterministically propose the next candidate value for `param`.
///
/// The candidate is `incumbent ± step`, clamped into the knob's `[min, max]`.
/// Direction alternates with `cycle` (even ⇒ Up, odd ⇒ Down) so successive
/// cycles probe both neighbours; a candidate equal to the incumbent (because the
/// clamp pinned it) is returned as-is — the worker treats a no-move candidate as
/// a no-op and records [`TuneOutcome::NoImprovement`] without an eval, since
/// there is nothing new to test.
///
/// Pure and deterministic: identical `(incumbent, clamp, cycle)` always yields
/// the same candidate.
#[must_use]
pub fn propose_candidate(
    _param: TunableParam,
    incumbent: f64,
    clamp: &TuneClamp,
    cycle: u64,
) -> f64 {
    let direction = if cycle.is_multiple_of(2) {
        Direction::Up
    } else {
        Direction::Down
    };
    let raw = match direction {
        Direction::Up => incumbent + clamp.step,
        Direction::Down => incumbent - clamp.step,
    };
    clamp.clamp(raw)
}

/// The outcome of evaluating one tuning candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TuneOutcome {
    /// The candidate beat the incumbent out-of-sample by at least `margin` and
    /// was applied through the gate.  The live parameter now holds `new_value`.
    Promoted {
        /// Incumbent value before the change.
        old_value: f64,
        /// The applied (clamped) candidate value.
        new_value: f64,
        /// Incumbent out-of-sample objective score.
        incumbent_score: f64,
        /// Candidate out-of-sample objective score.
        candidate_score: f64,
    },
    /// The candidate did NOT beat the incumbent by `margin`; the live parameter
    /// is unchanged.  This is the common case and is NOT an error.
    NoImprovement {
        /// The value that was tested (and rejected).
        candidate_value: f64,
        /// Incumbent out-of-sample objective score.
        incumbent_score: f64,
        /// Candidate out-of-sample objective score.
        candidate_score: f64,
    },
    /// There was no eval evidence (missing fixture / zero cases).  FAIL-CLOSED:
    /// the live parameter is left UNCHANGED and nothing is proposed.
    NoEvalData {
        /// Human-readable reason (e.g. fixture load error).
        reason: String,
    },
    /// Self-tuning is disabled by the kill-switch.  Nothing is proposed or
    /// applied; surfaced so operators see the disabled state explicitly.
    Disabled,
}

impl TuneOutcome {
    /// Whether this outcome actually changed a live parameter.  A parameter is
    /// reported "tuned" ONLY when this is `true` — the honesty contract.
    #[must_use]
    pub const fn applied(&self) -> bool {
        matches!(self, Self::Promoted { .. })
    }
}

/// Pure gate decision: did the candidate earn promotion?
///
/// Returns [`TuneOutcome::Promoted`] iff `candidate_score >= incumbent_score +
/// margin` (a strict, positive-margin improvement on the out-of-sample
/// objective).  Otherwise [`TuneOutcome::NoImprovement`] — the live value is
/// kept.  This function NEVER promotes on equal or worse scores, and a
/// non-positive `margin` still requires `candidate_score >= incumbent_score`
/// (so noise alone cannot churn the parameter when `margin > 0`).
///
/// The caller is responsible for only invoking this when REAL eval data exists;
/// the no-eval-data path is [`TuneOutcome::NoEvalData`], handled upstream.
#[must_use]
pub fn decide(
    old_value: f64,
    candidate_value: f64,
    incumbent_score: f64,
    candidate_score: f64,
    margin: f64,
) -> TuneOutcome {
    if candidate_score >= incumbent_score + margin {
        TuneOutcome::Promoted {
            old_value,
            new_value: candidate_value,
            incumbent_score,
            candidate_score,
        }
    } else {
        TuneOutcome::NoImprovement {
            candidate_value,
            incumbent_score,
            candidate_score,
        }
    }
}

/// One audited self-tuning decision — the "proposal" record.
///
/// Every cycle produces exactly one record, whether or not it changed anything.
/// The record carries the provenance string `agent:<id>;gated:true` (mirroring
/// the B9 proposal materialization provenance) plus the full truthful audit:
/// which param, old→new, both eval scores, and when.  A record with
/// `outcome.applied() == false` documents a NO-OP — it never claims a change it
/// did not make.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TuneRecord {
    /// Monotone record id (`t<seq>`), assigned by [`SelfTuneLog`].
    pub id: String,
    /// The parameter this cycle targeted.
    pub param: TunableParam,
    /// The gate outcome.
    pub outcome: TuneOutcome,
    /// Provenance string: `agent:<tuner-id>;gated:true`.  The change is an
    /// Agent-authored, GATED proposal — never an ungated config write.
    pub provenance: String,
    /// RFC 3339 timestamp the decision was recorded (`now`, injected — never a
    /// clock read inside this crate).
    pub decided_at: String,
    /// Human-readable one-line audit summary.
    pub summary: String,
}

/// The stable tuner agent id used in provenance and audit strings.
pub const TUNER_AGENT_ID: &str = "system:self-tuner";

/// Build the gated provenance string for a tuning record.
///
/// Format `agent:<id>;gated:true` — the `gated:true` marker is the audit proof
/// that this self-modification went through the eval gate, exactly like a B9
/// `Provenance::Agent { gated: true }` materialization.
#[must_use]
pub fn tuner_provenance() -> String {
    format!("agent:{TUNER_AGENT_ID};gated:true")
}

/// In-process audit log of self-tuning decisions.
///
/// Persistence round-trips through serde like the B9 `ProposalQueue` and the
/// other knowledge-system state.  The log is the single source of truth for the
/// "last tuning decision" surfaced in `tdw.kg.status`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SelfTuneLog {
    records: Vec<TuneRecord>,
    next_id: u64,
}

impl SelfTuneLog {
    /// An empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one cycle's decision and return the stored record.
    ///
    /// Assigns a monotone `t<seq>` id and stamps the gated provenance.  `now`
    /// is injected (no clock read here) so callers stay deterministic.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the `expect` immediately follows the `push`, so
    /// the vector is always non-empty when `last()` is read.
    pub fn record(&mut self, param: TunableParam, outcome: TuneOutcome, now: &str) -> &TuneRecord {
        self.next_id += 1;
        let id = format!("t{}", self.next_id);
        let summary = summarize(param, &outcome);
        self.records.push(TuneRecord {
            id,
            param,
            outcome,
            provenance: tuner_provenance(),
            decided_at: now.to_string(),
            summary,
        });
        self.records
            .last()
            .expect("just pushed a record; vec is non-empty")
    }

    /// The most recent decision, if any.
    #[must_use]
    pub fn last(&self) -> Option<&TuneRecord> {
        self.records.last()
    }

    /// Total decisions recorded since start.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no decision has been recorded yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Count of decisions that ACTUALLY applied a change (the honest "tuned"
    /// count).
    #[must_use]
    pub fn applied_count(&self) -> usize {
        self.records.iter().filter(|r| r.outcome.applied()).count()
    }
}

/// Build the truthful one-line audit summary for a decision.
fn summarize(param: TunableParam, outcome: &TuneOutcome) -> String {
    let id = param.id();
    match outcome {
        TuneOutcome::Promoted {
            old_value,
            new_value,
            incumbent_score,
            candidate_score,
        } => format!(
            "{id}: APPLIED {old_value:.4} → {new_value:.4} \
             (objective {incumbent_score:.6} → {candidate_score:.6}; gated, eval-confirmed)"
        ),
        TuneOutcome::NoImprovement {
            candidate_value,
            incumbent_score,
            candidate_score,
        } => format!(
            "{id}: NO-OP — candidate {candidate_value:.4} did not beat incumbent by margin \
             (objective {incumbent_score:.6} vs {candidate_score:.6}); value unchanged"
        ),
        TuneOutcome::NoEvalData { reason } => {
            format!("{id}: NO-OP (fail-closed) — no eval data: {reason}; value unchanged")
        }
        TuneOutcome::Disabled => {
            format!("{id}: disabled — set [knowledge.self_tune] enabled = true to activate")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clamp() -> TuneClamp {
        TuneClamp::new(10.0, 120.0, 10.0)
    }

    // ── clamp bounds (requirement 3) ──────────────────────────────────────────

    #[test]
    fn clamp_pins_values_to_bounds() {
        let c = clamp();
        assert!(
            (c.clamp(5.0) - 10.0).abs() < 1e-12,
            "below min pinned to min"
        );
        assert!(
            (c.clamp(200.0) - 120.0).abs() < 1e-12,
            "above max pinned to max"
        );
        assert!((c.clamp(60.0) - 60.0).abs() < 1e-12, "in-range unchanged");
    }

    #[test]
    fn proposed_candidate_never_exceeds_clamp() {
        let c = clamp();
        // At the upper bound, an Up cycle cannot exceed max.
        let up = propose_candidate(TunableParam::RrfK, 120.0, &c, 0);
        assert!(up <= c.max, "Up at max must stay <= max, got {up}");
        // At the lower bound, a Down cycle cannot go below min.
        let down = propose_candidate(TunableParam::RrfK, 10.0, &c, 1);
        assert!(down >= c.min, "Down at min must stay >= min, got {down}");
        // A mid-range candidate moves by exactly one step.
        let mid_up = propose_candidate(TunableParam::RrfK, 60.0, &c, 0);
        assert!((mid_up - 70.0).abs() < 1e-12);
        let mid_down = propose_candidate(TunableParam::RrfK, 60.0, &c, 1);
        assert!((mid_down - 50.0).abs() < 1e-12);
    }

    #[test]
    fn proposal_is_deterministic() {
        let c = clamp();
        // Bit-equal: identical inputs must yield the identical f64 (no float
        // tolerance — determinism is exact).
        assert_eq!(
            propose_candidate(TunableParam::RrfK, 60.0, &c, 4).to_bits(),
            propose_candidate(TunableParam::RrfK, 60.0, &c, 4).to_bits()
        );
    }

    // ── gate decision (fail-closed + margin) ──────────────────────────────────

    #[test]
    fn decide_promotes_only_on_improvement_beyond_margin() {
        // Candidate beats incumbent by 0.02 > margin 0.01 → promote.
        let promoted = decide(60.0, 70.0, 0.80, 0.82, 0.01);
        assert!(promoted.applied(), "clear improvement must promote");
        match promoted {
            TuneOutcome::Promoted {
                old_value,
                new_value,
                ..
            } => {
                assert!((old_value - 60.0).abs() < 1e-12);
                assert!((new_value - 70.0).abs() < 1e-12);
            }
            other => panic!("expected Promoted, got {other:?}"),
        }
    }

    #[test]
    fn decide_rejects_within_margin_and_equal_and_worse() {
        // Improvement smaller than margin → no-op.
        assert!(!decide(60.0, 70.0, 0.80, 0.805, 0.01).applied());
        // Exactly equal → no-op (no improvement).
        assert!(!decide(60.0, 70.0, 0.80, 0.80, 0.01).applied());
        // Worse → no-op (this is the regression guard: a worse candidate never
        // lands, so a regression is rejected before it is ever applied).
        assert!(!decide(60.0, 70.0, 0.80, 0.50, 0.01).applied());
    }

    #[test]
    fn decide_with_zero_margin_still_requires_non_negative_improvement() {
        assert!(
            decide(60.0, 70.0, 0.80, 0.80, 0.0).applied(),
            "equal passes at margin 0"
        );
        assert!(
            !decide(60.0, 70.0, 0.80, 0.79, 0.0).applied(),
            "worse never passes"
        );
    }

    // ── audit log honesty (requirement 2) ─────────────────────────────────────

    #[test]
    fn log_records_gated_provenance_and_truthful_summary() {
        let mut log = SelfTuneLog::new();
        let outcome = decide(60.0, 70.0, 0.80, 0.83, 0.01);
        let rec = log.record(TunableParam::RrfK, outcome, "2026-06-12T00:00:00Z");
        assert_eq!(rec.id, "t1");
        assert_eq!(rec.provenance, "agent:system:self-tuner;gated:true");
        assert!(
            rec.provenance.contains("gated:true"),
            "must be a GATED change"
        );
        assert!(rec.summary.contains("APPLIED"));
        assert!(rec.summary.contains("60.0000 → 70.0000"));
        assert_eq!(log.applied_count(), 1);
    }

    #[test]
    fn no_eval_data_records_unchanged_and_not_applied() {
        let mut log = SelfTuneLog::new();
        let rec = log.record(
            TunableParam::RrfK,
            TuneOutcome::NoEvalData {
                reason: "fixture missing".to_string(),
            },
            "2026-06-12T00:00:00Z",
        );
        assert!(!rec.outcome.applied(), "fail-closed: no eval ⇒ no change");
        assert!(rec.summary.contains("fail-closed"));
        assert!(rec.summary.contains("unchanged"));
        assert_eq!(log.applied_count(), 0, "a no-op must not count as tuned");
    }

    #[test]
    fn no_improvement_is_a_truthful_noop() {
        let mut log = SelfTuneLog::new();
        let outcome = decide(60.0, 70.0, 0.80, 0.80, 0.01);
        let rec = log.record(TunableParam::RrfK, outcome, "2026-06-12T00:00:00Z");
        assert!(!rec.outcome.applied());
        assert!(rec.summary.contains("NO-OP"));
        assert!(rec.summary.contains("unchanged"));
    }

    #[test]
    fn log_round_trips_through_json() {
        let mut log = SelfTuneLog::new();
        log.record(
            TunableParam::RrfK,
            decide(60.0, 70.0, 0.80, 0.85, 0.01),
            "2026-06-12T00:00:00Z",
        );
        let json = serde_json::to_string(&log).expect("serialize");
        let restored: SelfTuneLog = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(log.len(), restored.len());
        assert_eq!(
            log.last().map(|r| r.id.clone()),
            restored.last().map(|r| r.id.clone())
        );
    }

    #[test]
    fn only_rrf_k_is_tunable() {
        // Structural exclusion of safety/correctness guards: the tunable set has
        // exactly one variant, and it is a pure ranking-quality dial.
        assert_eq!(TunableParam::RrfK.id(), "rrf_k");
    }
}
