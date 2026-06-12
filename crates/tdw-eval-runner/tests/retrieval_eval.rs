//! Integration tests for the B11 retrieval evaluation harness in `tdw-eval-runner`.
//!
//! ## What runs in CI vs what is env-gated
//!
//! - **Always runs** (no env var required): `hash_embedder_*` tests that use the
//!   deterministic `HashEmbeddingProvider` + in-memory vector+lexical engines.
//! - **Env-gated (`TDW_LOCAL_MODEL_DIR`)**: `local_model_leg_*` — self-skips when
//!   the var is unset, following the precedent in
//!   `crates/tdw-embed-local/tests/local_model.rs`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use tdw_embed_local::HashEmbeddingProvider;
use tdw_eval_runner::retrieval_eval::{
    DriftKey, RetrievalEvalCase, build_in_memory_retriever, run_retrieval_eval,
};
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::KnowledgeDocument;

// ── fixture helpers ──────────────────────────────────────────────────────────

fn entity(id: &str) -> Entity {
    Entity {
        entity_id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: Vec::new(),
    }
}

/// Three dated documents used across all always-run tests.
///
/// - `doc-old`  (2026-01-01): "acme earnings report strong cloud growth"
/// - `doc-mid`  (2026-02-01): "beta supply chain components note"
/// - `doc-new`  (2026-03-01): "acme earnings beat later quarter"
fn corpus() -> Vec<KnowledgeDocument> {
    vec![
        KnowledgeDocument {
            id: "doc-old".into(),
            body: "acme earnings report strong cloud growth".into(),
            entity: entity("instrument:acme"),
            tags: vec!["sector:tech".into()],
            source: None,
            plane: Some("platform".into()),
            as_of: Some("2026-01-01".into()),
            mentions: Vec::new(),
        },
        KnowledgeDocument {
            id: "doc-mid".into(),
            body: "beta supply chain components note".into(),
            entity: entity("instrument:beta"),
            tags: vec!["sector:industrial".into()],
            source: None,
            plane: Some("platform".into()),
            as_of: Some("2026-02-01".into()),
            mentions: Vec::new(),
        },
        KnowledgeDocument {
            id: "doc-new".into(),
            body: "acme earnings beat later quarter".into(),
            entity: entity("instrument:acme"),
            tags: vec!["sector:tech".into()],
            source: None,
            plane: Some("platform".into()),
            as_of: Some("2026-03-01".into()),
            mentions: Vec::new(),
        },
    ]
}

fn hash_drift_key() -> DriftKey {
    DriftKey {
        embedder_model: "local-hash-8".into(),
        rules_version: Some(1),
        infer_version: Some(1),
    }
}

// ── hash-embedder always-run tests ──────────────────────────────────────────

#[tokio::test]
async fn hash_embedder_scores_fixed_case_set_deterministically() {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let retriever = build_in_memory_retriever(embedder, corpus())
        .await
        .expect("retriever builds");

    let cases = vec![
        RetrievalEvalCase {
            case_id: "earnings".into(),
            query: "acme earnings report strong cloud growth".into(),
            expected_doc_ids: vec!["doc-old".into()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("split-b11".into()),
        },
        RetrievalEvalCase {
            case_id: "supply".into(),
            query: "beta supply chain components note".into(),
            expected_doc_ids: vec!["doc-mid".into()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("split-b11".into()),
        },
    ];

    let report = run_retrieval_eval(&retriever, &cases, 3, hash_drift_key())
        .await
        .expect("eval runs");

    assert_eq!(report.k, 3);
    assert_eq!(report.cases.len(), 2);
    assert_eq!(report.drift_key, hash_drift_key());

    // Both exact-body queries should recall their gold doc.
    assert!(
        (report.mean_recall_at_k - 1.0).abs() < 1e-12,
        "mean recall@3 should be 1.0, got {}",
        report.mean_recall_at_k
    );
    assert!(
        report.mrr > 0.0 && report.mrr <= 1.0,
        "MRR out of range: {}",
        report.mrr
    );

    // Determinism: second run yields byte-identical report.
    let again = run_retrieval_eval(&retriever, &cases, 3, hash_drift_key())
        .await
        .expect("re-run");
    assert_eq!(report, again, "eval must be deterministic");
}

/// Explicit chronological-leakage regression test (CRITICAL house rule).
///
/// Query `as_of = 2026-02-15` must NOT see `doc-new` (dated 2026-03-01).
/// The retriever's temporal filter provides this guarantee structurally.
#[tokio::test]
async fn hash_embedder_temporal_leakage_regression() {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let retriever = build_in_memory_retriever(embedder, corpus())
        .await
        .expect("retriever builds");

    // The case "expects" the future doc — this is intentionally wrong so we
    // can assert that recall is zero (the doc is invisible).
    let cases = vec![RetrievalEvalCase {
        case_id: "leakage-check".into(),
        query: "acme earnings beat later quarter".into(),
        expected_doc_ids: vec!["doc-new".into()],
        as_of: Some("2026-02-15".into()),
        tags_any: Vec::new(),
        fixed_split_id: None,
    }];

    let report = run_retrieval_eval(&retriever, &cases, 5, hash_drift_key())
        .await
        .expect("eval runs");

    let case = &report.cases[0];
    assert!(
        case.recall_at_k.abs() < 1e-12,
        "future doc must not be recalled; recall={}, ranked={:?}",
        case.recall_at_k,
        case.ranked
    );
    assert!(
        !case.ranked.iter().any(|id| id == "doc-new"),
        "doc-new (2026-03-01) leaked into a pre-publication query (as_of=2026-02-15): {:?}",
        case.ranked
    );
}

#[tokio::test]
async fn hash_embedder_report_round_trips_json() {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let retriever = build_in_memory_retriever(embedder, corpus())
        .await
        .expect("retriever builds");

    let cases = vec![RetrievalEvalCase {
        case_id: "rt".into(),
        query: "acme earnings report strong cloud growth".into(),
        expected_doc_ids: vec!["doc-old".into()],
        as_of: None,
        tags_any: Vec::new(),
        fixed_split_id: None,
    }];

    let report = run_retrieval_eval(&retriever, &cases, 3, hash_drift_key())
        .await
        .expect("eval runs");

    let json = serde_json::to_string(&report).expect("serialize");
    let restored: tdw_eval_runner::retrieval_eval::RetrievalEvalReport =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(report, restored);
}

// ── golden-split regression gate (nightly CI) ────────────────────────────────
//
// Runs only when `TDW_GOLDEN_SPLIT_CI=1` is set; self-skips otherwise so the
// standard `cargo test` invocation stays fast and hermetic.
//
// The checked-in baseline lives at
// `crates/tdw-eval-runner/baselines/golden-split-v1.json`.
//
// # Bless step (intentional baseline update)
//
// Never auto-overwrite the baseline.  When a metric change is intentional:
//
//   1. Run this test locally with `TDW_GOLDEN_SPLIT_CI=1` to produce a fresh
//      report:  `cargo test -p tdw-eval-runner --test retrieval_eval -- golden_split_regression_gate`
//   2. Copy the printed JSON over `baselines/golden-split-v1.json`.
//   3. Open a PR with a description of why the baseline changed (embedder
//      upgrade, split extension, intentional quality improvement, etc.).
//
// CI reads the baseline and fails on regression; it never writes back.
#[tokio::test]
async fn golden_split_regression_gate() {
    let Ok(flag) = std::env::var("TDW_GOLDEN_SPLIT_CI") else {
        eprintln!("TDW_GOLDEN_SPLIT_CI unset; skipping golden-split regression gate");
        return;
    };
    if flag != "1" {
        eprintln!("TDW_GOLDEN_SPLIT_CI={flag}; skipping golden-split regression gate");
        return;
    }

    use tdw_eval_runner::baseline_comparator::{RegressionThresholds, compare_to_baseline};
    use tdw_eval_runner::retrieval_eval::RetrievalEvalReport;

    // ── load baseline ────────────────────────────────────────────────────────
    let baseline_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/baselines/golden-split-v1.json"
    );
    let baseline_json =
        std::fs::read_to_string(baseline_path).expect("baseline file must be readable");
    let baseline: RetrievalEvalReport =
        serde_json::from_str(&baseline_json).expect("baseline must be valid JSON");

    // ── run current eval with HashEmbeddingProvider ──────────────────────────
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let retriever = build_in_memory_retriever(embedder, corpus())
        .await
        .expect("retriever builds");

    let cases = vec![
        RetrievalEvalCase {
            case_id: "earnings".into(),
            query: "acme earnings report strong cloud growth".into(),
            expected_doc_ids: vec!["doc-old".into()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("golden-split-v1".into()),
        },
        RetrievalEvalCase {
            case_id: "supply".into(),
            query: "beta supply chain components note".into(),
            expected_doc_ids: vec!["doc-mid".into()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("golden-split-v1".into()),
        },
    ];

    let current = run_retrieval_eval(&retriever, &cases, 3, hash_drift_key())
        .await
        .expect("eval runs");

    // ── emit fresh report for inspection / bless step ────────────────────────
    let report_json = serde_json::to_string_pretty(&current).expect("serialize");
    println!("=== fresh eval report (golden-split-v1) ===\n{report_json}\n===");

    // ── compare against baseline with production defaults ────────────────────
    let thresholds = RegressionThresholds::default();
    let verdict = compare_to_baseline(&baseline, &current, &thresholds);

    println!("=== verdict ===\n{}", verdict.summary);
    for d in &verdict.deltas {
        println!(
            "  {}: baseline={:.4} current={:.4} delta={:+.4} regressed={}",
            d.metric, d.baseline, d.current, d.delta, d.regressed
        );
    }

    assert!(
        !verdict.is_regression,
        "GOLDEN-SPLIT REGRESSION DETECTED — CI gate failed.\n\
         {}\n\n\
         If this is an intentional quality change, bless the baseline:\n\
           1. Inspect the fresh report printed above.\n\
           2. Copy it over crates/tdw-eval-runner/baselines/golden-split-v1.json\n\
           3. Open a PR documenting the intentional change.\n\
         DO NOT auto-overwrite the baseline.",
        verdict.summary
    );
}

// ── env-gated local-model leg ────────────────────────────────────────────────
//
// Compiled only when this crate's `local-model` feature is enabled (which
// activates `tdw-embed-local/model`). Within that feature gate the test
// self-skips at runtime when TDW_LOCAL_MODEL_DIR is unset, following the
// precedent in `crates/tdw-embed-local/tests/local_model.rs`.
//
// To run:
//   $env:TDW_LOCAL_MODEL_DIR = "C:\path\to\model"
//   cargo test -p tdw-eval-runner --features local-model -- local_model_leg

#[cfg(feature = "local-model")]
#[tokio::test(flavor = "multi_thread")]
async fn local_model_leg_scores_fixed_case_set() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(model_dir) = std::env::var("TDW_LOCAL_MODEL_DIR") else {
        eprintln!("TDW_LOCAL_MODEL_DIR unset; skipping local-model retrieval-eval leg");
        return Ok(());
    };

    use tdw_embed::EmbeddingProvider as _;
    use tdw_embed_local::LocalModelEmbeddingProvider;

    let provider = LocalModelEmbeddingProvider::from_dir(&model_dir)?;
    let model_id = provider.model_id().to_string();
    let embedder: Arc<dyn tdw_embed::EmbeddingProvider> = Arc::new(provider);

    let retriever = build_in_memory_retriever(Arc::clone(&embedder), corpus())
        .await
        .expect("retriever builds with local model");

    let cases = vec![RetrievalEvalCase {
        case_id: "earnings-local".into(),
        query: "acme earnings report strong cloud growth".into(),
        expected_doc_ids: vec!["doc-old".into()],
        as_of: None,
        tags_any: Vec::new(),
        fixed_split_id: Some("split-b11-local".into()),
    }];

    let drift_key = DriftKey {
        embedder_model: model_id,
        rules_version: None,
        infer_version: None,
    };

    let report = run_retrieval_eval(&retriever, &cases, 3, drift_key.clone())
        .await
        .expect("local-model eval runs");

    assert_eq!(report.k, 3);
    assert_eq!(report.drift_key, drift_key);
    assert!(
        report.mrr > 0.0,
        "local model should retrieve the gold doc; mrr={}",
        report.mrr
    );

    Ok(())
}
