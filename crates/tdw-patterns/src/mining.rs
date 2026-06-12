//! Deterministic frequent-motif mining engine (knowledge-system K-R4).
//!
//! [`PatternEngine`] scans the temporal graph, enumerates candidate
//! edge-label combinations of size 1..=[`MiningLimits::max_motif_edges`],
//! counts the number of distinct *source entities* that participate in **all**
//! labels in each combination (support counting), and persists combinations
//! that meet `min_support` as [`EntityKind::Pattern`] nodes with provenance
//! edges to their instances.
//!
//! **v1 algorithm note:** support is measured by `from`-entity overlap across
//! edge-label sets — NOT by connected-subgraph matching. See the crate-level
//! doc in `lib.rs` for the full description and planned evolution.
//!
//! # Determinism
//!
//! The engine produces the same output for the same graph snapshot on every
//! invocation. Determinism is guaranteed by:
//!
//! 1. Iterating edges via paginated [`GraphEngine::edges`] (stable sorted order
//!    within each page — `InMemoryGraphEngine` sorts by rel then by from/to).
//! 2. Sorting candidate pairs and instances before processing.
//! 3. Using `BTreeMap`/`BTreeSet` everywhere (no `HashMap`).
//! 4. Canonical motif encoding: edge labels sorted and joined with `"::"`.
//!
//! # Hard bounds (B7 posture — errors, not truncation)
//!
//! | Bound | Exceeded → |
//! |---|---|
//! | `max_candidates` | [`PatternError::CandidateBudgetExceeded`] |
//! | `max_instance_scan` | [`PatternError::InstanceBudgetExceeded`] |
//! | `max_iterations` | [`PatternError::IterationBudgetExceeded`] |
//!
//! # Temporal visibility
//!
//! Edges are filtered to those active at `now` (`valid_from` ≤ now AND
//! `valid_to` > now OR `valid_to` = None). This uses [`tdw_core::active_at`] — the
//! same predicate the graph engine and retriever use, so no independent
//! temporal-visibility implementation is introduced here.
//!
//! # Why-integration
//!
//! Pattern nodes written to the graph carry
//! `Provenance::System { detail: "pattern-mining:v0" }`. Their instance
//! provenance is written as edges with rel `"pattern_instance_of"` and the
//! same System provenance. The `tdw.kg.why` chain composer (K-X1) handles
//! Pattern nodes via the existing `why_entity` path — the entity step
//! surfaces `kind: "pattern"` and the crosswalk path walks
//! `pattern_instance_of` edges (already covered by `identified_by`/`same_as`
//! traversal in `fetch_crosswalk_neighbors`; we extend the why chain in
//! `knowledge_explain_tools.rs` for the `Pattern` kind explicitly).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance, active_at};
use tdw_taxonomy::EntityKind;
use thiserror::Error;

use crate::index::{PatternIndex, PatternRecord, pattern_node_id};

// ── Page size for edge scans ──────────────────────────────────────────────────

const EDGE_PAGE: usize = 256;

// ── MiningLimits ─────────────────────────────────────────────────────────────

/// Hard bounds for one mining run.
///
/// Exceeding ANY bound is a [`PatternError`], never silent truncation (B7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MiningLimits {
    /// Maximum distinct (from, to) candidate pairs examined across all
    /// motif sizes. Default: 8 000.
    pub max_candidates: usize,
    /// Maximum edge records scanned during instance counting. Default: 20 000.
    pub max_instance_scan: usize,
    /// Maximum iteration steps (inner loop ticks). Default: 50 000.
    pub max_iterations: usize,
    /// Maximum number of instance provenance edges written per pattern node
    /// (caps graph fan-out). Default: 64.
    pub max_provenance_edges: usize,
    /// Maximum number of edge labels in one motif (1..=3). Default: 3.
    pub max_motif_edges: usize,
    /// Minimum support count for a motif to be recorded. Default: 2.
    pub min_support: usize,
}

impl Default for MiningLimits {
    fn default() -> Self {
        Self {
            max_candidates: 8_000,
            max_instance_scan: 20_000,
            max_iterations: 50_000,
            max_provenance_edges: 64,
            max_motif_edges: 3,
            min_support: 2,
        }
    }
}

// ── MiningReport ─────────────────────────────────────────────────────────────

/// What one mining run produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MiningReport {
    /// Number of new Pattern nodes written to the graph.
    pub patterns_created: usize,
    /// Number of existing Pattern nodes whose support count was updated.
    pub patterns_updated: usize,
    /// Number of motifs examined (including those below `min_support`).
    pub motifs_examined: usize,
    /// Number of instance provenance edges written.
    pub instance_edges_written: usize,
}

// ── PatternError ─────────────────────────────────────────────────────────────

/// Mining errors (all hard — B7 posture).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatternError {
    /// Candidate-pair budget exceeded.
    #[error("pattern mining candidate budget exceeded: {limit}")]
    CandidateBudgetExceeded {
        /// The limit that was hit.
        limit: usize,
    },
    /// Instance-scan budget exceeded.
    #[error("pattern mining instance-scan budget exceeded: {limit}")]
    InstanceBudgetExceeded {
        /// The limit that was hit.
        limit: usize,
    },
    /// Iteration budget exceeded.
    #[error("pattern mining iteration budget exceeded: {limit}")]
    IterationBudgetExceeded {
        /// The limit that was hit.
        limit: usize,
    },
    /// A graph engine call failed.
    #[error("graph error: {0}")]
    Graph(String),
    /// (De)serialization of the pattern index failed.
    #[error("serde error: {0}")]
    Serde(String),
}

// ── PatternEngine ─────────────────────────────────────────────────────────────

/// The deterministic motif-mining engine.
///
/// The engine is stateless between calls — all mutable state flows through
/// the caller-owned [`PatternIndex`] (same pattern as [`DerivationIndex`] in
/// `tdw-infer`).
#[derive(Default)]
pub struct PatternEngine {
    limits: MiningLimits,
}

impl PatternEngine {
    /// A fresh engine with the given limits.
    #[must_use]
    pub const fn with_limits(limits: MiningLimits) -> Self {
        Self { limits }
    }

    /// Mine patterns from the graph at temporal point `now`, persisting
    /// `Pattern` nodes and updating `index`.
    ///
    /// `window` is a human-readable label stored on each `PatternRecord`
    /// (e.g. `"2025-01-01"` or `"all"`). Mining is idempotent: existing
    /// `Pattern` nodes have their support count updated, not duplicated.
    ///
    /// # Errors
    ///
    /// Returns a hard [`PatternError`] if any budget is exceeded, or if a
    /// graph engine call fails. Never silently truncates.
    pub async fn run_pattern_mining_at(
        &self,
        graph: &Arc<dyn GraphEngine>,
        index: &mut PatternIndex,
        now: &str,
        window: &str,
    ) -> Result<MiningReport, PatternError> {
        // 1. Collect all edges active at `now` (temporal gate).
        let edges = self.collect_active_edges(graph, now).await?;

        // 2. Build label → [(from, to)] lookup.
        let mut by_label: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for edge in &edges {
            by_label
                .entry(edge.rel.clone())
                .or_default()
                .push((edge.from.clone(), edge.to.clone()));
        }

        // 3. For each motif size 1..=max_motif_edges, enumerate candidate
        //    label combinations and count instances.
        let labels: Vec<String> = by_label.keys().cloned().collect();
        let mut report = MiningReport::default();
        let mut iterations: usize = 0;

        for size in 1..=self.limits.max_motif_edges {
            let combinations = label_combinations(&labels, size);
            for combo in combinations {
                iterations += 1;
                if iterations > self.limits.max_iterations {
                    return Err(PatternError::IterationBudgetExceeded {
                        limit: self.limits.max_iterations,
                    });
                }

                // Canonical: combo is already sorted (label_combinations sorts).
                let canonical = combo.join("::");

                // Count instances: entities that appear as `from` in ALL labels
                // of the combo. For a 1-edge motif this is just the pairs for
                // that label. For multi-edge motifs we intersect on `from`.
                let instances = self.count_instances(&by_label, &combo, &mut iterations)?;

                report.motifs_examined += 1;

                if instances.len() < self.limits.min_support {
                    continue;
                }

                // Persist the Pattern node and update index.
                let capped_instances: Vec<(String, String)> = instances
                    .into_iter()
                    .take(self.limits.max_provenance_edges)
                    .collect();
                let support = capped_instances.len();

                let record = PatternRecord::new(
                    canonical.clone(),
                    support,
                    window.to_string(),
                    capped_instances.clone(),
                );

                let is_new = !index.contains(&canonical);
                index.record(record);

                if is_new {
                    report.patterns_created += 1;
                } else {
                    report.patterns_updated += 1;
                }

                // Write Pattern node + provenance instance edges to graph.
                let edges_written = self
                    .persist_pattern(graph, &canonical, support, window, &capped_instances, now)
                    .await?;
                report.instance_edges_written += edges_written;
            }
        }

        Ok(report)
    }

    /// Count entity-pair instances matching a multi-label motif.
    ///
    /// Strategy: collect `from` entities that participate in ALL labels,
    /// then build (from, to) pairs for the first and last label.
    ///
    /// Bounded by `max_candidates` and `max_instance_scan`.
    fn count_instances(
        &self,
        by_label: &BTreeMap<String, Vec<(String, String)>>,
        combo: &[String],
        iterations: &mut usize,
    ) -> Result<Vec<(String, String)>, PatternError> {
        // Start with the from-set of the first label.
        let Some(first_pairs) = by_label.get(&combo[0]) else {
            return Ok(Vec::new());
        };

        let mut scan_count: usize = 0;

        // For size=1: instances are exactly the pairs.
        if combo.len() == 1 {
            scan_count += first_pairs.len();
            if scan_count > self.limits.max_instance_scan {
                return Err(PatternError::InstanceBudgetExceeded {
                    limit: self.limits.max_instance_scan,
                });
            }
            let mut result: Vec<(String, String)> = first_pairs.clone();
            result.sort();
            result.dedup();
            // Enforce candidate budget: if the de-duped pair count exceeds
            // max_candidates the miner would write more pattern instances than
            // the budget allows — treat as a hard CandidateBudgetExceeded.
            if result.len() > self.limits.max_candidates {
                return Err(PatternError::CandidateBudgetExceeded {
                    limit: self.limits.max_candidates,
                });
            }
            return Ok(result);
        }

        // For multi-label: intersect `from` sets across all labels, then
        // return (from, to_last) pairs where `from` appears in all labels.
        let mut from_in_all: BTreeSet<String> =
            first_pairs.iter().map(|(f, _)| f.clone()).collect();

        for label in &combo[1..] {
            *iterations += 1;
            if *iterations > self.limits.max_iterations {
                return Err(PatternError::IterationBudgetExceeded {
                    limit: self.limits.max_iterations,
                });
            }
            let Some(label_pairs) = by_label.get(label) else {
                return Ok(Vec::new());
            };
            scan_count += label_pairs.len();
            if scan_count > self.limits.max_instance_scan {
                return Err(PatternError::InstanceBudgetExceeded {
                    limit: self.limits.max_instance_scan,
                });
            }
            let label_froms: BTreeSet<String> =
                label_pairs.iter().map(|(f, _)| f.clone()).collect();
            from_in_all = from_in_all.intersection(&label_froms).cloned().collect();
            if from_in_all.is_empty() {
                return Ok(Vec::new());
            }
        }

        // Build (from, to) using the last label's `to` for each qualifying `from`.
        let last_label = &combo[combo.len() - 1];
        let last_pairs = by_label.get(last_label).map_or(&[] as &[_], Vec::as_slice);

        // Enforce instance-scan budget on the final pass too (finding #6 — dead-cap class).
        scan_count += last_pairs.len();
        if scan_count > self.limits.max_instance_scan {
            return Err(PatternError::InstanceBudgetExceeded {
                limit: self.limits.max_instance_scan,
            });
        }

        let mut result: Vec<(String, String)> = Vec::new();
        for (f, t) in last_pairs {
            if from_in_all.contains(f) {
                if result.len() >= self.limits.max_candidates {
                    return Err(PatternError::CandidateBudgetExceeded {
                        limit: self.limits.max_candidates,
                    });
                }
                result.push((f.clone(), t.clone()));
            }
        }
        result.sort();
        result.dedup();
        Ok(result)
    }

    /// Collect all edges active at `now` from the graph.
    ///
    /// `pattern_instance_of` edges are excluded: they are provenance edges
    /// written by the miner itself and must not participate in motif
    /// construction (which would cause self-referential patterns and
    /// non-determinism on re-mining — the determinism gate test verifies this).
    async fn collect_active_edges(
        &self,
        graph: &Arc<dyn GraphEngine>,
        now: &str,
    ) -> Result<Vec<GraphEdge>, PatternError> {
        let mut all = Vec::new();
        let mut offset = 0;
        loop {
            let page = graph
                .edges(None, offset, EDGE_PAGE)
                .await
                .map_err(|e| PatternError::Graph(e.to_string()))?;
            let count = page.len();
            for edge in page {
                // Skip the miner's own provenance edges so re-mining does not
                // discover patterns over pattern_instance_of edges.
                if edge.rel == "pattern_instance_of" {
                    continue;
                }
                if active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), now) {
                    all.push(edge);
                }
            }
            if count < EDGE_PAGE {
                break;
            }
            offset += EDGE_PAGE;
        }
        Ok(all)
    }

    /// Write a Pattern node and its instance provenance edges to the graph.
    ///
    /// Idempotent: `upsert_nodes` / `upsert_edges` on the same id updates in place.
    /// Returns the number of instance edges written.
    async fn persist_pattern(
        &self,
        graph: &Arc<dyn GraphEngine>,
        canonical: &str,
        support: usize,
        window: &str,
        instances: &[(String, String)],
        now: &str,
    ) -> Result<usize, PatternError> {
        let node_id = pattern_node_id(canonical);
        let prov = Provenance::System {
            detail: "pattern-mining:v0".to_string(),
        };

        // Write the Pattern node.
        let node = GraphNode {
            id: node_id.clone(),
            kind: EntityKind::Pattern,
            label: format!("Pattern: {canonical}"),
            aliases: Vec::new(),
            props: serde_json::json!({
                "canonical": canonical,
                "support": support,
                "window": window,
                "mined_at": now,
                "instance_count": instances.len(),
                "instance_cap": self.limits.max_provenance_edges,
                "min_support": self.limits.min_support,
            }),
            valid_from: Some(now.to_string()),
            valid_to: None,
        };
        graph
            .upsert_nodes(vec![node])
            .await
            .map_err(|e| PatternError::Graph(e.to_string()))?;

        // Write instance provenance edges: pattern_node -pattern_instance_of-> instance_from.
        // Bounded to max_provenance_edges (already capped at call site).
        let edges: Vec<tdw_core::GraphEdge> = instances
            .iter()
            .map(|(from, _to)| tdw_core::GraphEdge {
                from: node_id.clone(),
                to: from.clone(),
                rel: "pattern_instance_of".to_string(),
                props: serde_json::Value::Null,
                provenance: prov.clone(),
                valid_from: Some(now.to_string()),
                valid_to: None,
            })
            .collect();

        let edge_count = edges.len();
        if !edges.is_empty() {
            graph
                .upsert_edges(edges)
                .await
                .map_err(|e| PatternError::Graph(e.to_string()))?;
        }

        Ok(edge_count)
    }
}

// ── Motif helpers ─────────────────────────────────────────────────────────────

/// Generate all combinations of `size` labels from `labels`, sorted
/// lexicographically within each combination (canonical order).
///
/// Result is sorted by the canonical string for deterministic iteration.
fn label_combinations(labels: &[String], size: usize) -> Vec<Vec<String>> {
    let n = labels.len();
    if size == 0 || size > n {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut indices: Vec<usize> = (0..size).collect();
    loop {
        let mut combo: Vec<String> = indices.iter().map(|&i| labels[i].clone()).collect();
        combo.sort();
        result.push(combo);

        // Advance to next combination.
        let mut pos = size.saturating_sub(1);
        loop {
            indices[pos] += 1;
            if indices[pos] <= n - (size - pos) {
                for next in (pos + 1)..size {
                    indices[next] = indices[next - 1] + 1;
                }
                break;
            }
            if pos == 0 {
                return result;
            }
            pos -= 1;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_combinations_size_1() {
        let labels: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let combos = label_combinations(&labels, 1);
        assert_eq!(combos.len(), 3);
        assert!(combos.contains(&vec!["a".to_string()]));
    }

    #[test]
    fn label_combinations_size_2() {
        let labels: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let combos = label_combinations(&labels, 2);
        // C(3,2) = 3
        assert_eq!(combos.len(), 3);
        assert!(combos.contains(&vec!["a".to_string(), "b".to_string()]));
        assert!(combos.contains(&vec!["a".to_string(), "c".to_string()]));
        assert!(combos.contains(&vec!["b".to_string(), "c".to_string()]));
    }

    #[test]
    fn label_combinations_empty_when_size_exceeds_labels() {
        let labels: Vec<String> = ["a"].map(String::from).to_vec();
        assert!(label_combinations(&labels, 2).is_empty());
        assert!(label_combinations(&labels, 0).is_empty());
    }

    #[test]
    fn combinations_are_internally_sorted() {
        // Even if labels are provided in reverse order, each combo is sorted.
        let labels: Vec<String> = ["c", "a", "b"].map(String::from).to_vec();
        for combo in label_combinations(&labels, 2) {
            assert!(combo[0] <= combo[1], "combo not sorted: {combo:?}");
        }
    }
}
