//! Retrieval-quality evaluation harness (knowledge-system B11).
//!
//! Runs a fixed [`RetrievalEvalCase`] set through a [`Retriever`] and scores
//! the ranked hits with the standard IR metrics — recall@k, MRR, nDCG@k —
//! deterministically on the hash embedder + in-memory engines.
//!
//! Leakage safety is STRUCTURAL, not procedural: a case carries an `as_of`,
//! and the retriever's own temporal filter hides every later-dated (and
//! undated) document, so the corpus never needs snapshotting per case. Drift
//! is tracked by tagging every report with the [`DriftKey`] —
//! `(rules_version, infer_version, embedder_model)` — the same triple the MCP
//! search response stamps, so a metric change attributes to the exact
//! component versions that produced it.

// Every `usize as f64` here is a case count or a 1-based rank index — far
// below f64's 2^52 exact-integer ceiling (the same posture as `rrf.rs`).
#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use tdw_core::Result;

use crate::{KnowledgeQuery, QueryFilter, Retriever};

/// One evaluation case: a query and the document ids that SHOULD rank for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalEvalCase {
    /// Stable case identifier.
    pub case_id: String,
    /// The query text.
    pub query: String,
    /// Relevant document ids (the gold set). Order does not matter.
    pub expected_doc_ids: Vec<String>,
    /// Temporal point of view (`YYYY-MM-DD`); `None` = no temporal filter.
    #[serde(default)]
    pub as_of: Option<String>,
    /// Optional tag filter applied to every channel (subsumption-expanded).
    #[serde(default)]
    pub tags_any: Vec<String>,
    /// The fixed split this case belongs to — shared across compared runs so
    /// metric deltas are split-stable (`EvalFacets::fixed_split_id`).
    #[serde(default)]
    pub fixed_split_id: Option<String>,
}

/// The version triple a report is attributed to. A metric change keyed by a
/// different `DriftKey` is a version effect, not a regression.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftKey {
    /// The embedder's `model_id`.
    pub embedder_model: String,
    /// The tag-rule set version, when wired.
    #[serde(default)]
    pub rules_version: Option<u64>,
    /// The inference rule-set version, when wired.
    #[serde(default)]
    pub infer_version: Option<u64>,
}

/// Per-case scores at cutoff `k`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseScore {
    pub case_id: String,
    /// Fraction of the gold set retrieved within the top `k`.
    pub recall_at_k: f64,
    /// Reciprocal rank of the FIRST relevant hit (0.0 if none in `k`).
    pub reciprocal_rank: f64,
    /// Normalized DCG at `k` (binary relevance).
    pub ndcg_at_k: f64,
    /// The ranked hit ids actually returned (for inspection).
    pub ranked: Vec<String>,
}

/// Aggregate (mean over cases) plus the per-case breakdown and drift key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievalEvalReport {
    /// The cutoff the metrics were computed at.
    pub k: usize,
    /// Mean recall@k over all scored cases.
    pub mean_recall_at_k: f64,
    /// Mean reciprocal rank (MRR) over all scored cases.
    pub mrr: f64,
    /// Mean nDCG@k over all scored cases.
    pub mean_ndcg_at_k: f64,
    /// Per-case scores, in input order.
    pub cases: Vec<CaseScore>,
    /// The version triple this report attributes to.
    pub drift_key: DriftKey,
}

/// Recall@k: |relevant ∩ top-k| / |relevant|. A case with no gold set scores
/// 1.0 (nothing to miss) so it neither rewards nor penalizes.
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

/// Reciprocal rank of the first relevant hit within `k` (1-based ranks);
/// 0.0 when none is relevant.
#[must_use]
pub fn reciprocal_rank(ranked: &[String], expected: &BTreeSet<String>, k: usize) -> f64 {
    ranked
        .iter()
        .take(k)
        .position(|id| expected.contains(id))
        .map_or(0.0, |index| 1.0 / (index as f64 + 1.0))
}

/// nDCG@k under binary relevance.
///
/// DCG uses `1 / log2(rank + 1)` (1-based rank); the ideal DCG packs
/// `min(|relevant|, k)` hits at the top. Returns 0.0 for an empty ideal (no
/// gold set) so it cannot divide by zero.
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
        .map(|(index, _)| 1.0 / ((index as f64 + 2.0).log2()))
        .sum();
    let ideal_dcg: f64 = (0..ideal_hits)
        .map(|index| 1.0 / ((index as f64 + 2.0).log2()))
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

/// Run a fixed case set through `retriever` at cutoff `k`, returning the
/// aggregate report tagged with `drift_key`.
///
/// Each case's query runs with `top_k = k`, its `as_of`, and its tag filter;
/// the retriever's temporal filter makes the evaluation leakage-safe by
/// construction (later/undated documents are invisible).
///
/// # Errors
///
/// Returns the first failing query's error (a malformed case or an engine
/// fault) — evaluation never silently drops a case.
pub async fn run_eval(
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
                provenance_classes: None,
            },
            None,
        )?;
        let hits = retriever.search(&query).await?;
        let ranked: Vec<String> = hits.into_iter().map(|hit| hit.id).collect();
        scored.push(score_case(&case.case_id, ranked, &case.expected_doc_ids, k));
    }
    let count = scored.len().max(1) as f64;
    let mean = |select: fn(&CaseScore) -> f64| scored.iter().map(select).sum::<f64>() / count;
    Ok(RetrievalEvalReport {
        k,
        mean_recall_at_k: mean(|score| score.recall_at_k),
        mrr: mean(|score| score.reciprocal_rank),
        mean_ndcg_at_k: mean(|score| score.ndcg_at_k),
        cases: scored,
        drift_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn gold(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn recall_counts_only_top_k() {
        let ranked = ids(&["a", "b", "c", "d"]);
        let expected = gold(&["a", "d"]);
        assert!((recall_at_k(&ranked, &expected, 4) - 1.0).abs() < 1e-12);
        // d falls outside the top 2 → only a is recalled.
        assert!((recall_at_k(&ranked, &expected, 2) - 0.5).abs() < 1e-12);
        // No gold set is a vacuous 1.0.
        assert!((recall_at_k(&ranked, &gold(&[]), 2) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn reciprocal_rank_uses_first_relevant_one_based() {
        // First relevant at position 2 (1-based) → 1/2.
        assert!((reciprocal_rank(&ids(&["x", "a", "b"]), &gold(&["a"]), 3) - 0.5).abs() < 1e-12);
        // Relevant at rank 1 → 1.0.
        assert!((reciprocal_rank(&ids(&["a", "x"]), &gold(&["a"]), 3) - 1.0).abs() < 1e-12);
        // None relevant within k → 0.0.
        assert!(reciprocal_rank(&ids(&["x", "y"]), &gold(&["a"]), 2).abs() < 1e-12);
        // Relevant exists but beyond k → 0.0.
        assert!(reciprocal_rank(&ids(&["x", "y", "a"]), &gold(&["a"]), 2).abs() < 1e-12);
    }

    #[test]
    fn ndcg_is_one_for_ideal_ranking_and_less_otherwise() {
        let expected = gold(&["a", "b"]);
        // Perfect: both relevant docs at the top.
        assert!((ndcg_at_k(&ids(&["a", "b", "c"]), &expected, 3) - 1.0).abs() < 1e-12);
        // Swapped with an irrelevant doc in between — strictly worse.
        let imperfect = ndcg_at_k(&ids(&["a", "c", "b"]), &expected, 3);
        assert!(imperfect < 1.0 && imperfect > 0.0, "{imperfect}");
        // Hand-computed: rank1 a (gain 1/log2(2)=1), rank3 b (gain 1/log2(4)=0.5)
        // → dcg 1.5; ideal dcg = 1/log2(2) + 1/log2(3) = 1 + 0.6309 = 1.6309.
        let expected_ndcg = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
        assert!((imperfect - expected_ndcg).abs() < 1e-12, "{imperfect}");
        // No gold set → 0.0 (no ideal), never NaN.
        assert!(ndcg_at_k(&ids(&["a"]), &gold(&[]), 3).abs() < 1e-12);
    }

    #[test]
    fn ndcg_caps_ideal_at_k() {
        // Three relevant docs but k=2: ideal packs only 2.
        let expected = gold(&["a", "b", "c"]);
        assert!((ndcg_at_k(&ids(&["a", "b"]), &expected, 2) - 1.0).abs() < 1e-12);
    }
}
