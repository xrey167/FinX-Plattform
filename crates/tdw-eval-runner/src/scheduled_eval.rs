//! In-daemon scheduled self-eval for retrieval quality (knowledge-system K-L3).
//!
//! [`ScheduledEvalConfig`] — the `[knowledge.evals]` config block.
//! [`EvalFreshness`] — re-exported from `tdw-knowledge` (defined there to avoid
//! a circular dependency: `tdw-eval-runner → tdw-knowledge → tdw-eval-runner`).
//! [`run_scheduled_eval`] — the pure eval+compare logic, called by the worker
//! when the cron trigger fires; returns a [`ScheduledEvalOutcome`] (caller owns
//! persistence and cron registration).
//!
//! # Zero-feeds honesty
//!
//! If no golden split is configured (`split_id` is `None` or empty), the
//! caller should register no trigger and report [`EvalFreshness::Unconfigured`].
//! No silent do-nothing: the status field loudly says why no eval is running.
//!
//! # Cron registration is caller-owned
//!
//! This module is intentionally free of `tdw-cron` / `tdw-worker` dependencies
//! to avoid the cycle `tdw-cron → tdw-worker → tdw-service-api → tdw-eval-runner
//! → tdw-cron`.  The daemon wires the trigger using `tdw-cron::ScheduleRegistry`
//! directly, guided by `ScheduledEvalConfig::cadence` and
//! `eval_trigger_id(split_id)`.
//!
//! # Injected-now contract
//!
//! All functions that need a clock accept `now_ms: i64` (epoch milliseconds).
//! Nothing here reads the wall clock directly; the spawning daemon passes the
//! injected-now value, making the logic fully testable.
//!
//! # Persistence is caller-owned
//!
//! [`run_scheduled_eval`] returns a [`ScheduledEvalOutcome`] — the caller
//! decides where to persist it (in-memory cell, database, etc.).  This module
//! has no storage dependency.

#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::baseline_comparator::{RegressionThresholds, RegressionVerdict, compare_to_baseline};
use crate::retrieval_eval::{
    DriftKey, RetrievalEvalCase, RetrievalEvalReport, build_in_memory_retriever, run_retrieval_eval,
};

pub use tdw_embed::EmbeddingProvider;
pub use tdw_knowledge::KnowledgeDocument;
// EvalFreshness is defined in tdw-knowledge to avoid a circular dependency
// (tdw-eval-runner → tdw-knowledge → tdw-eval-runner).  Re-exported here so
// callers of this module get the type from a single import path.
pub use tdw_knowledge::runtime::EvalFreshness;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// The `[knowledge.evals]` configuration block.
///
/// Serde defaults: `split_id` = `None` (eval disabled), `cadence` = weekly
/// (`"0 3 * * MON"`), thresholds = 5 pp per metric, `split_fixture_path` =
/// `None` (falls back to the embedded fixture at
/// `baselines/<split_id>.json` relative to the `tdw-eval-runner` crate root).
///
/// Validation: `cadence` must be a valid 5-field cron expression (validated by
/// parsing it with the raw `cron` crate); thresholds must be in `[0.0, 1.0]`.
///
/// # Cron registration
///
/// The daemon reads `cadence` and `split_id` to build a `tdw-cron`
/// `ScheduledTrigger` with id `eval_trigger_id(split_id)`.  This crate does not
/// import `tdw-cron` to avoid a dependency cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduledEvalConfig {
    /// The `fixed_split_id` identifying which golden case set to run.
    ///
    /// When `None` or empty the daemon registers no trigger and status reports
    /// `Unconfigured` — loudly, not silently.
    #[serde(default)]
    pub split_id: Option<String>,

    /// 5-field cron expression for the eval cadence.
    ///
    /// Default: `"0 3 * * MON"` (weekly, Monday 03:00 UTC).
    ///
    /// The daemon expands this to a 7-field form (`"0 <expr> *"`) before
    /// passing it to `CronSchedule::parse`, matching `tdw-cron`'s convention.
    #[serde(default = "default_cadence")]
    pub cadence: String,

    /// Maximum allowed drop in mean recall\@k before a regression is declared.
    #[serde(default = "default_threshold")]
    pub max_recall_drop: f64,

    /// Maximum allowed drop in MRR before a regression is declared.
    #[serde(default = "default_threshold")]
    pub max_mrr_drop: f64,

    /// Maximum allowed drop in mean nDCG\@k before a regression is declared.
    #[serde(default = "default_threshold")]
    pub max_ndcg_drop: f64,

    /// Filesystem path to the golden-split fixture file (JSON).
    ///
    /// The fixture file must be a JSON object matching the [`GoldenSplitFixture`]
    /// shape: `{ "documents": [...], "cases": [...], "baseline": <report> }`.
    /// `baseline` may be `null` — that signals a first-run state where no
    /// prior baseline exists (see [`run_scheduled_eval_from_fixture`]).
    ///
    /// When `None`, the worker falls back to the embedded fixture at
    /// `<crate-root>/baselines/<split_id>.json`.  When the resolved path does
    /// not exist or fails to parse, the worker writes
    /// [`EvalFreshness::Stale`] to the cell and emits no alarm — a missing or
    /// corrupt fixture is a configuration error, never a retrieval regression.
    #[serde(default)]
    pub split_fixture_path: Option<String>,
}

fn default_cadence() -> String {
    "0 3 * * MON".to_string()
}

const fn default_threshold() -> f64 {
    0.05
}

impl Default for ScheduledEvalConfig {
    fn default() -> Self {
        Self {
            split_id: None,
            cadence: default_cadence(),
            max_recall_drop: default_threshold(),
            max_mrr_drop: default_threshold(),
            max_ndcg_drop: default_threshold(),
            split_fixture_path: None,
        }
    }
}

/// Error returned by config validation.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ScheduledEvalConfigError {
    /// The cron expression cannot be parsed.
    #[error("invalid cadence expression {expr:?}: {detail}")]
    InvalidCadence {
        /// The expression that failed.
        expr: String,
        /// The underlying cron parse error description.
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

impl ScheduledEvalConfig {
    /// Validate this config.
    ///
    /// Parses the 5-field `cadence` expression after expanding it to a 7-field
    /// form (`"0 <expr> *"`) — the same expansion `tdw-cron`'s `CronSchedule`
    /// applies — so invalid expressions are caught here without importing
    /// `tdw-cron`.
    ///
    /// # Errors
    ///
    /// Returns [`ScheduledEvalConfigError`] if the cadence is not a valid 5-field
    /// cron expression or a threshold is outside `[0.0, 1.0]`.
    pub fn validate(&self) -> Result<(), ScheduledEvalConfigError> {
        // Expand 5-field to 7-field: "0 <expr> *" — same as tdw-cron::CronSchedule.
        let seven_field = format!("0 {} *", self.cadence);
        seven_field.parse::<cron::Schedule>().map_err(|err| {
            ScheduledEvalConfigError::InvalidCadence {
                expr: self.cadence.clone(),
                detail: err.to_string(),
            }
        })?;
        for (field, value) in [
            ("max_recall_drop", self.max_recall_drop),
            ("max_mrr_drop", self.max_mrr_drop),
            ("max_ndcg_drop", self.max_ndcg_drop),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(ScheduledEvalConfigError::ThresholdOutOfRange { field, value });
            }
        }
        Ok(())
    }

    /// Extract [`RegressionThresholds`] from this config.
    #[must_use]
    pub const fn thresholds(&self) -> RegressionThresholds {
        RegressionThresholds {
            max_recall_drop: self.max_recall_drop,
            max_mrr_drop: self.max_mrr_drop,
            max_ndcg_drop: self.max_ndcg_drop,
        }
    }
}

// ---------------------------------------------------------------------------
// Golden-split fixture — the checked-in corpus+cases+baseline bundle
// ---------------------------------------------------------------------------

/// The checked-in fixture that feeds one cron-triggered eval run.
///
/// A `GoldenSplitFixture` bundles everything the worker needs at fire time:
/// the fixed document corpus, the fixed case set, and the persisted baseline
/// report to compare against.  All three are **checked into the repository**
/// and never generated at runtime.
///
/// # Baseline `None` = first-run
///
/// When `baseline` is `null` (or absent) in the fixture JSON, no prior
/// baseline exists.  The worker treats this as a **first-run** state: it runs
/// the eval, records the result as `Green` with summary
/// `"first-run: no prior baseline — current metrics recorded"`, and does **not**
/// alarm.  Operators bless the baseline by running the golden-split gate test
/// (see `retrieval_eval.rs`) and checking the printed report into the fixture
/// file.
///
/// # Zero-cases posture
///
/// An empty `cases` list is structurally incapable of producing a
/// `Regressed` result.  The worker writes `Stale` with notice
/// `"split fixture has no eval cases"` and does **not** alarm.  This prevents
/// a misconfigured or incomplete fixture from silently generating spurious
/// alarms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoldenSplitFixture {
    /// The fixed document corpus that seeds the in-memory retriever.
    pub documents: Vec<KnowledgeDocument>,
    /// The fixed golden case set.
    pub cases: Vec<RetrievalEvalCase>,
    /// The persisted baseline report to compare against.  `None` = first-run.
    pub baseline: Option<RetrievalEvalReport>,
}

/// Load a [`GoldenSplitFixture`] from a filesystem path.
///
/// # Errors
///
/// Returns a `String` describing the failure (file not found, invalid JSON,
/// schema mismatch).  The caller writes [`EvalFreshness::Stale`] on error
/// and does **not** alarm — a missing or corrupt fixture is a configuration
/// error, not a retrieval regression.
pub fn load_golden_split(path: &str) -> Result<GoldenSplitFixture, String> {
    let json = std::fs::read_to_string(path)
        .map_err(|err| format!("cannot read golden-split fixture {path:?}: {err}"))?;
    serde_json::from_str(&json)
        .map_err(|err| format!("invalid JSON in golden-split fixture {path:?}: {err}"))
}

/// The default fixture directory relative to the `tdw-eval-runner` crate root.
///
/// `baselines/<split_id>.json`.  Used when `split_fixture_path` is `None`.
/// The embedded `golden-split-v1.json` ships with the crate and is kept in
/// sync by the nightly CI golden-split gate (see `retrieval_eval.rs`).
#[must_use]
pub fn default_fixture_path(split_id: &str) -> String {
    format!("{}/baselines/{split_id}.json", env!("CARGO_MANIFEST_DIR"))
}

// ---------------------------------------------------------------------------
// run_scheduled_eval_from_fixture — the safe, no-false-alarm entry point
// ---------------------------------------------------------------------------

/// Run the scheduled eval using a loaded [`GoldenSplitFixture`], guarding
/// against every no-corpus / first-run / missing-baseline state.
///
/// This is the entry point the daemon worker calls after loading the fixture.
/// It enforces the zero-false-alarm contract:
///
/// - **Empty cases** → `Stale("split fixture has no eval cases")`, no alarm.
/// - **No baseline** (first-run) → runs eval, returns `Green` with summary
///   `"first-run: no prior baseline — current metrics recorded"`, no alarm.
/// - **Baseline present** → real comparison; `Regressed` only on genuine
///   metric drop.
///
/// # Errors
///
/// Propagates `tdw_core::Error` from the retriever build or eval runner.
/// The caller should write `Stale` on error.
pub async fn run_scheduled_eval_from_fixture(
    embedder: Arc<dyn EmbeddingProvider>,
    fixture: &GoldenSplitFixture,
    config: &ScheduledEvalConfig,
    now_ms: i64,
) -> tdw_core::Result<ScheduledEvalOutcome> {
    // Zero-cases posture: structurally incapable of Regressed.
    if fixture.cases.is_empty() {
        let split_id = config
            .split_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let stale = EvalFreshness::Stale {
            last_run_ms: now_ms,
            split_id,
            error: "split fixture has no eval cases".to_string(),
        };
        // Synthesise a sentinel outcome so callers can write the cell uniformly.
        let empty_report = RetrievalEvalReport {
            k: 0,
            mean_recall_at_k: 0.0,
            mrr: 0.0,
            mean_ndcg_at_k: 0.0,
            cases: vec![],
            drift_key: DriftKey {
                embedder_model: embedder.model_id().to_string(),
                rules_version: None,
                infer_version: None,
            },
        };
        let empty_verdict = RegressionVerdict {
            is_regression: false,
            summary: "no cases".to_string(),
            deltas: vec![],
            baseline_drift_key: empty_report.drift_key.clone(),
            current_drift_key: empty_report.drift_key.clone(),
            drift_key_changed: false,
        };
        return Ok(ScheduledEvalOutcome {
            report: empty_report,
            verdict: empty_verdict,
            freshness: stale,
            ran_at_ms: now_ms,
        });
    }

    let drift_key = DriftKey {
        embedder_model: embedder.model_id().to_string(),
        rules_version: None,
        infer_version: None,
    };

    match &fixture.baseline {
        None => {
            // First-run: run eval, return Green with notice — never alarm.
            let split_id = config
                .split_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            let retriever =
                build_in_memory_retriever(Arc::clone(&embedder), fixture.documents.clone()).await?;
            let report =
                run_retrieval_eval(&retriever, &fixture.cases, 3, drift_key.clone()).await?;
            let summary = "first-run: no prior baseline — current metrics recorded".to_string();
            let verdict = RegressionVerdict {
                is_regression: false,
                summary: summary.clone(),
                deltas: vec![],
                baseline_drift_key: drift_key.clone(),
                current_drift_key: drift_key,
                drift_key_changed: false,
            };
            Ok(ScheduledEvalOutcome {
                report,
                verdict,
                freshness: EvalFreshness::Green {
                    last_run_ms: now_ms,
                    split_id,
                    summary,
                },
                ran_at_ms: now_ms,
            })
        }
        Some(baseline) => {
            // Normal path: real comparison against persisted baseline.
            run_scheduled_eval(
                embedder,
                fixture.documents.clone(),
                &fixture.cases,
                drift_key,
                3,
                baseline,
                config,
                now_ms,
            )
            .await
        }
    }
}

// ---------------------------------------------------------------------------
// Stable trigger id helper (daemon uses this to build the ScheduledTrigger)
// ---------------------------------------------------------------------------

/// The stable cron trigger id for the knowledge self-eval.
///
/// Format: `knowledge-self-eval:<split_id>`.  Stable so `tdw-cron`'s
/// `due_triggers` deduplicates correctly across restarts.
///
/// The daemon calls this to build its `ScheduledTrigger`:
/// ```ignore
/// registry.add(ScheduledTrigger {
///     id: eval_trigger_id(&config.split_id),
///     schedule: CronSchedule::parse(&config.cadence)?,
///     action: /* Op::RunKnowledgeEval envelope */,
/// });
/// ```
#[must_use]
pub fn eval_trigger_id(split_id: &str) -> String {
    format!("knowledge-self-eval:{split_id}")
}

// ---------------------------------------------------------------------------
// EvalAlertSink — seam for routing regression alerts
// ---------------------------------------------------------------------------

/// Receives a formatted regression-alert body after a scheduled eval run
/// determines that metrics have fallen below threshold.
///
/// The sink is called synchronously on the eval-worker task; implementations
/// must not block indefinitely.  Any error is logged and does **not** abort
/// the worker — the freshness cell is already written before `notify` is
/// called.
///
/// # Production wiring
///
/// The composition root (`tdw-backend`) passes `None` today, which causes the
/// worker to fall back to an `eprintln!` log line (visible in daemon stdout and
/// captured by any log aggregator that tails the process).  When a durable
/// notification channel (e.g. `tdw-alerts` Postgres sink or webhook) is wired
/// into the daemon, the composition root passes `Some(Arc::new(<impl>))`.
///
/// # Testing
///
/// Tests pass a `MockEvalAlertSink` that records calls so assertions can verify
/// that the alert was emitted for a regressed outcome.
pub trait EvalAlertSink: Send + Sync {
    /// Receive a formatted alert body for a regression event.
    ///
    /// `split_id` identifies which golden split triggered the regression.
    /// `body` is the output of [`regression_alert_body`].
    fn notify(&self, split_id: &str, body: &str);
}

/// A no-op [`EvalAlertSink`] that discards every notification.
///
/// Use in production when no external alert channel is configured — the
/// `eprintln!` fallback in the eval worker still fires.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopEvalAlertSink;

impl EvalAlertSink for NoopEvalAlertSink {
    fn notify(&self, _split_id: &str, _body: &str) {}
}

// ---------------------------------------------------------------------------
// Scheduled eval outcome (caller-owned persistence)
// ---------------------------------------------------------------------------

/// Result of one scheduled eval run returned to the caller.
///
/// The caller decides where to store this (in-memory cell, database row,
/// etc.).  Persistence is always the caller's responsibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduledEvalOutcome {
    /// The fresh eval report.
    pub report: RetrievalEvalReport,
    /// Comparison verdict against the baseline.
    pub verdict: RegressionVerdict,
    /// The derived freshness status.
    pub freshness: EvalFreshness,
    /// Epoch-ms when the eval ran.
    pub ran_at_ms: i64,
}

// ---------------------------------------------------------------------------
// run_scheduled_eval — the pure eval+compare logic
// ---------------------------------------------------------------------------

/// Run the in-memory golden-split regression detection eval and return the
/// outcome.
///
/// This function implements **B11 deterministic regression detection**: it
/// builds a fresh in-memory retriever seeded with the provided document corpus,
/// runs the fixed golden case set through it, and compares aggregate metrics
/// against the persisted baseline.  It does **not** measure live-corpus quality
/// — the corpus is the same fixed document set used to generate the baseline,
/// so results are fully deterministic and reproducible offline.  Operators who
/// want live-corpus quality measurement should run a separate eval against the
/// production vector store.
///
/// # Parameters
///
/// - `embedder` — the same embedder model as the live runtime (injected, not
///   global); must match the baseline's `drift_key.embedder_model`.
/// - `documents` — the fixed golden-split document corpus that seeds the
///   in-memory retriever.  This is the same corpus used to produce the
///   committed baseline, not the live production corpus.
/// - `cases` — the fixed golden case set for `config.split_id`.
/// - `drift_key` — the current version triple (embedder model + rule-set +
///   infer versions).  Stamped on the fresh report; compared against the
///   baseline key to detect version-bump-caused drift.
/// - `k` — retrieval cutoff (top-k).
/// - `baseline` — the persisted golden-split baseline to compare against.
/// - `config` — the eval config (split id, thresholds).
/// - `now_ms` — injected current time (epoch ms); nothing here reads the
///   wall clock.
///
/// # Errors
///
/// Returns a `tdw_core::Error` if the in-memory retriever fails to build or
/// if the eval runner fails a case.  On error the caller should write
/// [`EvalFreshness::Stale`] to the shared cell with the error message.
#[allow(clippy::too_many_arguments)]
pub async fn run_scheduled_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    documents: Vec<KnowledgeDocument>,
    cases: &[RetrievalEvalCase],
    drift_key: DriftKey,
    k: usize,
    baseline: &RetrievalEvalReport,
    config: &ScheduledEvalConfig,
    now_ms: i64,
) -> tdw_core::Result<ScheduledEvalOutcome> {
    let split_id = config
        .split_id
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let retriever = build_in_memory_retriever(embedder, documents).await?;
    let report = run_retrieval_eval(&retriever, cases, k, drift_key).await?;
    let verdict = compare_to_baseline(baseline, &report, &config.thresholds());

    let freshness = if verdict.is_regression {
        EvalFreshness::Regressed {
            last_run_ms: now_ms,
            split_id,
            summary: verdict.summary.clone(),
            drift_key_changed: verdict.drift_key_changed,
        }
    } else {
        EvalFreshness::Green {
            last_run_ms: now_ms,
            split_id,
            summary: verdict.summary.clone(),
        }
    };

    Ok(ScheduledEvalOutcome {
        report,
        verdict,
        freshness,
        ran_at_ms: now_ms,
    })
}

// ---------------------------------------------------------------------------
// Alert emission helper
// ---------------------------------------------------------------------------

/// Format a human-readable alert body from a regression verdict.
///
/// Suitable for emission via `tdw-alerts` or any structured event channel.
/// Includes per-metric deltas and, when the drift key changed between baseline
/// and current runs, a clearly labelled `[drift-key changed]` note — so
/// operators can immediately determine whether a metric drop is attributable to
/// a version bump (embedder model, rules set, or infer version) rather than a
/// genuine code regression.
///
/// When `verdict.drift_key_changed` is `true`, the body carries the old and
/// new key values so operators can correlate the regression with the specific
/// component version that changed.
#[must_use]
pub fn regression_alert_body(verdict: &RegressionVerdict) -> String {
    let drift_note = if verdict.drift_key_changed {
        format!(
            " [drift-key changed — version bump may explain metric drop: \
             {:?} -> {:?}]",
            verdict.baseline_drift_key, verdict.current_drift_key
        )
    } else {
        String::new()
    };

    let deltas: Vec<String> = verdict
        .deltas
        .iter()
        .map(|d| {
            format!(
                "  {} baseline={:.4} current={:.4} delta={:+.4}{}",
                d.metric,
                d.baseline,
                d.current,
                d.delta,
                if d.regressed { " [REGRESSED]" } else { "" }
            )
        })
        .collect();

    format!(
        "Knowledge retrieval eval regression detected{drift_note}:\n{}\n{}",
        verdict.summary,
        deltas.join("\n")
    )
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

    fn perfect_report() -> RetrievalEvalReport {
        RetrievalEvalReport {
            k: 3,
            mean_recall_at_k: 1.0,
            mrr: 1.0,
            mean_ndcg_at_k: 1.0,
            cases: vec![CaseScore {
                case_id: "q1".to_string(),
                recall_at_k: 1.0,
                reciprocal_rank: 1.0,
                ndcg_at_k: 1.0,
                ranked: vec!["doc-a".to_string()],
            }],
            drift_key: drift_key(),
        }
    }

    fn regressed_report() -> RetrievalEvalReport {
        RetrievalEvalReport {
            k: 3,
            mean_recall_at_k: 0.50, // 0.50 drop from 1.0 > 0.05 threshold
            mrr: 0.50,
            mean_ndcg_at_k: 0.50,
            cases: vec![CaseScore {
                case_id: "q1".to_string(),
                recall_at_k: 0.50,
                reciprocal_rank: 0.50,
                ndcg_at_k: 0.50,
                ranked: vec!["doc-b".to_string()],
            }],
            drift_key: drift_key(),
        }
    }

    fn config_with_split(split: &str) -> ScheduledEvalConfig {
        ScheduledEvalConfig {
            split_id: Some(split.to_string()),
            ..ScheduledEvalConfig::default()
        }
    }

    // ── config validation ────────────────────────────────────────────────────

    #[test]
    fn default_config_validates() {
        assert!(ScheduledEvalConfig::default().validate().is_ok());
    }

    #[test]
    fn invalid_cadence_fails_validation() {
        let cfg = ScheduledEvalConfig {
            cadence: "not a cron".to_string(),
            ..ScheduledEvalConfig::default()
        };
        let err = cfg.validate().expect_err("invalid cadence");
        assert!(matches!(
            err,
            ScheduledEvalConfigError::InvalidCadence { .. }
        ));
    }

    #[test]
    fn threshold_above_one_fails_validation() {
        let cfg = ScheduledEvalConfig {
            max_recall_drop: 1.1,
            ..ScheduledEvalConfig::default()
        };
        let err = cfg.validate().expect_err("threshold > 1.0");
        assert!(matches!(
            err,
            ScheduledEvalConfigError::ThresholdOutOfRange {
                field: "max_recall_drop",
                ..
            }
        ));
    }

    #[test]
    fn threshold_negative_fails_validation() {
        let cfg = ScheduledEvalConfig {
            max_mrr_drop: -0.01,
            ..ScheduledEvalConfig::default()
        };
        let err = cfg.validate().expect_err("negative threshold");
        assert!(matches!(
            err,
            ScheduledEvalConfigError::ThresholdOutOfRange {
                field: "max_mrr_drop",
                ..
            }
        ));
    }

    // ── eval_trigger_id ───────────────────────────────────────────────────────

    #[test]
    fn eval_trigger_id_is_stable_and_namespaced() {
        assert_eq!(
            eval_trigger_id("golden-split-v1"),
            "knowledge-self-eval:golden-split-v1"
        );
        // Different splits → different ids.
        assert_ne!(eval_trigger_id("split-a"), eval_trigger_id("split-b"));
    }

    // ── EvalFreshness ─────────────────────────────────────────────────────────

    #[test]
    fn eval_freshness_unconfigured_is_not_alarm() {
        assert!(!EvalFreshness::Unconfigured.is_alarm());
    }

    #[test]
    fn eval_freshness_green_is_not_alarm() {
        assert!(
            !EvalFreshness::Green {
                last_run_ms: 0,
                split_id: "s".to_string(),
                summary: "OK".to_string(),
            }
            .is_alarm()
        );
    }

    #[test]
    fn eval_freshness_regressed_is_alarm() {
        assert!(
            EvalFreshness::Regressed {
                last_run_ms: 0,
                split_id: "s".to_string(),
                summary: "REGRESSION".to_string(),
                drift_key_changed: false,
            }
            .is_alarm()
        );
    }

    #[test]
    fn eval_freshness_stale_is_alarm() {
        assert!(
            EvalFreshness::Stale {
                last_run_ms: 0,
                split_id: "s".to_string(),
                error: "engine failure".to_string(),
            }
            .is_alarm()
        );
    }

    // ── run_scheduled_eval: injected-now + in-memory correctness ──────────────

    #[tokio::test]
    async fn run_scheduled_eval_green_on_perfect_baseline() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_kg::{Entity, EntityKind};

        let embedder = Arc::new(HashEmbeddingProvider::default());
        let entity = Entity {
            entity_id: "instrument:acme".to_string(),
            kind: EntityKind::Instrument,
            label: "Acme".to_string(),
            aliases: vec![],
        };
        let documents = vec![KnowledgeDocument {
            id: "doc-a".to_string(),
            body: "acme earnings report strong cloud growth".to_string(),
            entity,
            tags: vec![],
            source: None,
            plane: None,
            as_of: None,
            mentions: vec![],
        }];

        let cases = vec![RetrievalEvalCase {
            case_id: "q1".to_string(),
            query: "acme earnings report strong cloud growth".to_string(),
            expected_doc_ids: vec!["doc-a".to_string()],
            as_of: None,
            tags_any: vec![],
            fixed_split_id: Some("test-split".to_string()),
        }];

        let baseline = perfect_report();
        let config = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64; // 2026-06-07 12:00:00 UTC

        let outcome = run_scheduled_eval(
            embedder,
            documents,
            &cases,
            drift_key(),
            3,
            &baseline,
            &config,
            now_ms,
        )
        .await
        .expect("eval runs without error");

        assert_eq!(outcome.ran_at_ms, now_ms);
        assert!(!outcome.verdict.is_regression, "perfect corpus -> green");
        assert!(matches!(outcome.freshness, EvalFreshness::Green { .. }));
    }

    #[tokio::test]
    async fn run_scheduled_eval_regressed_on_low_baseline() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_kg::{Entity, EntityKind};

        let embedder = Arc::new(HashEmbeddingProvider::default());
        let entity = Entity {
            entity_id: "instrument:acme".to_string(),
            kind: EntityKind::Instrument,
            label: "Acme".to_string(),
            aliases: vec![],
        };
        let documents = vec![KnowledgeDocument {
            id: "doc-a".to_string(),
            body: "totally unrelated content".to_string(),
            entity,
            tags: vec![],
            source: None,
            plane: None,
            as_of: None,
            mentions: vec![],
        }];

        // Query that will miss, producing low recall.
        let cases = vec![RetrievalEvalCase {
            case_id: "q1".to_string(),
            query: "acme earnings report strong cloud growth".to_string(),
            expected_doc_ids: vec!["doc-nonexistent".to_string()],
            as_of: None,
            tags_any: vec![],
            fixed_split_id: Some("test-split".to_string()),
        }];

        // Baseline at perfect metrics — the actual run will show regression.
        let baseline = perfect_report();
        let config = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64;

        let outcome = run_scheduled_eval(
            embedder,
            documents,
            &cases,
            drift_key(),
            3,
            &baseline,
            &config,
            now_ms,
        )
        .await
        .expect("eval runs");

        // Actual recall will be 0 (gold doc not in corpus) -> regression vs 1.0 baseline.
        assert!(
            outcome.verdict.is_regression,
            "corpus miss should yield regression; freshness={:?}",
            outcome.freshness
        );
        assert!(matches!(outcome.freshness, EvalFreshness::Regressed { .. }));
    }

    // ── regression_alert_body includes deltas ─────────────────────────────────

    #[test]
    fn regression_alert_body_contains_metric_deltas() {
        let baseline = perfect_report();
        let current = regressed_report();
        let verdict = compare_to_baseline(
            &baseline,
            &current,
            &ScheduledEvalConfig::default().thresholds(),
        );
        let body = regression_alert_body(&verdict);
        assert!(body.contains("REGRESSION"), "body: {body}");
        assert!(body.contains("mean_recall_at_k"), "body: {body}");
        assert!(body.contains("mrr"), "body: {body}");
        assert!(body.contains("mean_ndcg_at_k"), "body: {body}");
    }

    // ── EvalFreshness serde round-trip ────────────────────────────────────────

    #[test]
    fn eval_freshness_round_trips_through_json() {
        let variants: Vec<EvalFreshness> = vec![
            EvalFreshness::Unconfigured,
            EvalFreshness::Pending {
                split_id: "golden-split-v1".to_string(),
            },
            EvalFreshness::Green {
                last_run_ms: 1_749_297_600_000,
                split_id: "golden-split-v1".to_string(),
                summary: "OK on split".to_string(),
            },
            EvalFreshness::Regressed {
                last_run_ms: 1_749_297_600_000,
                split_id: "golden-split-v1".to_string(),
                summary: "REGRESSION on split".to_string(),
                drift_key_changed: true,
            },
            EvalFreshness::Stale {
                last_run_ms: 1_749_297_600_000,
                split_id: "golden-split-v1".to_string(),
                error: "engine failure".to_string(),
            },
        ];
        for v in &variants {
            let json = serde_json::to_string(v).expect("serialize");
            let restored: EvalFreshness = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(v, &restored, "variant round-trip: {json}");
        }
    }

    // ── config serde defaults ─────────────────────────────────────────────────

    #[test]
    fn config_deserializes_with_defaults_from_empty_object() {
        let json = "{}";
        let cfg: ScheduledEvalConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(cfg.split_id, None);
        assert_eq!(cfg.cadence, "0 3 * * MON");
        assert!((cfg.max_recall_drop - 0.05).abs() < f64::EPSILON);
    }

    // ── EvalAlertSink ─────────────────────────────────────────────────────────

    /// A test-only [`EvalAlertSink`] that records the most recent notification.
    struct MockEvalAlertSink {
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }

    impl MockEvalAlertSink {
        fn new() -> Self {
            Self {
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn recorded(&self) -> Vec<(String, String)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl EvalAlertSink for MockEvalAlertSink {
        fn notify(&self, split_id: &str, body: &str) {
            self.calls
                .lock()
                .expect("lock")
                .push((split_id.to_string(), body.to_string()));
        }
    }

    #[test]
    fn eval_alert_sink_mock_receives_regression_alert() {
        let verdict = RegressionVerdict {
            is_regression: true,
            summary: "REGRESSION: recall dropped".to_string(),
            deltas: vec![],
            baseline_drift_key: DriftKey {
                embedder_model: "hash-8".to_string(),
                rules_version: None,
                infer_version: None,
            },
            current_drift_key: DriftKey {
                embedder_model: "hash-8".to_string(),
                rules_version: None,
                infer_version: None,
            },
            drift_key_changed: false,
        };

        let body = regression_alert_body(&verdict);
        let sink = MockEvalAlertSink::new();
        sink.notify("golden-split-v1", &body);

        let calls = sink.recorded();
        assert_eq!(calls.len(), 1, "sink must receive exactly one notification");
        let (split_id, received_body) = &calls[0];
        assert_eq!(split_id, "golden-split-v1");
        assert!(
            received_body.contains("REGRESSION"),
            "body must contain REGRESSION; got: {received_body}"
        );
    }

    #[test]
    fn noop_eval_alert_sink_does_not_panic() {
        let sink = NoopEvalAlertSink;
        // Must not panic regardless of input.
        sink.notify("any-split", "any body");
    }

    // ── GoldenSplitFixture helpers ────────────────────────────────────────────

    fn make_entity(id: &str) -> tdw_kg::Entity {
        tdw_kg::Entity {
            entity_id: id.to_string(),
            kind: tdw_kg::EntityKind::Instrument,
            label: id.to_string(),
            aliases: vec![],
        }
    }

    /// A corpus + cases that score recall = 1.0 with the hash embedder
    /// (exact-body query matches doc-a).
    fn green_fixture() -> GoldenSplitFixture {
        let doc = KnowledgeDocument {
            id: "doc-a".to_string(),
            body: "acme earnings report strong cloud growth".to_string(),
            entity: make_entity("instrument:acme"),
            tags: vec![],
            source: None,
            plane: None,
            as_of: None,
            mentions: vec![],
        };
        let case = RetrievalEvalCase {
            case_id: "earnings".to_string(),
            query: "acme earnings report strong cloud growth".to_string(),
            expected_doc_ids: vec!["doc-a".to_string()],
            as_of: None,
            tags_any: vec![],
            fixed_split_id: Some("test-split".to_string()),
        };
        // Baseline at perfect recall — matching corpus should stay Green.
        let baseline = RetrievalEvalReport {
            k: 3,
            mean_recall_at_k: 1.0,
            mrr: 1.0,
            mean_ndcg_at_k: 1.0,
            cases: vec![],
            drift_key: DriftKey {
                embedder_model: "local-hash-8".to_string(),
                rules_version: None,
                infer_version: None,
            },
        };
        GoldenSplitFixture {
            documents: vec![doc],
            cases: vec![case],
            baseline: Some(baseline),
        }
    }

    /// Same corpus but baseline expects a doc that is NOT in the corpus
    /// (will produce recall = 0 → Regressed vs perfect baseline).
    fn regressed_fixture() -> GoldenSplitFixture {
        let doc = KnowledgeDocument {
            id: "doc-a".to_string(),
            body: "totally unrelated body".to_string(),
            entity: make_entity("instrument:acme"),
            tags: vec![],
            source: None,
            plane: None,
            as_of: None,
            mentions: vec![],
        };
        let case = RetrievalEvalCase {
            case_id: "miss".to_string(),
            query: "acme earnings report strong cloud growth".to_string(),
            expected_doc_ids: vec!["doc-nonexistent".to_string()],
            as_of: None,
            tags_any: vec![],
            fixed_split_id: Some("test-split".to_string()),
        };
        // Perfect baseline — miss will produce regression.
        let baseline = RetrievalEvalReport {
            k: 3,
            mean_recall_at_k: 1.0,
            mrr: 1.0,
            mean_ndcg_at_k: 1.0,
            cases: vec![],
            drift_key: DriftKey {
                embedder_model: "local-hash-8".to_string(),
                rules_version: None,
                infer_version: None,
            },
        };
        GoldenSplitFixture {
            documents: vec![doc],
            cases: vec![case],
            baseline: Some(baseline),
        }
    }

    // ── run_scheduled_eval_from_fixture ───────────────────────────────────────

    #[tokio::test]
    async fn fixture_with_real_corpus_produces_green() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;

        let embedder = Arc::new(HashEmbeddingProvider::default());
        let fixture = green_fixture();
        let cfg = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64;

        let outcome = run_scheduled_eval_from_fixture(embedder, &fixture, &cfg, now_ms)
            .await
            .expect("eval must succeed");

        assert!(
            matches!(outcome.freshness, EvalFreshness::Green { .. }),
            "matching corpus + matching baseline must be Green; got {:?}",
            outcome.freshness
        );
        assert!(!outcome.freshness.is_alarm(), "Green must not alarm");
        assert!(!outcome.verdict.is_regression);
    }

    #[tokio::test]
    async fn fixture_with_injected_drift_produces_regressed() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;

        let embedder = Arc::new(HashEmbeddingProvider::default());
        let fixture = regressed_fixture(); // gold doc absent → recall = 0
        let cfg = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64;

        let outcome = run_scheduled_eval_from_fixture(embedder, &fixture, &cfg, now_ms)
            .await
            .expect("eval must succeed even on regression");

        assert!(
            matches!(outcome.freshness, EvalFreshness::Regressed { .. }),
            "corpus miss with perfect baseline must be Regressed; got {:?}",
            outcome.freshness
        );
        assert!(outcome.freshness.is_alarm(), "Regressed must alarm");
        assert!(outcome.verdict.is_regression);
    }

    #[tokio::test]
    async fn fixture_with_no_baseline_produces_green_first_run() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;

        let embedder = Arc::new(HashEmbeddingProvider::default());
        // Same docs/cases as green_fixture but baseline = None (first run).
        let mut fixture = green_fixture();
        fixture.baseline = None;
        let cfg = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64;

        let outcome = run_scheduled_eval_from_fixture(embedder, &fixture, &cfg, now_ms)
            .await
            .expect("first-run must succeed");

        assert!(
            matches!(outcome.freshness, EvalFreshness::Green { .. }),
            "first-run (no baseline) must be Green; got {:?}",
            outcome.freshness
        );
        assert!(!outcome.freshness.is_alarm(), "first-run must not alarm");
        // Summary must carry the first-run notice.
        if let EvalFreshness::Green { ref summary, .. } = outcome.freshness {
            assert!(
                summary.contains("first-run"),
                "summary must carry first-run notice; got: {summary}"
            );
        }
    }

    #[tokio::test]
    async fn fixture_with_no_cases_produces_stale_no_alarm() {
        use std::sync::Arc;
        use tdw_embed_local::HashEmbeddingProvider;

        let embedder = Arc::new(HashEmbeddingProvider::default());
        let mut fixture = green_fixture();
        fixture.cases.clear(); // zero cases → structurally incapable of Regressed
        let cfg = config_with_split("test-split");
        let now_ms = 1_749_297_600_000_i64;

        let outcome = run_scheduled_eval_from_fixture(embedder, &fixture, &cfg, now_ms)
            .await
            .expect("empty-cases must not propagate an error");

        assert!(
            matches!(outcome.freshness, EvalFreshness::Stale { .. }),
            "empty cases must be Stale; got {:?}",
            outcome.freshness
        );
        // Stale from empty cases IS is_alarm() = true (it's a Stale variant) —
        // but crucially it is NOT Regressed, so no regression alert fires.
        assert!(
            !matches!(outcome.freshness, EvalFreshness::Regressed { .. }),
            "empty cases must never produce Regressed"
        );
    }

    #[test]
    fn load_golden_split_returns_error_for_missing_path() {
        let result = load_golden_split("/nonexistent/path/to/fixture.json");
        let msg = result.expect_err("missing file must return Err, not Ok");
        assert!(
            msg.contains("cannot read golden-split fixture"),
            "error must describe the load failure; got: {msg}"
        );
    }

    #[test]
    fn load_golden_split_returns_error_for_invalid_json() {
        // Write a temp file with bad JSON via std::fs.
        let dir = std::env::temp_dir();
        let path = dir.join("tdw_test_invalid_fixture.json");
        std::fs::write(&path, b"not json at all").expect("write temp file");
        let path_str = path.to_string_lossy().to_string();

        let result = load_golden_split(&path_str);
        let _ = std::fs::remove_file(&path); // cleanup

        let msg = result.expect_err("invalid JSON must return Err");
        assert!(
            msg.contains("invalid JSON"),
            "error must describe JSON parse failure; got: {msg}"
        );
    }

    #[test]
    fn load_golden_split_loads_embedded_fixture() {
        // The crate-embedded golden-split-v1.json must parse without a baseline
        // field being required (baseline is Option).
        let path = default_fixture_path("golden-split-v1");
        // This path is only valid when running inside the workspace (CARGO_MANIFEST_DIR).
        // If the file doesn't exist (unusual CI setup), skip gracefully.
        if !std::path::Path::new(&path).exists() {
            eprintln!("embedded fixture not found at {path}; skipping");
            return;
        }
        let fixture = load_golden_split(&path).expect("embedded fixture must parse");
        // The embedded fixture has 2 cases (earnings + supply).
        assert!(
            !fixture.cases.is_empty(),
            "embedded fixture must have at least one case"
        );
    }
}
