//! Walk-forward knowledge validation — replay harness + K-R5 promote gate
//! (knowledge-system K-R7).
//!
//! # What this is
//!
//! Backtesting for knowledge itself, uniquely enabled by the bi-plane temporal
//! graph.  Given a [`InferRule`] and one or more historical [`ReplaySplit`]
//! points, the harness evaluates whether the rule — applied using **only** data
//! that was knowable at `split.as_of` — would have correctly predicted facts
//! that actually became true **after** `as_of` within a forward window.
//!
//! ## No-lookahead guarantee (the integrity core)
//!
//! The rule-application side (the "past" side) uses [`active_at`] from
//! `tdw-core` to filter every edge and node to those that were valid at
//! `as_of`.  This is the **same predicate** the production graph temporal filter
//! uses — it is reused, not reimplemented, so the guarantee is structural:
//!
//! - Input facts: edge `e` is eligible iff
//!   `active_at(e.valid_from, e.valid_to, as_of)` — i.e., `e.valid_from <= as_of
//!   < e.valid_to` (open bounds are open).
//! - Outcome facts: a fact realized in the forward window `(as_of, as_of+window]`
//!   satisfies `e.valid_from > as_of && e.valid_from <= as_of_plus_window` — the
//!   rule cannot see it during prediction because it was not `active_at(as_of)`.
//!
//! The leakage regression test in this module proves this invariant:
//! a rule that "predicts" using a fact with `valid_from` strictly after `as_of`
//! is impossible — the input filter excludes it.
//!
//! ## Metrics
//!
//! For each split the harness collects predictions (derived edges the rule would
//! make from the `as_of` graph) and outcomes (edges that became true in the
//! forward window):
//!
//! - **Precision** = `|predictions ∩ outcomes| / |predictions|`
//! - **Recall** = `|predictions ∩ outcomes| / |outcomes|`
//! - **Support** = `|outcomes|` (size of the realized ground-truth)
//!
//! When `predictions` is empty, precision is defined as `1.0` (no false
//! positives — vacuously correct).  When `outcomes` is empty, recall is `1.0`
//! (nothing to miss).
//!
//! ## K-R5 promote gate
//!
//! [`promote_gate`] is the seam K-R5 consumes: given a rule, a set of
//! historical splits, and a threshold, it returns a [`PromoteVerdict`] that
//! declares whether the rule's out-of-sample replay precision/recall clears the
//! bar.  K-R5 calls this before inducting a rule into the live rule set.
//!
//! ## Standing self-test (default OFF)
//!
//! [`ReplayEvalConfig`] gates an optional in-daemon scheduled replay over the
//! checked-in splits for the currently-active rules.  Default: `enabled = false`,
//! with a loud status note when disabled (the K-L3/K-R4 honesty posture).
//! Results land in `tdw.kg.status` as a replay-health line.  From-config
//! registration is gated behind the same `enabled` flag — nothing wires itself
//! up silently.
//!
//! # Production-wiring discipline
//!
//! [`promote_gate`] is a real callable function tested standalone.  The
//! standing self-test is config-gated; when disabled it loudly says so.
//! No dead wiring — every claim is traced through the real call path.

// Precision and recall indices are small non-negative integers; cast is safe.
#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEdge, active_at};
use tdw_infer::InferRule;

use crate::retrieval_eval::DriftKey;

// ── Temporal helpers ─────────────────────────────────────────────────────────

/// Whether an edge's `valid_from` falls strictly inside the forward window
/// `(as_of, as_of_plus_window]`.
///
/// An outcome edge must have become true **after** `as_of` (strictly, so the
/// rule could not have seen it during prediction) and **within** the window
/// (inclusive upper bound, matching the half-open document payload convention).
///
/// Both `as_of` and `as_of_plus_window` are normalized RFC 3339 UTC strings
/// (`YYYY-MM-DDThh:mm:ssZ`), compared lexicographically — the same convention
/// `active_at` and the graph validity model use.
#[must_use]
pub fn in_forward_window(edge: &GraphEdge, as_of: &str, as_of_plus_window: &str) -> bool {
    edge.valid_from
        .as_ref()
        .is_some_and(|from| from.as_str() > as_of && from.as_str() <= as_of_plus_window)
}

// ── Historical split point ────────────────────────────────────────────────────

/// One checked-in historical split for walk-forward replay.
///
/// The split is identified by a stable `split_id` (shared across compared runs
/// so metric deltas are split-stable).  `as_of` is the temporal cut: the rule
/// sees only facts active at this point; outcomes are facts that became true in
/// `(as_of, as_of_plus_window]`.
///
/// Splits are **checked into the repository** and never generated at runtime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaySplit {
    /// Stable split identifier.
    pub split_id: String,
    /// Temporal cut (RFC 3339 UTC, e.g. `"2024-01-01T00:00:00Z"`).
    /// The rule-application side sees only edges `active_at(as_of)`.
    pub as_of: String,
    /// Inclusive upper bound of the forward window
    /// (`"2024-04-01T00:00:00Z"` for a 90-day window after a 2024-01-01 cut).
    ///
    /// An outcome edge must have `valid_from` in `(as_of, as_of_plus_window]`.
    pub as_of_plus_window: String,
}

// ── Per-split scores ──────────────────────────────────────────────────────────

/// Replay metrics for one rule over one historical split.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitReplayScore {
    /// The split this score belongs to.
    pub split_id: String,
    /// Fraction of predictions that were confirmed as realized outcomes.
    ///
    /// `|predictions ∩ outcomes| / |predictions|`.
    /// `1.0` when `predictions` is empty (vacuously correct — no false positives).
    pub precision: f64,
    /// Fraction of realized outcomes the rule predicted.
    ///
    /// `|predictions ∩ outcomes| / |outcomes|`.
    /// `1.0` when `outcomes` is empty (nothing to miss).
    pub recall: f64,
    /// Count of realized outcome facts in the forward window (ground-truth size).
    pub support: usize,
    /// Count of predictions the rule generated from the `as_of` graph.
    pub predictions: usize,
}

// ── Per-rule report over all splits ──────────────────────────────────────────

/// Walk-forward replay report for one rule over a set of historical splits.
///
/// This type is `Serialize + Deserialize` so successive runs can be persisted
/// and diff-ed by the caller.  The runner itself has no storage — persistence
/// is the caller's responsibility (no hidden global state).
///
/// # Drift key
///
/// The report is stamped with a [`DriftKey`] (the same triple used by
/// [`crate::retrieval_eval::RetrievalEvalReport`]) so metric changes between
/// reports can be attributed to specific component version bumps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleReplayReport {
    /// The rule that was replayed.
    pub rule_id: String,
    /// Mean precision over all splits (macro-average).
    pub mean_precision: f64,
    /// Mean recall over all splits (macro-average).
    pub mean_recall: f64,
    /// Total support across all splits (sum of per-split `support`).
    pub total_support: usize,
    /// Per-split breakdown in input order.
    pub splits: Vec<SplitReplayScore>,
    /// The version triple this report is attributed to.
    pub drift_key: DriftKey,
}

// ── K-R5 promote gate ─────────────────────────────────────────────────────────

/// Thresholds the K-R5 promote gate applies to a rule's replay metrics.
///
/// A rule promotes only if its out-of-sample replay precision **and** recall
/// both clear their respective floors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromoteThreshold {
    /// Minimum mean precision across historical splits (0.0–1.0).
    pub min_precision: f64,
    /// Minimum mean recall across historical splits (0.0–1.0).
    pub min_recall: f64,
}

impl Default for PromoteThreshold {
    /// Conservative production defaults: 60 % precision, 20 % recall.
    ///
    /// These defaults are deliberately asymmetric — for knowledge induction,
    /// avoiding false positives (precision) matters more than coverage (recall).
    fn default() -> Self {
        Self {
            min_precision: 0.60,
            min_recall: 0.20,
        }
    }
}

/// Outcome of the [`promote_gate`] evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromoteVerdict {
    /// The rule clears the threshold — K-R5 may induct it.
    Promote {
        /// Human-readable summary.
        summary: String,
    },
    /// The rule does not clear the threshold — K-R5 must not induct it.
    Reject {
        /// Human-readable summary including which metric(s) failed.
        summary: String,
    },
}

impl PromoteVerdict {
    /// Whether the verdict is a promotion (the rule cleared the gate).
    #[must_use]
    pub const fn is_promote(&self) -> bool {
        matches!(self, Self::Promote { .. })
    }

    /// The human-readable summary regardless of variant.
    #[must_use]
    pub fn summary(&self) -> &str {
        match self {
            Self::Promote { summary } | Self::Reject { summary } => summary,
        }
    }
}

// ── Pure metric math ──────────────────────────────────────────────────────────

/// Precision: `|hits| / |predictions|`.  Vacuously `1.0` when `predictions = 0`.
#[must_use]
pub fn precision(hits: usize, predictions: usize) -> f64 {
    if predictions == 0 {
        1.0
    } else {
        hits as f64 / predictions as f64
    }
}

/// Recall: `|hits| / |outcomes|`.  Vacuously `1.0` when `outcomes = 0`.
#[must_use]
pub fn recall(hits: usize, outcomes: usize) -> f64 {
    if outcomes == 0 {
        1.0
    } else {
        hits as f64 / outcomes as f64
    }
}

// ── Rule application against a temporal slice ─────────────────────────────────

/// An edge triple used as a prediction or outcome key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EdgeTriple {
    /// Source node.
    pub from: String,
    /// Relationship type.
    pub rel: String,
    /// Target node.
    pub to: String,
}

impl EdgeTriple {
    fn from_edge(edge: &GraphEdge) -> Self {
        Self {
            from: edge.from.clone(),
            rel: edge.rel.clone(),
            to: edge.to.clone(),
        }
    }
}

/// Apply a [`InferRule::DeriveEdge`] rule to the edges in `all_edges` using
/// only those active at `as_of`, and return the set of derived edge triples.
///
/// This is the "past" side of the replay: the rule sees only what was knowable
/// at `as_of`.  The no-lookahead guarantee is structural — every edge is passed
/// through `active_at(e.valid_from, e.valid_to, as_of)` before it participates
/// in the chain join.  Non-`DeriveEdge` rules produce an empty set (this
/// harness only replays edge derivation rules).
///
/// Self-loops (`from == to`) are excluded, matching the production engine.
#[must_use]
pub fn apply_derive_edge_at(
    rule: &InferRule,
    all_edges: &[GraphEdge],
    as_of: &str,
) -> BTreeSet<EdgeTriple> {
    let InferRule::DeriveEdge {
        when, derived_type, ..
    } = rule
    else {
        return BTreeSet::new();
    };

    // ── No-lookahead filter: only edges active at as_of ───────────────────────
    // REUSE active_at from tdw-core — do NOT reimplement.
    let active: Vec<&GraphEdge> = all_edges
        .iter()
        .filter(|edge| active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), as_of))
        .collect();

    // ── Chain join (same algorithm as InferEngine::match_chain) ──────────────
    // Length-1 chain: single hop.  Longer chains: join head-to-tail.
    // Partials: (head_from, current_tail).
    if when.is_empty() {
        return BTreeSet::new();
    }

    let first_rel = &when[0].rel;
    let mut partials: Vec<(String, String)> = active
        .iter()
        .filter(|edge| &edge.rel == first_rel)
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect();

    for pattern in &when[1..] {
        // Build a lookup: rel-from → Vec<to> for this hop.
        let mut by_from: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for edge in active.iter().filter(|edge| edge.rel == pattern.rel) {
            by_from
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }
        let mut next = Vec::new();
        for (head, tail) in partials {
            if let Some(nexts) = by_from.get(&tail) {
                for new_tail in nexts {
                    next.push((head.clone(), new_tail.clone()));
                }
            }
        }
        partials = next;
    }

    partials
        .into_iter()
        .filter(|(from, to)| from != to) // exclude self-loops
        .map(|(from, to)| EdgeTriple {
            from,
            rel: derived_type.clone(),
            to,
        })
        .collect()
}

/// Collect outcome facts: edges of the `derived_type` produced by the rule
/// whose `valid_from` falls strictly in `(as_of, as_of_plus_window]`.
///
/// This is the "future" side of the replay — facts the rule could not have seen
/// during prediction because they were not active at `as_of`.
#[must_use]
pub fn collect_outcomes(
    rule: &InferRule,
    all_edges: &[GraphEdge],
    as_of: &str,
    as_of_plus_window: &str,
) -> BTreeSet<EdgeTriple> {
    let Some(derived_type) = rule.produced_type() else {
        return BTreeSet::new();
    };
    all_edges
        .iter()
        .filter(|edge| edge.rel == derived_type)
        .filter(|edge| in_forward_window(edge, as_of, as_of_plus_window))
        .map(EdgeTriple::from_edge)
        .collect()
}

// ── Per-split scorer ──────────────────────────────────────────────────────────

/// Score one rule against one historical split.
///
/// `all_edges` is the full temporal edge corpus (edges from ALL time — the split
/// filter is applied inside this function).  The function is pure and
/// deterministic: given the same inputs it always returns the same
/// [`SplitReplayScore`].
#[must_use]
pub fn score_split(
    rule: &InferRule,
    all_edges: &[GraphEdge],
    split: &ReplaySplit,
) -> SplitReplayScore {
    let preds = apply_derive_edge_at(rule, all_edges, &split.as_of);
    let outcomes = collect_outcomes(rule, all_edges, &split.as_of, &split.as_of_plus_window);

    let hits = preds.intersection(&outcomes).count();
    let p = precision(hits, preds.len());
    let r = recall(hits, outcomes.len());

    SplitReplayScore {
        split_id: split.split_id.clone(),
        precision: p,
        recall: r,
        support: outcomes.len(),
        predictions: preds.len(),
    }
}

// ── Full replay eval ──────────────────────────────────────────────────────────

/// Run walk-forward replay for `rule` over all `splits` and return the aggregate
/// [`RuleReplayReport`].
///
/// The function is pure and deterministic — all randomness and I/O live outside.
/// Callers inject the full temporal edge corpus; split-level temporal filtering
/// is applied inside.
///
/// # Panics
///
/// Does not panic; `splits` may be empty (producing `mean_precision = 1.0`,
/// `mean_recall = 1.0`, `total_support = 0`).
#[must_use]
pub fn run_replay_eval(
    rule: &InferRule,
    all_edges: &[GraphEdge],
    splits: &[ReplaySplit],
    drift_key: DriftKey,
) -> RuleReplayReport {
    let scored: Vec<SplitReplayScore> = splits
        .iter()
        .map(|split| score_split(rule, all_edges, split))
        .collect();

    // Vacuous case: no splits → nothing to fail on → both means are 1.0.
    let (mean_precision, mean_recall) = if scored.is_empty() {
        (1.0, 1.0)
    } else {
        let n = scored.len() as f64;
        (
            scored.iter().map(|s| s.precision).sum::<f64>() / n,
            scored.iter().map(|s| s.recall).sum::<f64>() / n,
        )
    };
    let total_support = scored.iter().map(|s| s.support).sum();

    RuleReplayReport {
        rule_id: rule.rule_id().to_string(),
        mean_precision,
        mean_recall,
        total_support,
        splits: scored,
        drift_key,
    }
}

// ── K-R5 promote gate (the contract K-R5 consumes) ───────────────────────────

/// Evaluate whether `rule` clears the promotion bar over `splits`.
///
/// # K-R5 contract
///
/// K-R5 calls this function before inducting an induced rule into the live rule
/// set.  The function:
///
/// 1. Runs walk-forward replay over the checked-in `splits` with the full
///    temporal edge corpus `all_edges`.
/// 2. Computes mean precision and mean recall across splits.
/// 3. Returns [`PromoteVerdict::Promote`] iff both metrics clear their
///    respective floors in `threshold`.
///
/// This is a **real callable function** tested standalone — not dead wiring.
/// The function is pure, synchronous, and deterministic.
///
/// # Example (the seam K-R5 calls)
///
/// ```rust
/// use tdw_eval_runner::replay_eval::{
///     ReplaySplit, PromoteThreshold, PromoteVerdict, promote_gate,
/// };
/// use tdw_infer::{InferRule, EdgePattern};
/// use tdw_eval_runner::retrieval_eval::DriftKey;
///
/// let rule = InferRule::DeriveEdge {
///     rule_id: "supplier-exposure".to_string(),
///     stratum: 0,
///     when: vec![
///         EdgePattern { rel: "supplier_of".to_string() },
///         EdgePattern { rel: "listed_on".to_string() },
///     ],
///     derived_type: "exposed_to".to_string(),
/// };
/// let splits: Vec<ReplaySplit> = vec![]; // empty → vacuous promote
/// let threshold = PromoteThreshold::default();
/// let drift_key = DriftKey::default();
/// let verdict = promote_gate(&rule, &[], &splits, &threshold, drift_key);
/// assert!(verdict.is_promote());
/// ```
#[must_use]
pub fn promote_gate(
    rule: &InferRule,
    all_edges: &[GraphEdge],
    splits: &[ReplaySplit],
    threshold: &PromoteThreshold,
    drift_key: DriftKey,
) -> PromoteVerdict {
    let report = run_replay_eval(rule, all_edges, splits, drift_key);

    let precision_ok = report.mean_precision >= threshold.min_precision;
    let recall_ok = report.mean_recall >= threshold.min_recall;

    if precision_ok && recall_ok {
        PromoteVerdict::Promote {
            summary: format!(
                "rule {:?} clears promote gate: precision {:.4} >= {:.4}, \
                 recall {:.4} >= {:.4} (support={}, splits={})",
                report.rule_id,
                report.mean_precision,
                threshold.min_precision,
                report.mean_recall,
                threshold.min_recall,
                report.total_support,
                report.splits.len(),
            ),
        }
    } else {
        let mut failed = Vec::new();
        if !precision_ok {
            failed.push(format!(
                "precision {:.4} < {:.4}",
                report.mean_precision, threshold.min_precision
            ));
        }
        if !recall_ok {
            failed.push(format!(
                "recall {:.4} < {:.4}",
                report.mean_recall, threshold.min_recall
            ));
        }
        PromoteVerdict::Reject {
            summary: format!(
                "rule {:?} rejected by promote gate: {} (support={}, splits={})",
                report.rule_id,
                failed.join("; "),
                report.total_support,
                report.splits.len(),
            ),
        }
    }
}

// ── Standing self-test config (default OFF) ───────────────────────────────────

/// Configuration for the optional in-daemon standing replay self-test.
///
/// Default: `enabled = false`.
///
/// When disabled (the default), the daemon **does not** schedule a replay eval
/// and emits the following status line to `tdw.kg.status`:
///
/// ```text
/// replay-health: DISABLED — set [knowledge.replay_eval] enabled = true to activate
/// ```
///
/// This is the K-L3/K-R4 honesty posture: a disabled self-test says so loudly
/// rather than being silent.  When enabled, the daemon schedules a cron-triggered
/// replay over the checked-in splits for each currently-active rule and records
/// results in `tdw.kg.status` as a `replay-health` line.
///
/// # Cron registration
///
/// This module is free of `tdw-cron` dependencies (mirrors the
/// `ScheduledEvalConfig` precedent).  The daemon reads `cadence` to build a
/// `tdw-cron` `ScheduledTrigger`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReplayEvalConfig {
    /// Whether the standing replay self-test is active.
    ///
    /// Default: `false` — the daemon registers no cron trigger and emits a loud
    /// disabled-status note.
    #[serde(default)]
    pub enabled: bool,

    /// 5-field cron expression for the replay cadence.
    ///
    /// Default: `"0 4 * * MON"` (weekly, Monday 04:00 UTC — offset from the
    /// retrieval eval at 03:00 to avoid resource contention).
    #[serde(default = "default_replay_cadence")]
    pub cadence: String,

    /// Minimum precision required for the rule to pass the standing replay check.
    ///
    /// Default: `0.60`.
    #[serde(default = "default_min_precision")]
    pub min_precision: f64,

    /// Minimum recall required for the rule to pass the standing replay check.
    ///
    /// Default: `0.20`.
    #[serde(default = "default_min_recall")]
    pub min_recall: f64,
}

fn default_replay_cadence() -> String {
    "0 4 * * MON".to_string()
}

const fn default_min_precision() -> f64 {
    0.60
}

const fn default_min_recall() -> f64 {
    0.20
}

impl Default for ReplayEvalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence: default_replay_cadence(),
            min_precision: default_min_precision(),
            min_recall: default_min_recall(),
        }
    }
}

/// The stable cron trigger id for the knowledge replay self-test.
///
/// Format: `knowledge-replay-eval:<rule_id>`.
#[must_use]
pub fn replay_trigger_id(rule_id: &str) -> String {
    format!("knowledge-replay-eval:{rule_id}")
}

/// The status line emitted to `tdw.kg.status` when the replay self-test is disabled.
///
/// The daemon writes this to the shared status cell so operators can see that
/// replay health is not being tracked.
#[must_use]
pub const fn replay_disabled_status() -> &'static str {
    "replay-health: DISABLED — set [knowledge.replay_eval] enabled = true to activate"
}

/// Error returned by [`ReplayEvalConfig::validate`].
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ReplayEvalConfigError {
    /// The cron expression cannot be parsed.
    #[error("invalid cadence expression {expr:?}: {detail}")]
    InvalidCadence {
        /// The expression that failed.
        expr: String,
        /// The underlying parse error description.
        detail: String,
    },
    /// A threshold value is outside `[0.0, 1.0]`.
    #[error("threshold {field} = {value} is out of range [0.0, 1.0]")]
    ThresholdOutOfRange {
        /// Name of the threshold field.
        field: &'static str,
        /// The invalid value.
        value: f64,
    },
}

impl ReplayEvalConfig {
    /// Validate this config.
    ///
    /// Parses the 5-field `cadence` expression after expanding it to a 7-field
    /// form (`"0 <expr> *"`) — the same expansion `ScheduledEvalConfig::validate`
    /// applies.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayEvalConfigError`] if the cadence is invalid or a
    /// threshold is outside `[0.0, 1.0]`.
    pub fn validate(&self) -> Result<(), ReplayEvalConfigError> {
        let seven_field = format!("0 {} *", self.cadence);
        seven_field.parse::<cron::Schedule>().map_err(|err| {
            ReplayEvalConfigError::InvalidCadence {
                expr: self.cadence.clone(),
                detail: err.to_string(),
            }
        })?;
        for (field, value) in [
            ("min_precision", self.min_precision),
            ("min_recall", self.min_recall),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(ReplayEvalConfigError::ThresholdOutOfRange { field, value });
            }
        }
        Ok(())
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_core::{GraphEdge, Provenance};
    use tdw_infer::{EdgePattern, InferRule};

    // ── test helpers ─────────────────────────────────────────────────────────

    fn ingest_prov() -> Provenance {
        Provenance::Ingest {
            source: "test".to_string(),
        }
    }

    /// Build an edge with an explicit `valid_from`.
    fn dated_edge(from: &str, rel: &str, to: &str, valid_from: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: ingest_prov(),
            valid_from: Some(valid_from.to_string()),
            valid_to: None,
        }
    }

    /// Build an undated edge (no `valid_from`).
    fn undated_edge(from: &str, rel: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: ingest_prov(),
            valid_from: None,
            valid_to: None,
        }
    }

    fn derive_rule(rule_id: &str, rels: &[&str], derived: &str) -> InferRule {
        InferRule::DeriveEdge {
            rule_id: rule_id.to_string(),
            stratum: 0,
            when: rels
                .iter()
                .map(|rel| EdgePattern {
                    rel: (*rel).to_string(),
                })
                .collect(),
            derived_type: derived.to_string(),
        }
    }

    fn split(id: &str, as_of: &str, window_end: &str) -> ReplaySplit {
        ReplaySplit {
            split_id: id.to_string(),
            as_of: as_of.to_string(),
            as_of_plus_window: window_end.to_string(),
        }
    }

    fn default_drift_key() -> DriftKey {
        DriftKey {
            embedder_model: "stub".to_string(),
            rules_version: Some(1),
            infer_version: Some(1),
        }
    }

    // ── pure metric math (hand-computed) ─────────────────────────────────────

    #[test]
    fn precision_hand_computed() {
        // 2 hits out of 4 predictions → 0.5
        assert!((precision(2, 4) - 0.5).abs() < 1e-12);
        // 3 hits out of 3 predictions → 1.0
        assert!((precision(3, 3) - 1.0).abs() < 1e-12);
        // 0 hits out of 5 predictions → 0.0
        assert!(precision(0, 5).abs() < 1e-12);
    }

    #[test]
    fn precision_vacuous_when_no_predictions() {
        // No predictions → vacuously 1.0 (no false positives).
        assert!((precision(0, 0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn recall_hand_computed() {
        // 1 hit out of 3 outcomes → 1/3
        assert!((recall(1, 3) - 1.0 / 3.0).abs() < 1e-12);
        // 3 hits out of 3 → 1.0
        assert!((recall(3, 3) - 1.0).abs() < 1e-12);
        // 0 hits out of 2 → 0.0
        assert!(recall(0, 2).abs() < 1e-12);
    }

    #[test]
    fn recall_vacuous_when_no_outcomes() {
        // No outcomes → vacuously 1.0 (nothing to miss).
        assert!((recall(0, 0) - 1.0).abs() < 1e-12);
    }

    // ── in_forward_window ─────────────────────────────────────────────────────

    #[test]
    fn in_forward_window_strictly_after_as_of_and_within_window() {
        let as_of = "2024-01-01T00:00:00Z";
        let window_end = "2024-04-01T00:00:00Z";

        // Strictly after as_of and within window → true.
        let e = dated_edge("a", "r", "b", "2024-02-01T00:00:00Z");
        assert!(in_forward_window(&e, as_of, window_end));

        // Exactly at the window end (inclusive upper bound) → true.
        let e = dated_edge("a", "r", "b", "2024-04-01T00:00:00Z");
        assert!(in_forward_window(&e, as_of, window_end));

        // Exactly at as_of → NOT in forward window (not strictly after).
        let e = dated_edge("a", "r", "b", "2024-01-01T00:00:00Z");
        assert!(!in_forward_window(&e, as_of, window_end));

        // Before as_of → NOT in forward window.
        let e = dated_edge("a", "r", "b", "2023-12-01T00:00:00Z");
        assert!(!in_forward_window(&e, as_of, window_end));

        // After window_end → NOT in forward window.
        let e = dated_edge("a", "r", "b", "2024-07-01T00:00:00Z");
        assert!(!in_forward_window(&e, as_of, window_end));

        // No valid_from → NOT a new outcome.
        let e = undated_edge("a", "r", "b");
        assert!(!in_forward_window(&e, as_of, window_end));
    }

    // ── no-lookahead guarantee (leakage regression test) ─────────────────────

    /// Leakage regression: a rule that would "predict" using a fact with
    /// `valid_from` strictly after `as_of` must NOT see it.
    ///
    /// This test proves the no-lookahead guarantee structurally:
    /// the `active_at` filter excludes any edge whose `valid_from > as_of`,
    /// so post-split edges can never participate in chain matching.
    #[test]
    fn no_lookahead_post_split_edge_cannot_participate_in_prediction() {
        // Rule: A -supplier_of-> B -listed_on-> X  =>  A -exposed_to-> X
        let rule = derive_rule("r", &["supplier_of", "listed_on"], "exposed_to");

        // supplier_of edge is active before the split — visible at as_of.
        let pre_split = dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z");

        // listed_on edge becomes known AFTER as_of — must NOT be visible.
        let post_split = dated_edge("b", "listed_on", "x", "2024-06-01T00:00:00Z");

        let as_of = "2024-01-01T00:00:00Z";
        let all_edges = [pre_split, post_split];

        let predictions = apply_derive_edge_at(&rule, &all_edges, as_of);

        // Without the listed_on edge (post-split), the chain cannot complete —
        // no predictions must be generated.
        assert!(
            predictions.is_empty(),
            "post-split edge must not participate in prediction; got {:?}",
            predictions
        );
    }

    /// Complementary leakage check: when BOTH edges are active at as_of,
    /// the prediction IS generated (the filter does not over-suppress).
    #[test]
    fn both_pre_split_edges_produce_prediction() {
        let rule = derive_rule("r", &["supplier_of", "listed_on"], "exposed_to");

        let e1 = dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z");
        let e2 = dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z");

        let as_of = "2024-01-01T00:00:00Z";
        let predictions = apply_derive_edge_at(&rule, &[e1, e2], as_of);

        assert_eq!(predictions.len(), 1);
        assert!(predictions.contains(&EdgeTriple {
            from: "a".to_string(),
            rel: "exposed_to".to_string(),
            to: "x".to_string(),
        }));
    }

    /// Undated edges (no valid_from) are active at any time — they always pass
    /// the active_at filter and CAN participate in chain matching.
    #[test]
    fn undated_edges_are_active_at_any_as_of() {
        let rule = derive_rule("r", &["owns"], "related_to");
        let e = undated_edge("a", "owns", "b");
        let predictions = apply_derive_edge_at(&rule, &[e], "2024-01-01T00:00:00Z");
        assert_eq!(predictions.len(), 1);
    }

    // ── score_split: hand-computed precision/recall ───────────────────────────

    #[test]
    fn score_split_hand_computed_precision_and_recall() {
        // Setup:
        // - At as_of, edges a→b (supplier_of) and b→x (listed_on) are active.
        //   The rule would predict a→x (exposed_to).
        // - In the forward window, a→x (exposed_to) actually materialises.
        // - So: predictions = {a→x}, outcomes = {a→x} → precision=1, recall=1.
        let rule = derive_rule("rule-1", &["supplier_of", "listed_on"], "exposed_to");

        let e1 = dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z");
        let e2 = dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z");
        // Outcome: exposed_to a→x realises in the forward window.
        let outcome = dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z");

        let sp = split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z");
        let score = score_split(&rule, &[e1, e2, outcome], &sp);

        assert_eq!(score.split_id, "s1");
        assert_eq!(score.predictions, 1);
        assert_eq!(score.support, 1);
        assert!(
            (score.precision - 1.0).abs() < 1e-12,
            "precision={}",
            score.precision
        );
        assert!(
            (score.recall - 1.0).abs() < 1e-12,
            "recall={}",
            score.recall
        );
    }

    #[test]
    fn score_split_false_positive_lowers_precision() {
        // Rule predicts a→x (via a→b→x at as_of) but a→x does NOT realise in window.
        // predictions = {a→x}, outcomes = {} → precision = 1.0 (no FPs), recall = 1.0
        // Wait — outcomes is empty → recall vacuously 1.0; precision: 1 pred, 0 hits
        // → precision = 0/1 = 0.0.
        let rule = derive_rule("rule-fp", &["supplier_of", "listed_on"], "exposed_to");

        let e1 = dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z");
        let e2 = dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z");
        // No outcome in the window.

        let sp = split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z");
        let score = score_split(&rule, &[e1, e2], &sp);

        assert_eq!(score.predictions, 1);
        assert_eq!(score.support, 0); // no outcomes
        // precision = 0 hits / 1 prediction = 0.0
        assert!(
            score.precision.abs() < 1e-12,
            "precision={}",
            score.precision
        );
        // recall = vacuous 1.0 (no outcomes to miss)
        assert!(
            (score.recall - 1.0).abs() < 1e-12,
            "recall={}",
            score.recall
        );
    }

    #[test]
    fn score_split_false_negative_lowers_recall() {
        // Outcome realises (a→x) but rule has no path to predict it (missing hop).
        // predictions = {}, outcomes = {a→x} → precision = vacuous 1.0, recall = 0.0
        let rule = derive_rule("rule-fn", &["supplier_of", "listed_on"], "exposed_to");

        // Only listed_on at as_of — supplier_of is missing, so the chain can't complete.
        let e2 = dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z");
        let outcome = dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z");

        let sp = split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z");
        let score = score_split(&rule, &[e2, outcome], &sp);

        assert_eq!(score.predictions, 0);
        assert_eq!(score.support, 1);
        // precision = vacuous 1.0 (no predictions → no false positives)
        assert!(
            (score.precision - 1.0).abs() < 1e-12,
            "precision={}",
            score.precision
        );
        // recall = 0 / 1 = 0.0
        assert!(score.recall.abs() < 1e-12, "recall={}", score.recall);
    }

    // ── run_replay_eval: deterministic across two runs ────────────────────────

    #[test]
    fn replay_eval_is_deterministic_across_two_runs() {
        let rule = derive_rule("r", &["supplier_of", "listed_on"], "exposed_to");
        let edges = vec![
            dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z"),
            dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z"),
        ];
        let splits = vec![
            split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z"),
            split("s2", "2023-06-01T00:00:00Z", "2024-01-01T00:00:00Z"),
        ];

        let report_a = run_replay_eval(&rule, &edges, &splits, default_drift_key());
        let report_b = run_replay_eval(&rule, &edges, &splits, default_drift_key());

        assert_eq!(report_a, report_b, "replay eval must be deterministic");
    }

    // ── run_replay_eval: mean is macro-average across splits ──────────────────

    #[test]
    fn replay_eval_mean_is_macro_average() {
        // Split 1: precision=1.0, recall=1.0 (hit).
        // Split 2: precision=0.0, recall=0.0 (miss — outcome absent from preds window).
        // Mean precision = (1.0 + 0.0) / 2 = 0.5; mean recall = (1.0 + 0.0) / 2 = 0.5.
        // But: split 2 has outcome that the rule cannot predict (no path at s2.as_of).
        let rule = derive_rule("r", &["supplier_of", "listed_on"], "exposed_to");

        // Both supplier_of and listed_on exist from 2023-01-01.
        // At s1 (as_of=2024-01-01), both are active → prediction a→x.
        // At s2 (as_of=2022-01-01), neither exists yet → no prediction.
        let edges = vec![
            dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z"),
            dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            // Outcome at s1 window.
            dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z"),
            // Outcome at s2 window.
            dated_edge("a", "exposed_to", "x", "2022-06-01T00:00:00Z"),
        ];
        let splits = vec![
            split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z"),
            split("s2", "2022-01-01T00:00:00Z", "2023-01-01T00:00:00Z"),
        ];

        let report = run_replay_eval(&rule, &edges, &splits, default_drift_key());

        // s1: predictions=1, outcomes=1 → precision=1.0, recall=1.0.
        // s2: predictions=0, outcomes=1 → precision=vacuous 1.0, recall=0.0.
        // mean_precision = (1.0 + 1.0) / 2 = 1.0.
        // mean_recall    = (1.0 + 0.0) / 2 = 0.5.
        assert!(
            (report.mean_precision - 1.0).abs() < 1e-12,
            "mean_precision={}",
            report.mean_precision
        );
        assert!(
            (report.mean_recall - 0.5).abs() < 1e-12,
            "mean_recall={}",
            report.mean_recall
        );
        assert_eq!(report.total_support, 2); // 1 outcome per split
    }

    // ── promote_gate threshold tests ──────────────────────────────────────────

    #[test]
    fn promote_gate_clears_when_both_metrics_meet_threshold() {
        let rule = derive_rule("r", &["supplier_of", "listed_on"], "exposed_to");
        let edges = vec![
            dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z"),
            dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z"),
        ];
        let splits = vec![split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z")];
        // Threshold below both 1.0 values → promote.
        let threshold = PromoteThreshold {
            min_precision: 0.60,
            min_recall: 0.20,
        };
        let verdict = promote_gate(&rule, &edges, &splits, &threshold, default_drift_key());

        assert!(verdict.is_promote(), "verdict: {:?}", verdict.summary());
    }

    #[test]
    fn promote_gate_rejects_when_recall_below_threshold() {
        // Rule has no valid chain at as_of → recall = 0.0.
        let rule = derive_rule("r-low-recall", &["supplier_of", "listed_on"], "exposed_to");
        let edges = vec![
            // listed_on exists but supplier_of is absent → chain cannot complete.
            dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            // Outcome in window — but no prediction.
            dated_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z"),
        ];
        let splits = vec![split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z")];
        let threshold = PromoteThreshold {
            min_precision: 0.0,
            min_recall: 0.50,
        };
        let verdict = promote_gate(&rule, &edges, &splits, &threshold, default_drift_key());

        assert!(
            !verdict.is_promote(),
            "expected Reject; got: {:?}",
            verdict.summary()
        );
        assert!(
            verdict.summary().contains("recall"),
            "summary={}",
            verdict.summary()
        );
    }

    #[test]
    fn promote_gate_rejects_when_precision_below_threshold() {
        // Rule predicts a→x (via chain) but a→x does NOT realise → precision=0.
        let rule = derive_rule(
            "r-low-precision",
            &["supplier_of", "listed_on"],
            "exposed_to",
        );
        let edges = vec![
            dated_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z"),
            dated_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            // No outcome for a→x in window — the prediction is a false positive.
        ];
        let splits = vec![split("s1", "2024-01-01T00:00:00Z", "2024-07-01T00:00:00Z")];
        let threshold = PromoteThreshold {
            min_precision: 0.50,
            min_recall: 0.0,
        };
        let verdict = promote_gate(&rule, &edges, &splits, &threshold, default_drift_key());

        assert!(
            !verdict.is_promote(),
            "expected Reject; got: {:?}",
            verdict.summary()
        );
        assert!(
            verdict.summary().contains("precision"),
            "summary={}",
            verdict.summary()
        );
    }

    #[test]
    fn promote_gate_vacuous_promote_on_empty_splits() {
        // No splits → vacuous promote (nothing to fail on).
        let rule = derive_rule("r", &["owns"], "related_to");
        let threshold = PromoteThreshold::default();
        let verdict = promote_gate(&rule, &[], &[], &threshold, default_drift_key());
        assert!(verdict.is_promote(), "empty splits must vacuously promote");
    }

    // ── serde round-trip ──────────────────────────────────────────────────────

    #[test]
    fn report_round_trips_through_json() {
        let report = RuleReplayReport {
            rule_id: "r".to_string(),
            mean_precision: 0.8,
            mean_recall: 0.6,
            total_support: 5,
            splits: vec![SplitReplayScore {
                split_id: "s1".to_string(),
                precision: 0.8,
                recall: 0.6,
                support: 5,
                predictions: 4,
            }],
            drift_key: default_drift_key(),
        };

        let json = serde_json::to_string(&report).expect("serialize");
        let restored: RuleReplayReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, restored);
    }

    #[test]
    fn promote_verdict_round_trips_through_json() {
        let promote = PromoteVerdict::Promote {
            summary: "ok".to_string(),
        };
        let reject = PromoteVerdict::Reject {
            summary: "fail".to_string(),
        };
        for v in [&promote, &reject] {
            let json = serde_json::to_string(v).expect("serialize");
            let restored: PromoteVerdict = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(v, &restored);
        }
    }

    // ── self-loop exclusion ───────────────────────────────────────────────────

    #[test]
    fn self_loops_are_excluded_from_predictions() {
        // A -r1-> A -r2-> X  chain would produce A -derived-> X, not A -derived-> A.
        // But A -r1-> A -r2-> A (full self-loop chain) must be excluded.
        let rule = derive_rule("r", &["r1", "r2"], "derived");
        let edges = vec![undated_edge("a", "r1", "a"), undated_edge("a", "r2", "a")];
        let predictions = apply_derive_edge_at(&rule, &edges, "2024-01-01T00:00:00Z");
        assert!(
            predictions.is_empty(),
            "self-loop prediction must be excluded: {:?}",
            predictions
        );
    }

    // ── PropagateTag rules produce empty predictions ─────────────────────────

    #[test]
    fn propagate_tag_rule_produces_empty_predictions() {
        let rule = InferRule::PropagateTag {
            rule_id: "pt".to_string(),
            stratum: 0,
            tag: "risk:sanctioned".to_string(),
            include_descendants: false,
            along: vec!["owned_by".to_string()],
            outbound: true,
            max_hops: 1,
        };
        let edges = vec![undated_edge("a", "owned_by", "b")];
        let predictions = apply_derive_edge_at(&rule, &edges, "2024-01-01T00:00:00Z");
        assert!(
            predictions.is_empty(),
            "PropagateTag has no edge predictions"
        );

        let outcomes = collect_outcomes(
            &rule,
            &edges,
            "2024-01-01T00:00:00Z",
            "2024-07-01T00:00:00Z",
        );
        assert!(outcomes.is_empty(), "PropagateTag has no edge outcomes");
    }

    // ── ReplayEvalConfig ──────────────────────────────────────────────────────

    #[test]
    fn default_config_validates() {
        assert!(ReplayEvalConfig::default().validate().is_ok());
    }

    #[test]
    fn default_config_is_disabled() {
        assert!(!ReplayEvalConfig::default().enabled);
    }

    #[test]
    fn disabled_status_is_not_empty() {
        let status = replay_disabled_status();
        assert!(
            status.contains("DISABLED"),
            "status must say DISABLED: {status}"
        );
        assert!(
            status.contains("enabled = true"),
            "status must tell how to enable: {status}"
        );
    }

    #[test]
    fn invalid_cadence_fails_validation() {
        let cfg = ReplayEvalConfig {
            cadence: "not a cron".to_string(),
            ..ReplayEvalConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ReplayEvalConfigError::InvalidCadence { .. })
        ));
    }

    #[test]
    fn threshold_out_of_range_fails_validation() {
        let cfg = ReplayEvalConfig {
            min_precision: 1.1,
            ..ReplayEvalConfig::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(ReplayEvalConfigError::ThresholdOutOfRange {
                field: "min_precision",
                ..
            })
        ));
    }

    #[test]
    fn replay_trigger_id_is_namespaced_and_stable() {
        assert_eq!(
            replay_trigger_id("supplier-exposure"),
            "knowledge-replay-eval:supplier-exposure"
        );
        assert_ne!(replay_trigger_id("r1"), replay_trigger_id("r2"));
    }

    #[test]
    fn config_deserializes_with_defaults_from_empty_object() {
        let cfg: ReplayEvalConfig =
            serde_json::from_str("{}").expect("empty object must deserialize");
        assert!(!cfg.enabled);
        assert_eq!(cfg.cadence, "0 4 * * MON");
        assert!((cfg.min_precision - 0.60).abs() < f64::EPSILON);
        assert!((cfg.min_recall - 0.20).abs() < f64::EPSILON);
    }
}
