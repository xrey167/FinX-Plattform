//! Published knowledge-retrieval benchmark suite (knowledge-system K-M5).
//!
//! A LongMemEval-style **task set over the existing B11 retrieval-eval harness**
//! ([`crate::retrieval_eval`]). It does NOT introduce a parallel eval engine:
//! quality scoring reuses [`run_retrieval_eval`] verbatim. This module only adds
//! a checked-in benchmark corpus + task set, a latency-percentile pass, and a
//! drift-stamped, machine-readable [`BenchmarkReport`].
//!
//! ## What is measured (honest scope)
//!
//! - **Recall\@k, MRR, nDCG\@k** over a fixed, checked-in task set — fully
//!   deterministic (hash embedder + in-memory engines + fixed split), so the
//!   quality numbers are byte-identical across runs and reproducible in CI.
//! - **Retrieval latency percentiles** (p50 / p90 / p99, microseconds) measured
//!   in-process over the same task set. Latency is **environment-dependent** and
//!   therefore informational — it is NOT asserted stable across runs. The
//!   deterministic quality block is what the CI reproducibility gate checks.
//!
//! ## What is NOT measured
//!
//! - No head-to-head competitor runs. This suite publishes the platform's own
//!   numbers and methodology; a competitor comparison would require the same
//!   task set through the competitor's retriever under identical embedder and
//!   corpus conditions (see `docs/benchmarks.md`). We do not fabricate
//!   competitor numbers.
//! - No live-corpus / production-vector-store quality. The corpus is the fixed
//!   checked-in set, matching the B11 harness contract.
//! - No end-to-end LLM answer quality — this measures the *retrieval* layer that
//!   feeds the agent, not generation.
//!
//! ## Determinism
//!
//! - Embeddings: `HashEmbeddingProvider` (no RNG, no network).
//! - Clock: callers inject a monotonic tick via a closure. Tests pass a
//!   deterministic synthetic clock so latency is reproducible under test; the
//!   `xtask` runner passes a real `Instant`-backed clock for real wall numbers.
//!   No function here reads the wall clock directly.

// Percentile indices and case counts are small non-negative integers, well
// within f64's 2^52 exact-integer range; the percentile rank is `ceil`ed from a
// probability in [0, 1] times a small count, so it is always a small
// non-negative integer (no truncation or sign loss in practice).
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::Result;
use tdw_embed::EmbeddingProvider;
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::KnowledgeDocument;
use tdw_retrieve::{KnowledgeQuery, QueryFilter, Retriever};

use crate::retrieval_eval::{
    DriftKey, RetrievalEvalCase, RetrievalEvalReport, build_in_memory_retriever, run_retrieval_eval,
};

/// The stable identifier of the published benchmark task set. Bumped only on an
/// intentional, documented task-set change (a new split is a new id).
pub const BENCHMARK_SPLIT_ID: &str = "knowledge-bench-v1";

/// Retrieval cutoff `k` used for every benchmark metric.
pub const BENCHMARK_K: usize = 5;

// ── Latency block ────────────────────────────────────────────────────────────

/// Retrieval-latency percentiles over the task set, in **microseconds**.
///
/// Informational only: latency is environment-dependent and is NOT part of the
/// reproducibility gate. The quality block ([`QualityMetrics`]) is the
/// deterministic, reproducible part of the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencyPercentiles {
    /// Number of per-query latency samples (one per task).
    pub samples: usize,
    /// Median (p50) retrieval latency, microseconds.
    pub p50_micros: u64,
    /// p90 retrieval latency, microseconds.
    pub p90_micros: u64,
    /// p99 retrieval latency, microseconds.
    pub p99_micros: u64,
    /// Maximum observed retrieval latency, microseconds.
    pub max_micros: u64,
}

impl LatencyPercentiles {
    /// Compute nearest-rank percentiles from raw per-query micro-second samples.
    ///
    /// Nearest-rank (not interpolated) so the percentile is always an observed
    /// sample value. An empty input yields an all-zero block.
    #[must_use]
    pub fn from_samples(mut micros: Vec<u64>) -> Self {
        if micros.is_empty() {
            return Self {
                samples: 0,
                p50_micros: 0,
                p90_micros: 0,
                p99_micros: 0,
                max_micros: 0,
            };
        }
        micros.sort_unstable();
        let n = micros.len();
        // Nearest-rank: index = ceil(p * n) - 1, clamped to [0, n-1].
        let at = |p: f64| -> u64 {
            let rank = (p * n as f64).ceil() as usize;
            let idx = rank.saturating_sub(1).min(n - 1);
            micros[idx]
        };
        Self {
            samples: n,
            p50_micros: at(0.50),
            p90_micros: at(0.90),
            p99_micros: at(0.99),
            max_micros: micros[n - 1],
        }
    }
}

// ── Quality block ────────────────────────────────────────────────────────────

/// Aggregate retrieval-quality metrics over the task set.
///
/// These are the deterministic, reproducible numbers. They are lifted directly
/// from the B11 [`RetrievalEvalReport`] produced by [`run_retrieval_eval`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Retrieval cutoff the metrics were computed at.
    pub k: usize,
    /// Number of scored tasks.
    pub tasks: usize,
    /// Mean recall\@k over the task set.
    pub mean_recall_at_k: f64,
    /// Mean reciprocal rank (MRR) over the task set.
    pub mrr: f64,
    /// Mean nDCG\@k over the task set.
    pub mean_ndcg_at_k: f64,
}

impl QualityMetrics {
    /// Project the deterministic quality block out of a B11 report.
    ///
    /// `const` is valid here: `Vec::len` is a stable `const fn` since Rust 1.87
    /// (`const_vec_string_slice`), and the toolchain floor is 1.95. `nursery`'s
    /// `missing_const_for_fn` requires it.
    #[must_use]
    pub const fn from_report(report: &RetrievalEvalReport) -> Self {
        Self {
            k: report.k,
            tasks: report.cases.len(),
            mean_recall_at_k: report.mean_recall_at_k,
            mrr: report.mrr,
            mean_ndcg_at_k: report.mean_ndcg_at_k,
        }
    }
}

// ── Report ───────────────────────────────────────────────────────────────────

/// The full, drift-stamped benchmark report (the published artifact).
///
/// Serialized to `docs/benchmarks-knowledge.json` by `xtask bench-knowledge`.
/// Two runs with the same code and embedder produce **identical** `split_id`,
/// `quality`, and `drift_key` blocks; only `latency` may vary run-to-run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkReport {
    /// Stable task-set identifier ([`BENCHMARK_SPLIT_ID`]).
    pub split_id: String,
    /// The deterministic, reproducible quality block.
    pub quality: QualityMetrics,
    /// The informational, environment-dependent latency block.
    pub latency: LatencyPercentiles,
    /// The version triple (embedder / rules / infer) this report is attributed
    /// to — the reproducibility/drift key.
    pub drift_key: DriftKey,
}

// ── Built-in LongMemEval-style task set ──────────────────────────────────────

/// Build an entity for the benchmark corpus.
fn entity(id: &str) -> Entity {
    Entity {
        entity_id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: Vec::new(),
    }
}

/// Build one benchmark corpus document.
fn doc(id: &str, body: &str, ent: &str, tag: &str, as_of: &str) -> KnowledgeDocument {
    KnowledgeDocument {
        id: id.to_string(),
        body: body.to_string(),
        entity: entity(ent),
        tags: vec![tag.to_string()],
        source: None,
        plane: Some("platform".to_string()),
        as_of: Some(as_of.to_string()),
        mentions: Vec::new(),
        author: None,
    }
}

/// The fixed, checked-in benchmark corpus.
///
/// A LongMemEval-style "memory" of dated financial knowledge facts spread across
/// several entities and dates ("sessions"), so the task set can exercise
/// single-fact recall, distractor robustness, and temporal (point-in-time)
/// retrieval. Every document is dated so the temporal-leakage cases are
/// structurally enforced by the retriever's `as_of` filter.
#[must_use]
pub fn benchmark_corpus() -> Vec<KnowledgeDocument> {
    vec![
        doc(
            "acme-earnings-q1",
            "acme reported strong first quarter cloud revenue growth and raised guidance",
            "instrument:acme",
            "sector:tech",
            "2026-01-15",
        ),
        doc(
            "acme-buyback",
            "acme board authorized a new share buyback program of two billion dollars",
            "instrument:acme",
            "sector:tech",
            "2026-02-10",
        ),
        doc(
            "acme-earnings-q2",
            "acme second quarter results beat estimates on datacenter demand",
            "instrument:acme",
            "sector:tech",
            "2026-04-20",
        ),
        doc(
            "beta-supply",
            "beta disclosed a supply chain disruption affecting component shipments",
            "instrument:beta",
            "sector:industrial",
            "2026-01-30",
        ),
        doc(
            "beta-recovery",
            "beta said component shipments recovered and margins normalized",
            "instrument:beta",
            "sector:industrial",
            "2026-03-25",
        ),
        doc(
            "gamma-dividend",
            "gamma increased its quarterly dividend and reaffirmed annual outlook",
            "instrument:gamma",
            "sector:financials",
            "2026-02-05",
        ),
        doc(
            "gamma-acquisition",
            "gamma agreed to acquire a regional payments network for eight hundred million",
            "instrument:gamma",
            "sector:financials",
            "2026-05-12",
        ),
        doc(
            "delta-downgrade",
            "delta was downgraded by analysts citing weak consumer demand trends",
            "instrument:delta",
            "sector:consumer",
            "2026-01-22",
        ),
        doc(
            "delta-restructuring",
            "delta announced a restructuring plan to cut costs across retail operations",
            "instrument:delta",
            "sector:consumer",
            "2026-04-08",
        ),
        doc(
            "epsilon-fda",
            "epsilon received regulatory approval for its lead respiratory therapy",
            "instrument:epsilon",
            "sector:healthcare",
            "2026-03-01",
        ),
    ]
}

/// One benchmark task.
fn task(case_id: &str, query: &str, expected: &[&str], as_of: Option<&str>) -> RetrievalEvalCase {
    RetrievalEvalCase {
        case_id: case_id.to_string(),
        query: query.to_string(),
        expected_doc_ids: expected.iter().map(|s| (*s).to_string()).collect(),
        as_of: as_of.map(ToString::to_string),
        tags_any: Vec::new(),
        fixed_split_id: Some(BENCHMARK_SPLIT_ID.to_string()),
    }
}

/// The fixed, checked-in benchmark task set (LongMemEval-style).
///
/// Categories exercised:
/// - **single-fact recall** — direct retrieval of one dated fact;
/// - **distractor robustness** — same-entity facts compete for the top slot;
/// - **temporal / point-in-time** — `as_of` hides later facts, so only the
///   pre-publication fact is retrievable.
#[must_use]
pub fn benchmark_tasks() -> Vec<RetrievalEvalCase> {
    vec![
        // single-fact recall
        task(
            "acme-q1-recall",
            "acme strong first quarter cloud revenue growth raised guidance",
            &["acme-earnings-q1"],
            None,
        ),
        task(
            "acme-buyback-recall",
            "acme board authorized new share buyback program two billion",
            &["acme-buyback"],
            None,
        ),
        task(
            "beta-supply-recall",
            "beta supply chain disruption affecting component shipments",
            &["beta-supply"],
            None,
        ),
        task(
            "gamma-dividend-recall",
            "gamma increased quarterly dividend reaffirmed annual outlook",
            &["gamma-dividend"],
            None,
        ),
        task(
            "delta-downgrade-recall",
            "delta downgraded by analysts weak consumer demand trends",
            &["delta-downgrade"],
            None,
        ),
        task(
            "epsilon-fda-recall",
            "epsilon regulatory approval lead respiratory therapy",
            &["epsilon-fda"],
            None,
        ),
        // distractor robustness — gamma acquisition vs gamma dividend
        task(
            "gamma-acq-distractor",
            "gamma agreed to acquire regional payments network eight hundred million",
            &["gamma-acquisition"],
            None,
        ),
        // distractor robustness — beta recovery vs beta supply
        task(
            "beta-recovery-distractor",
            "beta component shipments recovered margins normalized",
            &["beta-recovery"],
            None,
        ),
        // temporal / point-in-time — as_of before acme-earnings-q2 (2026-04-20)
        // must retrieve the Q1 fact, never the later Q2 fact. The query shares
        // the Q1 body's salient tokens so the pre-publication fact ranks first.
        task(
            "acme-pit-q1-only",
            "acme first quarter cloud revenue growth raised guidance",
            &["acme-earnings-q1"],
            Some("2026-03-31"),
        ),
        // temporal — as_of before delta-restructuring (2026-04-08) must surface
        // the earlier downgrade, not the later restructuring.
        task(
            "delta-pit-downgrade",
            "delta downgraded analysts weak consumer demand trends",
            &["delta-downgrade"],
            Some("2026-03-31"),
        ),
    ]
}

// ── Clock abstraction (injected, never reads the wall clock here) ────────────

/// A monotonic micro-second clock injected by the caller.
///
/// `tick_micros()` returns a monotonically non-decreasing micro-second counter.
/// Tests pass a deterministic synthetic clock; the `xtask` runner passes an
/// `Instant`-backed clock. This module never calls `Instant::now` itself.
///
/// The bound is `Send + Sync` so the benchmark future stays `Send` (the runner
/// holds `&C` across `await` points while timing each retrieval).
pub trait BenchClock: Send + Sync {
    /// Current micro-second tick (monotonically non-decreasing).
    fn tick_micros(&self) -> u64;
}

/// A deterministic clock that advances by a fixed step on every read.
///
/// Used by tests so the latency block is reproducible. The first reading is
/// `0`, the second `step`, and so on — so a per-query "elapsed" of exactly
/// `step` micro-seconds is observed for every task. Backed by an atomic so the
/// clock is `Sync` (and the benchmark future stays `Send`).
pub struct FixedStepClock {
    step: u64,
    counter: std::sync::atomic::AtomicU64,
}

impl FixedStepClock {
    /// Build a clock that advances `step` micro-seconds per read.
    #[must_use]
    pub const fn new(step: u64) -> Self {
        Self {
            step,
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl BenchClock for FixedStepClock {
    fn tick_micros(&self) -> u64 {
        self.counter
            .fetch_add(self.step, std::sync::atomic::Ordering::Relaxed)
    }
}

// ── Runner ───────────────────────────────────────────────────────────────────

/// Run the published benchmark suite and return the drift-stamped report.
///
/// Quality scoring reuses [`run_retrieval_eval`] (the B11 harness) verbatim;
/// latency is measured by timing each task's retrieval with the injected
/// `clock`. The `quality` and `drift_key` blocks are fully deterministic; only
/// `latency` depends on `clock`.
///
/// # Errors
///
/// Propagates a [`tdw_core::Error`] if the in-memory retriever fails to build or
/// a task query is malformed / faults.
pub async fn run_benchmark<C: BenchClock>(
    embedder: Arc<dyn EmbeddingProvider>,
    clock: &C,
) -> Result<BenchmarkReport> {
    let documents = benchmark_corpus();
    let tasks = benchmark_tasks();
    let drift_key = DriftKey {
        embedder_model: embedder.model_id().to_string(),
        rules_version: None,
        infer_version: None,
    };

    let retriever = build_in_memory_retriever(Arc::clone(&embedder), documents).await?;

    // Deterministic quality via the B11 harness (single source of truth).
    let report = run_retrieval_eval(&retriever, &tasks, BENCHMARK_K, drift_key.clone()).await?;
    let quality = QualityMetrics::from_report(&report);

    // Informational latency: time each task's retrieval with the injected clock.
    let latency = measure_latency(&retriever, &tasks, clock).await?;

    Ok(BenchmarkReport {
        split_id: BENCHMARK_SPLIT_ID.to_string(),
        quality,
        latency,
        drift_key,
    })
}

/// Time each task's retrieval call and reduce to percentile micro-seconds.
async fn measure_latency<C: BenchClock>(
    retriever: &Retriever,
    tasks: &[RetrievalEvalCase],
    clock: &C,
) -> Result<LatencyPercentiles> {
    let mut samples = Vec::with_capacity(tasks.len());
    for case in tasks {
        let query = KnowledgeQuery::try_new(
            &case.query,
            BENCHMARK_K,
            QueryFilter {
                tags_any: case.tags_any.clone(),
                entity_kinds: None,
                plane: None,
                as_of: case.as_of.clone(),
                provenance_classes: None,
            },
            None,
        )?;
        let start = clock.tick_micros();
        let _ = retriever.search(&query).await?;
        let end = clock.tick_micros();
        samples.push(end.saturating_sub(start));
    }
    Ok(LatencyPercentiles::from_samples(samples))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_embed_local::HashEmbeddingProvider;

    #[test]
    fn task_set_and_corpus_are_self_consistent() {
        let corpus = benchmark_corpus();
        let tasks = benchmark_tasks();
        assert!(!corpus.is_empty(), "corpus must be non-empty");
        assert!(!tasks.is_empty(), "task set must be non-empty");

        let ids: std::collections::BTreeSet<&str> = corpus.iter().map(|d| d.id.as_str()).collect();
        // Every gold doc id referenced by a task must exist in the corpus.
        for task in &tasks {
            for gold in &task.expected_doc_ids {
                assert!(
                    ids.contains(gold.as_str()),
                    "task {} references unknown gold doc {gold}",
                    task.case_id
                );
            }
            assert_eq!(
                task.fixed_split_id.as_deref(),
                Some(BENCHMARK_SPLIT_ID),
                "task {} must carry the benchmark split id",
                task.case_id
            );
        }
    }

    #[test]
    fn percentiles_nearest_rank_are_observed_values() {
        let p = LatencyPercentiles::from_samples(vec![10, 20, 30, 40, 50]);
        assert_eq!(p.samples, 5);
        assert_eq!(p.p50_micros, 30); // ceil(0.5*5)=3 -> idx 2 -> 30
        assert_eq!(p.p90_micros, 50); // ceil(0.9*5)=5 -> idx 4 -> 50
        assert_eq!(p.p99_micros, 50);
        assert_eq!(p.max_micros, 50);
    }

    #[test]
    fn percentiles_empty_is_all_zero() {
        let p = LatencyPercentiles::from_samples(vec![]);
        assert_eq!(p.samples, 0);
        assert_eq!(p.p50_micros, 0);
        assert_eq!(p.max_micros, 0);
    }

    #[test]
    fn fixed_step_clock_advances_deterministically() {
        let clock = FixedStepClock::new(7);
        assert_eq!(clock.tick_micros(), 0);
        assert_eq!(clock.tick_micros(), 7);
        assert_eq!(clock.tick_micros(), 14);
    }

    #[tokio::test]
    async fn benchmark_quality_block_is_deterministic_across_runs() {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let clock = FixedStepClock::new(5);

        let first = run_benchmark(Arc::clone(&embedder) as Arc<dyn EmbeddingProvider>, &clock)
            .await
            .expect("first run");
        let clock2 = FixedStepClock::new(5);
        let second = run_benchmark(Arc::clone(&embedder) as Arc<dyn EmbeddingProvider>, &clock2)
            .await
            .expect("second run");

        // Whole report is identical under the deterministic clock.
        assert_eq!(first, second, "benchmark must be fully deterministic");
        assert_eq!(first.split_id, BENCHMARK_SPLIT_ID);
        assert_eq!(first.quality.k, BENCHMARK_K);
        assert_eq!(first.quality.tasks, benchmark_tasks().len());
    }

    #[tokio::test]
    async fn benchmark_recall_is_high_on_its_own_task_set() {
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let clock = FixedStepClock::new(1);
        let report = run_benchmark(embedder as Arc<dyn EmbeddingProvider>, &clock)
            .await
            .expect("run");
        // The deterministic hash embedder is a content-blind CI stand-in, so
        // the published quality floor is modest by design (see docs/benchmarks.md
        // methodology). Assert the committed floor; a drop below it signals a
        // real retrieval regression rather than embedder noise.
        assert!(
            report.quality.mean_recall_at_k >= 0.5,
            "mean recall@k below the committed floor: {}",
            report.quality.mean_recall_at_k
        );
        assert!(
            report.quality.mrr > 0.0 && report.quality.mrr <= 1.0,
            "mrr out of range: {}",
            report.quality.mrr
        );
    }

    #[tokio::test]
    async fn benchmark_temporal_tasks_do_not_leak_future_facts() {
        // The acme point-in-time task (as_of 2026-03-31) must NOT recall the
        // later Q2 earnings doc (2026-04-20). Run the eval and confirm the
        // future doc never appears in that task's ranked list.
        let embedder = Arc::new(HashEmbeddingProvider::default());
        let documents = benchmark_corpus();
        let tasks = benchmark_tasks();
        let retriever = build_in_memory_retriever(embedder, documents)
            .await
            .expect("retriever builds");
        let drift_key = DriftKey {
            embedder_model: "local-hash-8".to_string(),
            rules_version: None,
            infer_version: None,
        };
        let report = run_retrieval_eval(&retriever, &tasks, BENCHMARK_K, drift_key)
            .await
            .expect("eval runs");
        let pit = report
            .cases
            .iter()
            .find(|c| c.case_id == "acme-pit-q1-only")
            .expect("pit task present");
        assert!(
            !pit.ranked.iter().any(|id| id == "acme-earnings-q2"),
            "future doc acme-earnings-q2 leaked into a point-in-time query: {:?}",
            pit.ranked
        );
    }
}
