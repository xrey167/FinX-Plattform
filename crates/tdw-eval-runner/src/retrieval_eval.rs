//! Retrieval-quality evaluation harness for `tdw-eval-runner` (knowledge-system B11).
//!
//! Runs fixed [`RetrievalEvalCase`] sets through a [`Retriever`] and scores the
//! ranked hits with standard IR metrics — recall\@k, MRR, nDCG\@k — using only
//! in-memory engines and the deterministic hash embedder so the always-run path
//! is fully offline and CI-safe.
//!
//! ## What runs in CI vs what is env-gated
//!
//! - **Always runs**: all unit tests (pure metric functions, serde round-trip,
//!   leakage regression) using the `HashEmbeddingProvider` + in-memory engines.
//! - **Env-gated (`TDW_LOCAL_MODEL_DIR`)**: the `local_model` leg in the
//!   integration test suite — it self-skips (prints a note and returns `Ok`)
//!   when the env var is unset, following the precedent in
//!   `crates/tdw-embed-local/tests/local_model.rs`.
//!
//! ## Drift tracking
//!
//! Every [`RetrievalEvalReport`] is stamped with a [`DriftKey`] —
//! `(embedder_model, rules_version, infer_version)` — drawn from the same
//! [`KnowledgeVersions`] triple that `KnowledgeRuntime` stamps onto MCP search
//! responses (knowledge-system B8). Callers build the key from the live runtime;
//! successive reports can be diff-ed to attribute metric changes to specific
//! component version bumps.
//!
//! ## Leakage safety
//!
//! A [`RetrievalEvalCase`] carries an `as_of` date. That date is fed verbatim
//! into the [`Retriever`]'s own temporal filter, which makes documents dated
//! *after* `as_of` (and undated documents) structurally invisible — leakage
//! safety is a retriever property, never a corpus snapshotting policy.

// Rank indices and case counts are small non-negative integers, well within
// f64's 2^52 exact-integer range.
#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::Result;
use tdw_embed::EmbeddingProvider;
use tdw_knowledge::{KnowledgeDocument, collection_name};
use tdw_retrieve::{KnowledgeQuery, QueryFilter, Retriever};
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;

/// One evaluation case: a fixed query with a gold document set and an optional
/// temporal point-of-view.
///
/// Cases are **checked-in fixtures** identified by `fixed_split_id` — they are
/// never generated at runtime. A stable `fixed_split_id` shared across compared
/// runs guarantees that metric deltas are split-stable (not confounded by a
/// different case mix).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalEvalCase {
    /// Stable case identifier — must be unique within a case set.
    pub case_id: String,
    /// The query text.
    pub query: String,
    /// Relevant document ids (the gold set). Order is irrelevant.
    pub expected_doc_ids: Vec<String>,
    /// Temporal point of view (`YYYY-MM-DD`); `None` = no temporal filter.
    ///
    /// When set, documents dated *after* this date and undated documents are
    /// hidden by the retriever's temporal filter — leakage is impossible.
    #[serde(default)]
    pub as_of: Option<String>,
    /// Optional tag filter, subsumption-expanded by the retriever when a tag
    /// engine is attached.
    #[serde(default)]
    pub tags_any: Vec<String>,
    /// The fixed split this case belongs to. Shared across compared runs so
    /// metric deltas are split-stable.
    #[serde(default)]
    pub fixed_split_id: Option<String>,
}

/// The version triple a report is attributed to.
///
/// Build this from [`tdw_knowledge::runtime::KnowledgeVersions`] (the same
/// triple stamped onto MCP search responses, knowledge-system B8). A metric
/// change between two reports with *different* `DriftKey`s is a version effect,
/// not a regression.
///
/// No hidden global state: callers inject this; nothing here reads a clock or
/// a global.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftKey {
    /// The embedder's `model_id` string (e.g. `"local-hash-8"`).
    pub embedder_model: String,
    /// Tag-rule set version, when auto-tagging is wired.
    #[serde(default)]
    pub rules_version: Option<u64>,
    /// Inference rule-set version, when `tdw-infer` is wired.
    #[serde(default)]
    pub infer_version: Option<u64>,
}

/// Per-case scores at cutoff `k`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseScore {
    /// The case identifier.
    pub case_id: String,
    /// Fraction of the gold set retrieved in the top `k` (0.0–1.0).
    pub recall_at_k: f64,
    /// Reciprocal rank of the *first* relevant hit (0.0 if none in top `k`).
    pub reciprocal_rank: f64,
    /// Normalized DCG at `k` under binary relevance (0.0–1.0).
    pub ndcg_at_k: f64,
    /// The actual ranked document ids returned (for inspection and regression
    /// diffing).
    pub ranked: Vec<String>,
}

/// Aggregate means over all cases plus the per-case breakdown and drift key.
///
/// This type is `Serialize + Deserialize` so successive runs can be persisted
/// and diff-ed by the caller. The runner itself has no storage — persistence
/// is the caller's responsibility (no hidden global state).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalEvalReport {
    /// The cutoff at which all metrics were computed.
    pub k: usize,
    /// Mean recall\@k over all scored cases.
    pub mean_recall_at_k: f64,
    /// Mean reciprocal rank (MRR) over all scored cases.
    pub mrr: f64,
    /// Mean nDCG\@k over all scored cases.
    pub mean_ndcg_at_k: f64,
    /// Per-case scores in input order.
    pub cases: Vec<CaseScore>,
    /// The version triple this report is attributed to.
    pub drift_key: DriftKey,
}

// ── Pure metric functions ────────────────────────────────────────────────────

/// Recall\@k: |relevant ∩ top-k| / |relevant|.
///
/// An empty gold set scores `1.0` (nothing to miss), so it neither rewards
/// nor penalises the run.
#[must_use]
pub fn recall_at_k(ranked: &[String], expected: &BTreeSet<String>, k: usize) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let hits = ranked
        .iter()
        .take(k)
        .filter(|id| expected.contains(*id))
        .count();
    hits as f64 / expected.len() as f64
}

/// Reciprocal rank of the first relevant hit within `k` (1-based).
///
/// Returns `0.0` when no relevant document appears in the top `k`.
#[must_use]
pub fn reciprocal_rank(ranked: &[String], expected: &BTreeSet<String>, k: usize) -> f64 {
    ranked
        .iter()
        .take(k)
        .position(|id| expected.contains(id))
        .map_or(0.0, |pos| 1.0 / (pos as f64 + 1.0))
}

/// nDCG\@k under binary relevance.
///
/// Each relevant hit at 1-based rank `r` contributes `1 / log₂(r + 1)` to
/// the DCG (implemented as `1 / log₂(i + 2)` where `i` is the 0-based
/// position — the two forms are identical). The ideal DCG is the DCG of a
/// perfect ranking that places `min(|relevant|, k)` relevant documents at
/// ranks 1 … min(|relevant|, k). Returns `0.0` when the gold set is empty
/// (ideal DCG is zero; no division-by-zero).
#[must_use]
pub fn ndcg_at_k(ranked: &[String], expected: &BTreeSet<String>, k: usize) -> f64 {
    let ideal_hits = expected.len().min(k);
    if ideal_hits == 0 {
        return 0.0;
    }
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .filter(|(_, id)| expected.contains(*id))
        .map(|(i, _)| 1.0 / ((i as f64 + 2.0).log2()))
        .sum();
    let ideal_dcg: f64 = (0..ideal_hits)
        .map(|i| 1.0 / ((i as f64 + 2.0).log2()))
        .sum();
    dcg / ideal_dcg
}

/// Score one already-ranked case.
#[must_use]
pub fn score_case(case_id: &str, ranked: Vec<String>, expected: &[String], k: usize) -> CaseScore {
    let gold: BTreeSet<String> = expected.iter().cloned().collect();
    CaseScore {
        case_id: case_id.to_string(),
        recall_at_k: recall_at_k(&ranked, &gold, k),
        reciprocal_rank: reciprocal_rank(&ranked, &gold, k),
        ndcg_at_k: ndcg_at_k(&ranked, &gold, k),
        ranked,
    }
}

// ── Retriever construction helpers ──────────────────────────────────────────

/// Build an in-memory vector+lexical [`Retriever`] seeded with `documents`
/// using `embedder`, for deterministic offline evaluation.
///
/// Documents are embedded and inserted into an `InMemoryVectorEngine` +
/// `InMemoryLexicalEngine`. The collection and index names are derived from
/// the embedder's `model_id`.
///
/// # Errors
///
/// Returns a [`tdw_core::Error`] if embedding or upserting a document fails.
pub async fn build_in_memory_retriever(
    embedder: Arc<dyn EmbeddingProvider>,
    documents: Vec<KnowledgeDocument>,
) -> Result<Retriever> {
    use tdw_core::{LexicalDoc, LexicalEngine, VectorEngine, VectorPoint};
    use tdw_knowledge::document_payload;

    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let collection = collection_name(embedder.model_id());
    let index = format!("{collection}_text");

    let bodies: Vec<String> = documents.iter().map(|d| d.body.clone()).collect();
    let embeddings = embedder
        .embed_batch(&bodies)
        .await
        .map_err(|e| tdw_core::Error::Provider(e.to_string()))?;

    let mut points = Vec::with_capacity(documents.len());
    let mut lex_docs = Vec::with_capacity(documents.len());
    for (doc, emb) in documents.iter().zip(&embeddings) {
        let payload = document_payload(doc);
        points.push(VectorPoint {
            id: doc.id.clone(),
            vector: emb.vector.clone(),
            payload: payload.clone(),
        });
        lex_docs.push(LexicalDoc {
            id: doc.id.clone(),
            body: doc.body.clone(),
            fields: payload,
        });
    }

    vectors
        .upsert(&collection, points)
        .await
        .map_err(|e| tdw_core::Error::Storage(e.to_string()))?;
    lexical
        .index(&index, lex_docs)
        .await
        .map_err(|e| tdw_core::Error::Storage(e.to_string()))?;

    Ok(Retriever::new(embedder, vectors, collection).with_lexical(lexical, index))
}

// ── Eval runner ─────────────────────────────────────────────────────────────

/// Run a fixed [`RetrievalEvalCase`] set through `retriever` at cutoff `k`,
/// returning the aggregate report tagged with `drift_key`.
///
/// Each case's query runs with `top_k = k`, its `as_of`, and its tag filter.
/// Errors are loud — evaluation never silently drops a case.
///
/// # Errors
///
/// Returns the first failing query's error (malformed case or engine fault).
pub async fn run_retrieval_eval(
    retriever: &Retriever,
    cases: &[RetrievalEvalCase],
    k: usize,
    drift_key: DriftKey,
) -> Result<RetrievalEvalReport> {
    let mut scored = Vec::with_capacity(cases.len());
    for case in cases {
        let query = KnowledgeQuery::try_new(
            &case.query,
            k,
            QueryFilter {
                tags_any: case.tags_any.clone(),
                entity_kinds: None,
                plane: None,
                as_of: case.as_of.clone(),
            },
            None,
        )?;
        let hits = retriever.search(&query).await?;
        let ranked: Vec<String> = hits.into_iter().map(|h| h.id).collect();
        scored.push(score_case(&case.case_id, ranked, &case.expected_doc_ids, k));
    }
    let n = scored.len().max(1) as f64;
    let mean = |f: fn(&CaseScore) -> f64| scored.iter().map(f).sum::<f64>() / n;
    Ok(RetrievalEvalReport {
        k,
        mean_recall_at_k: mean(|s| s.recall_at_k),
        mrr: mean(|s| s.reciprocal_rank),
        mean_ndcg_at_k: mean(|s| s.ndcg_at_k),
        cases: scored,
        drift_key,
    })
}

// ── Unit tests (always run in CI) ────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }
    fn gold(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    // ── recall@k ────────────────────────────────────────────────────────────

    #[test]
    fn recall_perfect_at_full_k() {
        assert!((recall_at_k(&ids(&["a", "b"]), &gold(&["a", "b"]), 2) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn recall_counts_only_top_k() {
        // "d" is relevant but at rank 4; within top-2 only "a" is retrieved.
        let ranked = ids(&["a", "b", "c", "d"]);
        let expected = gold(&["a", "d"]);
        assert!((recall_at_k(&ranked, &expected, 4) - 1.0).abs() < 1e-12);
        assert!((recall_at_k(&ranked, &expected, 2) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn recall_empty_gold_is_vacuous_one() {
        assert!((recall_at_k(&ids(&["a", "b"]), &gold(&[]), 2) - 1.0).abs() < 1e-12);
    }

    // ── MRR ─────────────────────────────────────────────────────────────────

    #[test]
    fn rr_first_hit_at_rank_one() {
        assert!((reciprocal_rank(&ids(&["a", "b"]), &gold(&["a"]), 3) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rr_first_hit_at_rank_two() {
        assert!((reciprocal_rank(&ids(&["x", "a", "b"]), &gold(&["a"]), 3) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn rr_no_hit_within_k_is_zero() {
        assert!(reciprocal_rank(&ids(&["x", "y"]), &gold(&["a"]), 2).abs() < 1e-12);
    }

    #[test]
    fn rr_hit_beyond_k_is_zero() {
        assert!(reciprocal_rank(&ids(&["x", "y", "a"]), &gold(&["a"]), 2).abs() < 1e-12);
    }

    // ── nDCG@k ───────────────────────────────────────────────────────────────

    #[test]
    fn ndcg_perfect_ranking_is_one() {
        assert!((ndcg_at_k(&ids(&["a", "b", "c"]), &gold(&["a", "b"]), 3) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ndcg_imperfect_hand_computed() {
        // Rank-1 "a" (1/log2(2)=1.0), rank-3 "b" (1/log2(4)=0.5) → dcg=1.5.
        // Ideal: 1/log2(2)+1/log2(3) = 1.0+0.630… → ndcg = 1.5 / 1.630…
        let expected = gold(&["a", "b"]);
        let got = ndcg_at_k(&ids(&["a", "c", "b"]), &expected, 3);
        let want = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
        assert!((got - want).abs() < 1e-12, "got {got}, want {want}");
    }

    #[test]
    fn ndcg_empty_gold_is_zero_not_nan() {
        let v = ndcg_at_k(&ids(&["a"]), &gold(&[]), 3);
        assert!(v.abs() < 1e-12, "expected 0.0, got {v}");
    }

    #[test]
    fn ndcg_caps_ideal_at_k() {
        // Three relevant but k=2: ideal packs 2 → perfect recall → nDCG=1.
        assert!((ndcg_at_k(&ids(&["a", "b"]), &gold(&["a", "b", "c"]), 2) - 1.0).abs() < 1e-12);
    }

    // ── score_case ───────────────────────────────────────────────────────────

    #[test]
    fn score_case_aggregates_all_three_metrics() {
        let cs = score_case("c1", ids(&["a", "x", "b"]), &["a".into(), "b".into()], 3);
        assert_eq!(cs.case_id, "c1");
        assert!((cs.recall_at_k - 1.0).abs() < 1e-12);
        assert!((cs.reciprocal_rank - 1.0).abs() < 1e-12);
        // nDCG: a at rank1 (1/log2(2)=1) b at rank3 (1/log2(4)=0.5) → same as imperfect test.
        let want = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
        assert!((cs.ndcg_at_k - want).abs() < 1e-12, "{}", cs.ndcg_at_k);
    }

    // ── serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn report_round_trips_through_json() {
        let report = RetrievalEvalReport {
            k: 5,
            mean_recall_at_k: 0.75,
            mrr: 0.833,
            mean_ndcg_at_k: 0.9,
            cases: vec![CaseScore {
                case_id: "q1".into(),
                recall_at_k: 0.75,
                reciprocal_rank: 0.833,
                ndcg_at_k: 0.9,
                ranked: ids(&["doc-a", "doc-b"]),
            }],
            drift_key: DriftKey {
                embedder_model: "local-hash-8".into(),
                rules_version: Some(3),
                infer_version: Some(2),
            },
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let restored: RetrievalEvalReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, restored);
    }
}
