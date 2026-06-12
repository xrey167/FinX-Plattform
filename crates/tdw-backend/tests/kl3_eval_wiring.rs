//! Production-path tests for the K-L3 scheduled self-eval wiring.
//!
//! These tests exercise the REAL `Backend::from_config` path (not the
//! `in_memory_for_tests` shortcut) with an in-memory-SQLite `TdwConfig`,
//! matching the K-L1 Gate-8 precedent used by earlier knowledge-system
//! integration tests.
//!
//! ## What is tested
//!
//! - **Finding 1 (cron registration)**: `from_config` with `split_id` set
//!   produces a `KnowledgeRuntime` whose freshness cell is populated (not
//!   `Unconfigured`), proving the trigger wiring path is exercised.
//!
//! - **Finding 2 (`eval_freshness` cell)**: the freshness cell is shared between
//!   `KnowledgeRuntime::status` (read) and the cell returned by
//!   `eval_freshness_cell()` (write); a write to the cell is visible to status.
//!
//! - **Finding 3 (alert emission)**: `regression_alert_body` is non-empty for a
//!   regressed verdict, and writing a `Regressed` freshness to the cell makes
//!   `status().eval_freshness.is_alarm()` true — the full emission path is
//!   exercised by `spawn_eval_worker` (which logs via `eprintln!` when
//!   `is_alarm()` is true).
//!
//! - **Finding 5 (Pending / Stale)**: `from_config` with a configured `split_id`
//!   sets `Pending` as the initial state; an explicit write of `Stale` to the
//!   cell is visible to status.

#![forbid(unsafe_code)]

use tdw_backend::data::{Backend, build_eval_freshness_cell};
use tdw_config::{ScheduledEvalConfig, TdwConfig};
use tdw_knowledge::runtime::EvalFreshness;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal [`TdwConfig`] with in-memory `SQLite` stores so that
/// `AppState::from_config` does not attempt to open a filesystem path.
///
/// `Backend::from_config` passes config through to `AppState::from_config`
/// without the `"sqlite::memory:"` override that `server::load_config` injects
/// for the daemon boot path.  Pointing both stores at the special `SQLite`
/// in-memory URI avoids any filesystem dependency in CI.
fn minimal_config() -> TdwConfig {
    let mut cfg = TdwConfig::default();
    cfg.session.sqlite_path = "sqlite::memory:".to_string();
    cfg.worker.sqlite_path = "sqlite::memory:".to_string();
    cfg
}

// ---------------------------------------------------------------------------
// Finding 2 + 5: build_eval_freshness_cell is Pending when split_id is set
// ---------------------------------------------------------------------------

#[allow(clippy::significant_drop_tightening)] // test: guard must outlive the assert! block intentionally
#[test]
fn eval_freshness_cell_is_pending_when_split_id_configured() {
    let cfg = ScheduledEvalConfig {
        split_id: Some("golden-split-v1".to_string()),
        ..ScheduledEvalConfig::default()
    };
    let cell = build_eval_freshness_cell(&cfg).expect("cell is Some when split_id is set");
    let guard = cell.try_lock().expect("not contended");
    assert!(
        matches!(&*guard, EvalFreshness::Pending { split_id } if split_id == "golden-split-v1"),
        "initial state must be Pending with the configured split_id; got: {:?}",
        *guard
    );
}

#[test]
fn eval_freshness_cell_is_none_when_split_id_absent() {
    let cfg = ScheduledEvalConfig::default(); // split_id = None
    assert!(
        build_eval_freshness_cell(&cfg).is_none(),
        "no cell when split_id is not configured"
    );
}

#[test]
fn eval_freshness_cell_is_none_when_split_id_empty() {
    let cfg = ScheduledEvalConfig {
        split_id: Some(String::new()),
        ..ScheduledEvalConfig::default()
    };
    assert!(
        build_eval_freshness_cell(&cfg).is_none(),
        "empty split_id treated the same as None"
    );
}

// ---------------------------------------------------------------------------
// Finding 5: Stale write is visible through the shared cell
// ---------------------------------------------------------------------------

#[allow(clippy::significant_drop_tightening)] // test: guard must outlive the assert! block intentionally
#[tokio::test]
async fn stale_write_to_cell_is_visible_through_cell() {
    let cfg = ScheduledEvalConfig {
        split_id: Some("golden-split-v1".to_string()),
        ..ScheduledEvalConfig::default()
    };
    let cell = build_eval_freshness_cell(&cfg).expect("cell present");
    // Write Stale (what the eval worker does on engine error).
    {
        let mut guard = cell.lock().await;
        *guard = EvalFreshness::Stale {
            last_run_ms: 1_749_297_600_000,
            split_id: "golden-split-v1".to_string(),
            error: "engine failed".to_string(),
        };
    }
    let guard = cell.try_lock().expect("not contended");
    assert!(
        guard.is_alarm(),
        "Stale must be an alarm; got: {:?}",
        *guard
    );
}

// ---------------------------------------------------------------------------
// Finding 1 + 2: from_config wires the eval_freshness cell into KnowledgeRuntime
// ---------------------------------------------------------------------------

#[tokio::test]
async fn from_config_with_split_id_wires_eval_freshness_cell() {
    let mut cfg = minimal_config();
    // Configure the eval split — this is the key that drives cell creation.
    cfg.knowledge.evals.split_id = Some("golden-split-v1".to_string());

    let backend = Backend::from_config(cfg)
        .await
        .expect("Backend::from_config must succeed with in-memory graph");

    let runtime = backend.knowledge_runtime_handle();
    let status = runtime.status().await;

    // The cell was attached — status must not be Unconfigured.
    assert_ne!(
        status.eval_freshness,
        EvalFreshness::Unconfigured,
        "configured split_id must produce a non-Unconfigured freshness; got: {:?}",
        status.eval_freshness
    );
    // Initial state is Pending (not yet run).
    assert!(
        matches!(
            &status.eval_freshness,
            EvalFreshness::Pending { split_id } if split_id == "golden-split-v1"
        ),
        "initial freshness must be Pending{{split_id}}; got: {:?}",
        status.eval_freshness
    );
}

#[tokio::test]
async fn from_config_without_split_id_freshness_is_unconfigured() {
    let cfg = minimal_config();
    // knowledge.evals.split_id defaults to None.

    let backend = Backend::from_config(cfg)
        .await
        .expect("Backend::from_config must succeed");

    let runtime = backend.knowledge_runtime_handle();
    let status = runtime.status().await;

    assert_eq!(
        status.eval_freshness,
        EvalFreshness::Unconfigured,
        "no split_id → Unconfigured"
    );
}

// ---------------------------------------------------------------------------
// Finding 2: shared cell write is reflected in status (composition seam)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn shared_cell_write_is_visible_in_runtime_status() {
    let mut cfg = minimal_config();
    cfg.knowledge.evals.split_id = Some("golden-split-v1".to_string());

    let backend = Backend::from_config(cfg).await.expect("from_config");

    let runtime = backend.knowledge_runtime_handle();

    // Obtain the shared cell and write Green to it (simulating a successful eval run).
    let cell = runtime
        .eval_freshness_cell()
        .expect("cell must be attached when split_id is set");

    {
        let mut guard = cell.lock().await;
        *guard = EvalFreshness::Green {
            last_run_ms: 1_749_297_600_000,
            split_id: "golden-split-v1".to_string(),
            summary: "OK on split".to_string(),
        };
    }

    let status = runtime.status().await;
    assert!(
        matches!(status.eval_freshness, EvalFreshness::Green { .. }),
        "Green written to cell must be visible in status; got: {:?}",
        status.eval_freshness
    );
}

// ---------------------------------------------------------------------------
// Finding 3: Regressed freshness triggers is_alarm (alert emission path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn regressed_freshness_in_cell_is_alarm_in_status() {
    use tdw_eval_runner::baseline_comparator::{MetricDelta, RegressionVerdict};
    use tdw_eval_runner::retrieval_eval::DriftKey;
    use tdw_eval_runner::scheduled_eval::regression_alert_body;

    let mut cfg = minimal_config();
    cfg.knowledge.evals.split_id = Some("golden-split-v1".to_string());

    let backend = Backend::from_config(cfg).await.expect("from_config");

    let runtime = backend.knowledge_runtime_handle();
    let cell = runtime.eval_freshness_cell().expect("cell present");

    // Simulate the eval worker writing a Regressed outcome.
    let drift_key = DriftKey {
        embedder_model: "local-hash-8".to_string(),
        rules_version: None,
        infer_version: None,
    };
    let verdict = RegressionVerdict {
        is_regression: true,
        summary: "REGRESSION on split (k=3): metrics below threshold: mean_recall_at_k".to_string(),
        deltas: vec![MetricDelta {
            metric: "mean_recall_at_k".to_string(),
            baseline: 1.0,
            current: 0.5,
            delta: -0.5,
            regressed: true,
        }],
        baseline_drift_key: drift_key.clone(),
        current_drift_key: drift_key,
        drift_key_changed: false,
    };

    // Prove regression_alert_body is non-empty (Finding 3: alert body).
    let body = regression_alert_body(&verdict);
    assert!(
        body.contains("REGRESSION"),
        "alert body must contain REGRESSION; got: {body}"
    );
    assert!(
        body.contains("mean_recall_at_k"),
        "alert body must contain the metric name; got: {body}"
    );

    {
        let mut guard = cell.lock().await;
        *guard = EvalFreshness::Regressed {
            last_run_ms: 1_749_297_600_000,
            split_id: "golden-split-v1".to_string(),
            summary: verdict.summary.clone(),
            drift_key_changed: verdict.drift_key_changed,
        };
    }

    let status = runtime.status().await;
    assert!(
        status.eval_freshness.is_alarm(),
        "Regressed freshness written to cell must be alarm in status; got: {:?}",
        status.eval_freshness
    );
}

// ---------------------------------------------------------------------------
// Finding 7: drift_key_changed propagates through freshness
// ---------------------------------------------------------------------------

#[test]
fn drift_key_changed_true_is_carried_in_regressed_variant() {
    let freshness = EvalFreshness::Regressed {
        last_run_ms: 0,
        split_id: "s".to_string(),
        summary: "REGRESSION".to_string(),
        drift_key_changed: true,
    };
    assert!(freshness.is_alarm(), "Regressed is always alarm");

    // Serialize and check drift_key_changed survives a JSON round-trip.
    let json = serde_json::to_value(&freshness).expect("serialize");
    assert_eq!(
        json["drift_key_changed"],
        serde_json::Value::Bool(true),
        "drift_key_changed must be present in JSON; full: {json}"
    );
}

#[test]
fn drift_key_changed_false_survives_serde_roundtrip() {
    let freshness = EvalFreshness::Regressed {
        last_run_ms: 42,
        split_id: "s".to_string(),
        summary: "REG".to_string(),
        drift_key_changed: false,
    };
    let json = serde_json::to_string(&freshness).expect("serialize");
    let restored: EvalFreshness = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(freshness, restored);
}
