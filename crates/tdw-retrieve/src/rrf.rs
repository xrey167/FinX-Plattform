//! Reciprocal Rank Fusion — the pure, standalone-tested heart of hybrid
//! retrieval (knowledge-system B4).
//!
//! `score(d) = Σ_channels 1 / (k + rank_channel(d))` with 1-based ranks and
//! the conventional `k = 60`. RRF consumes only RANKS, never raw scores, so
//! channels with incomparable score scales (cosine similarity, match counts,
//! tag recency) fuse without calibration — and ranking divergence between
//! in-memory and real backends is dampened to what rank order can express.

use std::collections::BTreeMap;

/// The conventional RRF dampening constant.
pub const RRF_K: f64 = 60.0;

/// One channel's ranked candidate list (best first). Entries carry the raw
/// channel score purely for explanation; fusion ignores it.
#[derive(Clone, Debug, PartialEq)]
pub struct RankedEntry {
    /// Candidate document id.
    pub id: String,
    /// The channel's own score, for explanations only.
    pub raw_score: f32,
}

/// A fused candidate: RRF score plus, per contributing channel index, the
/// 1-based rank and raw score it arrived with.
#[derive(Clone, Debug, PartialEq)]
pub struct Fused {
    /// Candidate document id.
    pub id: String,
    /// The fused RRF score.
    pub score: f64,
    /// `(channel index, 1-based rank, raw channel score)` per contribution.
    pub contributions: Vec<(usize, usize, f32)>,
}

/// Fuse ranked lists with RRF. Deterministic: ties break by ascending id.
/// Duplicate ids WITHIN one channel keep only their best (first) rank.
#[must_use]
pub fn rrf_fuse(channels: &[Vec<RankedEntry>], k: f64) -> Vec<Fused> {
    let mut fused: BTreeMap<String, Fused> = BTreeMap::new();
    for (channel_index, entries) in channels.iter().enumerate() {
        let mut seen_in_channel = std::collections::BTreeSet::new();
        for (position, entry) in entries.iter().enumerate() {
            if !seen_in_channel.insert(entry.id.clone()) {
                continue;
            }
            let rank = position + 1;
            #[allow(clippy::cast_precision_loss)] // ranks are tiny
            let increment = 1.0 / (k + rank as f64);
            let candidate = fused.entry(entry.id.clone()).or_insert_with(|| Fused {
                id: entry.id.clone(),
                score: 0.0,
                contributions: Vec::new(),
            });
            candidate.score += increment;
            candidate
                .contributions
                .push((channel_index, rank, entry.raw_score));
        }
    }
    let mut result: Vec<Fused> = fused.into_values().collect();
    // Descending score; ascending id on ties (BTreeMap already yields ids in
    // order, and the sort below is stable).
    result.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, raw: f32) -> RankedEntry {
        RankedEntry {
            id: id.to_string(),
            raw_score: raw,
        }
    }

    #[test]
    fn fuses_ranks_not_scores_and_breaks_ties_by_id() {
        // doc-b is rank 1 in one channel and rank 2 in the other; doc-a and
        // doc-c each appear once at rank 1/2. Raw scores are wildly different
        // scales and must not matter.
        let channels = vec![
            vec![entry("doc-a", 0.99), entry("doc-b", 0.42)],
            vec![entry("doc-b", 17.0), entry("doc-c", 3.0)],
        ];
        let fused = rrf_fuse(&channels, RRF_K);
        assert_eq!(fused[0].id, "doc-b", "two contributions win");
        assert!(fused[0].score > fused[1].score);
        // doc-a (rank 1, one channel) must beat doc-c (rank 2, one channel).
        assert_eq!(fused[1].id, "doc-a", "rank 1 beats rank 2 at equal count");
        assert_eq!(fused[2].id, "doc-c");
        // Contributions carry (channel, rank, raw) for explanations.
        assert_eq!(fused[0].contributions, vec![(0, 2, 0.42), (1, 1, 17.0)]);

        // Exact tie (same rank, single channel each): ascending id wins.
        let tied = rrf_fuse(
            &[vec![entry("doc-z", 1.0)], vec![entry("doc-a", 1.0)]],
            RRF_K,
        );
        assert_eq!(tied[0].id, "doc-a");
        assert_eq!(tied[1].id, "doc-z");
    }

    #[test]
    fn duplicate_ids_within_a_channel_keep_best_rank_only() {
        let fused = rrf_fuse(&[vec![entry("doc-a", 1.0), entry("doc-a", 0.5)]], RRF_K);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].contributions, vec![(0, 1, 1.0)]);
    }

    #[test]
    fn empty_channels_fuse_to_nothing() {
        assert!(rrf_fuse(&[], RRF_K).is_empty());
        assert!(rrf_fuse(&[Vec::new(), Vec::new()], RRF_K).is_empty());
    }
}
