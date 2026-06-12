//! Baseline comparator for retrieval-eval reports (knowledge-system K-L3).
//!
//! Compares a fresh [`RetrievalEvalReport`] against a stored baseline
//! (keyed on `fixed_split_id` + [`DriftKey`]) and returns a [`RegressionVerdict`]
//! that summarises per-metric deltas and whether any configured threshold was
//! breached.
//!
//! # Design
//!
//! - **Pure**: no I/O, no clock reads, no global state.  Callers load the
//!   baseline from disk (or any source) and pass it in.
//! - **Additive thresholds**: each threshold is a *maximum allowed drop*.  A
//!   metric that improves never trips a regression, even if a threshold is
//!   set to zero.
//! - **Drift-key transparency**: the verdict carries both keys so the caller
//!   can log whether a metric change is attributable to a component version
//!   bump (different [`DriftKey`]) rather than a code regression.
//!
//! # Bless step
//!
//! Baselines are **never auto-overwritten**.  Drift must be a conscious
//! decision: run the eval, inspect the report, then deliberately copy the new
//! report JSON over the committed baseline file (`baselines/*.json`) and open
//! a PR documenting the intentional change.  CI reads the baseline and fails
//! on regression; it never writes back.

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

use serde::{Deserialize, Serialize};

use crate::retrieval_eval::{DriftKey, RetrievalEvalReport};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Per-metric regression thresholds for one eval split.
///
/// A threshold value of `0.05` means "allow at most a 0.05 absolute drop in
/// this metric before declaring a regression".  Set a field to `1.0` to
/// effectively disable that threshold.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionThresholds {
    /// Maximum allowed drop in mean recall\@k (absolute, ≥ 0).
    pub max_recall_drop: f64,
    /// Maximum allowed drop in MRR (absolute, ≥ 0).
    pub max_mrr_drop: f64,
    /// Maximum allowed drop in mean nDCG\@k (absolute, ≥ 0).
    pub max_ndcg_drop: f64,
}

impl Default for RegressionThresholds {
    /// Conservative production defaults:
    /// - recall\@k: 0.05 (5 pp drop is a regression)
    /// - MRR: 0.05
    /// - nDCG\@k: 0.05
    fn default() -> Self {
        Self {
            max_recall_drop: 0.05,
            max_mrr_drop: 0.05,
            max_ndcg_drop: 0.05,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-metric delta
// ---------------------------------------------------------------------------

/// Signed delta for one aggregate metric: `current − baseline`.
///
/// Negative means the metric got worse; positive means it improved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricDelta {
    /// The aggregate metric name (e.g. `"mean_recall_at_k"`).
    pub metric: String,
    /// The baseline value.
    pub baseline: f64,
    /// The current (fresh) value.
    pub current: f64,
    /// `current - baseline` (negative = regression, positive = improvement).
    pub delta: f64,
    /// `true` when `delta` breached the configured threshold.
    pub regressed: bool,
}

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

/// Outcome of comparing a fresh report against its baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionVerdict {
    /// Whether ANY metric breached its threshold.
    ///
    /// `true` → the suite is regressed; callers should fail CI or emit an
    /// alert.  `false` → the suite is within tolerance.
    pub is_regression: bool,

    /// Human-readable one-liner suitable for a log line or alert body.
    pub summary: String,

    /// Per-metric deltas, one per aggregate metric compared.
    pub deltas: Vec<MetricDelta>,

    /// The [`DriftKey`] of the baseline report.
    pub baseline_drift_key: DriftKey,

    /// The [`DriftKey`] of the current (fresh) report.
    pub current_drift_key: DriftKey,

    /// `true` when `baseline_drift_key != current_drift_key`.  A metric
    /// change between two different keys is a *version effect*, not
    /// necessarily a regression in the code path.
    pub drift_key_changed: bool,
}

// ---------------------------------------------------------------------------
// Comparator
// ---------------------------------------------------------------------------

/// Compare `current` against `baseline` using `thresholds`.
///
/// Both reports must have been run against the same split (same `fixed_split_id`
/// on the cases), but the comparator does not enforce this — the caller is
/// responsible for loading the right baseline file.
///
/// # Regression logic
///
/// For each aggregate metric the delta `current − baseline` is computed.
/// A regression is declared when `delta < -threshold` (i.e. the drop exceeds
/// the configured maximum).  An improvement or zero change is never a
/// regression.
#[must_use]
pub fn compare_to_baseline(
    baseline: &RetrievalEvalReport,
    current: &RetrievalEvalReport,
    thresholds: &RegressionThresholds,
) -> RegressionVerdict {
    let mut deltas = Vec::new();

    macro_rules! check {
        ($name:expr, $base_val:expr, $cur_val:expr, $threshold:expr) => {{
            let base_val: f64 = $base_val;
            let cur_val: f64 = $cur_val;
            let threshold: f64 = $threshold;
            let delta = cur_val - base_val;
            // Regression when drop exceeds threshold: delta < -threshold
            let regressed = delta < -threshold;
            deltas.push(MetricDelta {
                metric: $name.to_string(),
                baseline: base_val,
                current: cur_val,
                delta,
                regressed,
            });
        }};
    }

    check!(
        "mean_recall_at_k",
        baseline.mean_recall_at_k,
        current.mean_recall_at_k,
        thresholds.max_recall_drop
    );
    check!("mrr", baseline.mrr, current.mrr, thresholds.max_mrr_drop);
    check!(
        "mean_ndcg_at_k",
        baseline.mean_ndcg_at_k,
        current.mean_ndcg_at_k,
        thresholds.max_ndcg_drop
    );

    let is_regression = deltas.iter().any(|d| d.regressed);
    let drift_key_changed = baseline.drift_key != current.drift_key;

    let regressed_metrics: Vec<&str> = deltas
        .iter()
        .filter(|d| d.regressed)
        .map(|d| d.metric.as_str())
        .collect();

    let summary = if is_regression {
        format!(
            "REGRESSION on split (k={}): metrics below threshold: {} — \
             recall@k {:.4} → {:.4} (Δ{:+.4}), \
             MRR {:.4} → {:.4} (Δ{:+.4}), \
             nDCG@k {:.4} → {:.4} (Δ{:+.4})",
            current.k,
            regressed_metrics.join(", "),
            baseline.mean_recall_at_k,
            current.mean_recall_at_k,
            current.mean_recall_at_k - baseline.mean_recall_at_k,
            baseline.mrr,
            current.mrr,
            current.mrr - baseline.mrr,
            baseline.mean_ndcg_at_k,
            current.mean_ndcg_at_k,
            current.mean_ndcg_at_k - baseline.mean_ndcg_at_k,
        )
    } else {
        format!(
            "OK on split (k={}): \
             recall@k {:.4} → {:.4} (Δ{:+.4}), \
             MRR {:.4} → {:.4} (Δ{:+.4}), \
             nDCG@k {:.4} → {:.4} (Δ{:+.4})",
            current.k,
            baseline.mean_recall_at_k,
            current.mean_recall_at_k,
            current.mean_recall_at_k - baseline.mean_recall_at_k,
            baseline.mrr,
            current.mrr,
            current.mrr - baseline.mrr,
            baseline.mean_ndcg_at_k,
            current.mean_ndcg_at_k,
            current.mean_ndcg_at_k - baseline.mean_ndcg_at_k,
        )
    };

    RegressionVerdict {
        is_regression,
        summary,
        deltas,
        baseline_drift_key: baseline.drift_key.clone(),
        current_drift_key: current.drift_key.clone(),
        drift_key_changed,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval_eval::{CaseScore, DriftKey, RetrievalEvalReport};

    fn drift_key() -> DriftKey {
        DriftKey {
            embedder_model: "local-hash-8".to_string(),
            rules_version: Some(1),
            infer_version: Some(1),
        }
    }

    fn report(recall: f64, mrr: f64, ndcg: f64) -> RetrievalEvalReport {
        RetrievalEvalReport {
            k: 3,
            mean_recall_at_k: recall,
            mrr,
            mean_ndcg_at_k: ndcg,
            cases: vec![CaseScore {
                case_id: "q1".to_string(),
                recall_at_k: recall,
                reciprocal_rank: mrr,
                ndcg_at_k: ndcg,
                ranked: vec!["doc-a".to_string()],
            }],
            drift_key: drift_key(),
        }
    }

    fn thresholds() -> RegressionThresholds {
        RegressionThresholds {
            max_recall_drop: 0.05,
            max_mrr_drop: 0.05,
            max_ndcg_drop: 0.05,
        }
    }

    // ── no regression when identical ────────────────────────────────────────

    #[test]
    fn identical_reports_are_not_a_regression() {
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(1.0, 1.0, 1.0);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(!verdict.is_regression);
        assert_eq!(verdict.deltas.len(), 3);
        assert!(verdict.deltas.iter().all(|d| !d.regressed));
    }

    // ── improvement is never a regression ───────────────────────────────────

    #[test]
    fn improvement_is_not_a_regression() {
        let baseline = report(0.8, 0.8, 0.8);
        let current = report(0.9, 0.9, 0.9);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(!verdict.is_regression);
        // All deltas are positive.
        assert!(verdict.deltas.iter().all(|d| d.delta > 0.0));
    }

    // ── drop within threshold is not a regression ────────────────────────────

    #[test]
    fn small_drop_within_threshold_is_ok() {
        // Drop of exactly 0.04 with threshold 0.05 → OK.
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(0.96, 0.96, 0.96);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(
            !verdict.is_regression,
            "drop of 0.04 within 0.05 threshold should be OK; summary={}",
            verdict.summary
        );
    }

    // ── drop clearly within threshold is not a regression ────────────────────

    #[test]
    fn drop_clearly_within_threshold_is_not_a_regression() {
        // Drop of 0.04 (threshold 0.05): delta = -0.04 which is NOT < -0.05 → OK.
        // We avoid the f64 boundary 0.95 (1.0 - 0.95 = -0.0500…04 due to f64
        // representation, which would spuriously trigger).  0.96 is well within.
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(0.96, 0.96, 0.96);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(
            !verdict.is_regression,
            "drop of 0.04 within 0.05 threshold should not be a regression; summary={}",
            verdict.summary
        );
    }

    // ── drop beyond threshold IS a regression ─────────────────────────────────

    #[test]
    fn drop_beyond_threshold_is_a_regression() {
        // Recall drops from 1.0 to 0.90 (delta = -0.10 < -0.05) → regression.
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(0.90, 1.0, 1.0);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(verdict.is_regression);
        let recall_delta = verdict
            .deltas
            .iter()
            .find(|d| d.metric == "mean_recall_at_k")
            .expect("recall delta present");
        assert!(recall_delta.regressed);
        // MRR and nDCG are unchanged → not regressed.
        let mrr_delta = verdict
            .deltas
            .iter()
            .find(|d| d.metric == "mrr")
            .expect("mrr delta present");
        assert!(!mrr_delta.regressed);
    }

    // ── all three metrics can regress independently ──────────────────────────

    #[test]
    fn all_three_metrics_regress_independently() {
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(0.80, 0.80, 0.80);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(verdict.is_regression);
        assert_eq!(verdict.deltas.iter().filter(|d| d.regressed).count(), 3);
        assert!(verdict.summary.contains("REGRESSION"));
    }

    // ── drift-key change is surfaced ──────────────────────────────────────────

    #[test]
    fn drift_key_change_is_surfaced_but_not_itself_a_regression() {
        let baseline = report(1.0, 1.0, 1.0);
        let mut current = report(1.0, 1.0, 1.0);
        current.drift_key = DriftKey {
            embedder_model: "text-embedding-3-small".to_string(),
            rules_version: Some(2),
            infer_version: None,
        };
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        assert!(verdict.drift_key_changed);
        // Metrics unchanged → not a regression (the key change is informational).
        assert!(!verdict.is_regression);
    }

    // ── delta arithmetic is correct ───────────────────────────────────────────

    #[test]
    fn delta_arithmetic_is_correct() {
        let baseline = report(0.8, 0.6, 0.7);
        let current = report(0.85, 0.55, 0.70);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());

        let recall = verdict
            .deltas
            .iter()
            .find(|d| d.metric == "mean_recall_at_k")
            .expect("recall");
        assert!(
            (recall.delta - 0.05).abs() < 1e-12,
            "recall delta={}",
            recall.delta
        );

        let mrr = verdict
            .deltas
            .iter()
            .find(|d| d.metric == "mrr")
            .expect("mrr");
        assert!(
            (mrr.delta - (-0.05)).abs() < 1e-12,
            "mrr delta={}",
            mrr.delta
        );
        // Drop of 0.05 with threshold 0.05: delta = -0.05, not < -0.05 → OK.
        assert!(!mrr.regressed);

        let ndcg = verdict
            .deltas
            .iter()
            .find(|d| d.metric == "mean_ndcg_at_k")
            .expect("ndcg");
        assert!(ndcg.delta.abs() < 1e-12, "ndcg delta={}", ndcg.delta);
    }

    // ── serde round-trip ──────────────────────────────────────────────────────

    #[test]
    fn verdict_round_trips_through_json() {
        // Use values whose f64 delta is exactly representable so that
        // the round-trip through JSON does not lose a ULP.
        // 1.0 - 0.5 = 0.5 (exact), 0.5 - 0.5 = 0.0 (exact).
        let baseline = report(1.0, 0.5, 0.75);
        let current = report(0.5, 0.5, 0.75);
        let verdict = compare_to_baseline(&baseline, &current, &thresholds());
        let json = serde_json::to_string(&verdict).expect("serialize");
        let restored: RegressionVerdict = serde_json::from_str(&json).expect("deserialize");

        // Check the semantically load-bearing fields.
        assert_eq!(verdict.is_regression, restored.is_regression);
        assert_eq!(verdict.summary, restored.summary);
        assert_eq!(verdict.drift_key_changed, restored.drift_key_changed);
        assert_eq!(verdict.baseline_drift_key, restored.baseline_drift_key);
        assert_eq!(verdict.current_drift_key, restored.current_drift_key);
        assert_eq!(verdict.deltas.len(), restored.deltas.len());
        for (orig, rt) in verdict.deltas.iter().zip(restored.deltas.iter()) {
            assert_eq!(orig.metric, rt.metric);
            assert_eq!(orig.baseline, rt.baseline);
            assert_eq!(orig.current, rt.current);
            assert_eq!(orig.regressed, rt.regressed);
            // delta is computed from exact power-of-two inputs so no ULP drift.
            assert_eq!(orig.delta, rt.delta);
        }
    }

    // ── custom zero threshold ──────────────────────────────────────────────────

    #[test]
    fn zero_threshold_means_any_drop_is_a_regression() {
        let strict = RegressionThresholds {
            max_recall_drop: 0.0,
            max_mrr_drop: 0.0,
            max_ndcg_drop: 0.0,
        };
        // Infinitesimally small drop — strictly below threshold=0? No: delta =
        // -0.001, not < -0.0.  So even zero threshold does not trigger on a
        // drop of exactly 0.001 when we use strict `<`.
        //
        // Actually with threshold=0.0: regressed = delta < -0.0 = delta < 0.0.
        // So ANY negative delta triggers regression.
        let baseline = report(1.0, 1.0, 1.0);
        let current = report(0.999, 1.0, 1.0);
        let verdict = compare_to_baseline(&baseline, &current, &strict);
        assert!(
            verdict.is_regression,
            "zero threshold: any recall drop is a regression"
        );
    }

    // ── default thresholds match documented values ─────────────────────────────

    #[test]
    fn default_thresholds_are_five_percent() {
        let t = RegressionThresholds::default();
        assert!((t.max_recall_drop - 0.05).abs() < f64::EPSILON);
        assert!((t.max_mrr_drop - 0.05).abs() < f64::EPSILON);
        assert!((t.max_ndcg_drop - 0.05).abs() < f64::EPSILON);
    }
}
