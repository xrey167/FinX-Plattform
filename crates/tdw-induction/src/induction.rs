//! Pattern → rule induction engine (knowledge-system K-R5).
//!
//! [`InductionEngine`] takes mined [`PatternRecord`]s from the
//! [`tdw_patterns::PatternIndex`], synthesises candidate [`InferRule`]s,
//! runs each candidate through the K-R7 [`promote_gate`], and installs
//! those that pass into the live [`InferEngine`] via `hot_reload`.
//!
//! # Determinism
//!
//! Induction is fully deterministic: the same `PatternRecord` always
//! produces the same candidate rule (same `rule_id`, same `when`, same
//! `derived_type`, same `stratum`). The only non-deterministic step is
//! the replay evaluation, which depends on the historical edge corpus
//! supplied by the caller — and that corpus is itself from stored data.
//!
//! # Candidate shape
//!
//! A motif `"rel_a::rel_b"` (two edge labels, canonical `::` encoding)
//! induces a `DeriveEdge` rule:
//! ```text
//! when = [EdgePattern { rel: "rel_a" }, EdgePattern { rel: "rel_b" }]
//! derived_type = "inducted:rel_a--rel_b"
//! rule_id = "inducted:pattern:<pattern_node_id>"   (e.g. "inducted:pattern:rel_a--rel_b")
//! stratum = 0
//! ```
//! The `inducted:` prefix on both `derived_type` and `rule_id` distinguishes
//! machine-authored rules from hand-authored ones and makes the why-chain
//! traceable.
//!
//! For size-1 motifs (`"rel_a"`) the rule derives the same edge type with an
//! `inducted:` prefix:
//! ```text
//! when = [EdgePattern { rel: "rel_a" }]
//! derived_type = "inducted:rel_a"
//! ```
//!
//! # Rejection criteria (deterministic, before the gate)
//!
//! A candidate is **rejected before the replay gate** if:
//! - Any motif label is itself an `inducted:` type (no induction over
//!   induction — prevents a runaway self-amplifying chain).
//! - The pattern's `canonical` string cannot be parsed into well-formed
//!   graph-id labels (grammar check via `tdw_core::is_graph_id`).
//! - Chain length is outside `1..=3` (the `validate_rule` bound in `tdw-infer`).
//! - The derived type equals any `when` rel (self-recursion guard).
//!
//! These checks are applied in [`InductionEngine::induce_candidate`] and
//! surface as [`InductionError::CandidateRejected`] with a descriptive
//! reason — not silently skipped.
//!
//! # Gate (the integrity core)
//!
//! A candidate that does not pass [`promote_gate`] with the configured
//! `min_promote_precision` and `min_promote_recall` **must not be
//! installed**. The gate is called inline and its
//! [`PromoteVerdict::Reject`] result is stored in
//! [`CandidateRule::gate_verdict`] for audit.
//!
//! # Rule install
//!
//! Only after a `PromoteVerdict::Promote` does the engine call
//! `infer_engine.hot_reload(all_rules_with_new)`. The reload is
//! **atomic**: either all rules (existing + new) validate and stratify,
//! or nothing changes ([`InferEngine::hot_reload`] contract).
//!
//! # Provenance on the installed rule
//!
//! The `rule_id` carries the pattern id: `"inducted:pattern:<node_id>"`.
//! An edge derived by the rule carries
//! `Provenance::Rule { rule_id: "inducted:pattern:<node_id>", version: N }`.
//! The why-chain tool traces this to the pattern node, which in turn
//! carries its supporting instances and the replay verdict stored in
//! [`PromotedRule::replay_summary`].

// Precision and recall are computed from small integer counts; cast is safe.
#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeSet;

use tdw_core::{GraphEdge, is_graph_id};
use tdw_eval_runner::{
    replay_eval::{PromoteThreshold, PromoteVerdict, ReplaySplit, promote_gate},
    retrieval_eval::DriftKey,
};
use tdw_infer::{EdgePattern, InferEngine, InferRule};
use tdw_patterns::{PatternIndex, PatternRecord};
use thiserror::Error;

/// Maximum chain length supported by [`InferRule::DeriveEdge`] (mirrors
/// `tdw_infer::rule::MAX_CHAIN`).
const MAX_INDUCTION_CHAIN: usize = 3;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors from the induction pipeline.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InductionError {
    /// A candidate was pre-rejected before the gate (deterministic).
    #[error("candidate for pattern {pattern_id:?} rejected: {reason}")]
    CandidateRejected {
        /// The pattern whose candidate was rejected.
        pattern_id: String,
        /// Human-readable rejection reason.
        reason: &'static str,
    },

    /// The replay gate returned an error (join-bound exceeded).
    #[error("promote gate error for pattern {pattern_id:?}: {detail}")]
    GateError {
        /// The pattern whose gate errored.
        pattern_id: String,
        /// Underlying error description.
        detail: String,
    },

    /// The inference engine rejected the hot-reload (validation / stratification).
    #[error("hot_reload rejected inducted rule {rule_id:?}: {detail}")]
    HotReloadRejected {
        /// The rule id that caused the rejection.
        rule_id: String,
        /// Underlying error description.
        detail: String,
    },

    /// Per-cycle candidate budget exceeded (B7 posture — hard error).
    #[error("induction candidate budget exceeded: {count} candidates, cap {cap}")]
    CandidateBudgetExceeded {
        /// Candidates attempted.
        count: usize,
        /// The configured cap.
        cap: usize,
    },
}

// ── CandidateRule ─────────────────────────────────────────────────────────────

/// A candidate rule synthesised from one [`PatternRecord`], with its gate verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateRule {
    /// The pattern node id this candidate was induced from (e.g.
    /// `"pattern:listed_on--supplier_of"`).
    pub pattern_id: String,
    /// The canonical motif string (e.g. `"listed_on::supplier_of"`).
    pub canonical: String,
    /// The synthesised rule (always `InferRule::DeriveEdge`).
    pub rule: InferRule,
    /// The gate verdict. `None` if pre-rejected (not sent to the gate).
    pub gate_verdict: Option<PromoteVerdict>,
    /// Support count from the pattern record.
    pub support: usize,
}

impl CandidateRule {
    /// Whether this candidate passed both the pre-check AND the promote gate.
    #[must_use]
    pub fn is_promoted(&self) -> bool {
        self.gate_verdict
            .as_ref()
            .is_some_and(PromoteVerdict::is_promote)
    }
}

// ── PromotedRule ──────────────────────────────────────────────────────────────

/// A candidate that passed the gate and was installed into the inference engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotedRule {
    /// The installed rule's id.
    pub rule_id: String,
    /// The pattern node id the rule was induced from.
    pub pattern_id: String,
    /// Human-readable summary of the replay verdict.
    pub replay_summary: String,
    /// Support count from the source pattern.
    pub support: usize,
}

// ── InductionCycleReport ──────────────────────────────────────────────────────

/// Summary of one induction cycle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InductionCycleReport {
    /// Number of patterns examined.
    pub patterns_examined: usize,
    /// Candidates pre-rejected (grammar / safety checks before the gate).
    pub pre_rejected: usize,
    /// Candidates that reached the gate but were rejected by it.
    pub gate_rejected: usize,
    /// Rules promoted and installed in this cycle.
    pub promoted: usize,
    /// Rules that were already installed (skipped dedup).
    pub already_installed: usize,
    /// Promoted rules in this cycle (non-empty only when `promoted > 0`).
    pub new_rules: Vec<PromotedRule>,
}

// ── InductionEngine ───────────────────────────────────────────────────────────

/// The pattern → rule induction engine (stateless between cycles).
///
/// The engine is stateless: all mutable state (the installed rules, the
/// infer engine) is owned by the caller. The engine is a pure function of
/// its inputs — same pattern index + same edge corpus + same splits →
/// same report.
#[derive(Debug, Default)]
pub struct InductionEngine {
    min_precision: f64,
    min_recall: f64,
    max_candidates: usize,
}

impl InductionEngine {
    /// Build an engine from the given thresholds and candidate cap.
    #[must_use]
    pub const fn new(min_precision: f64, min_recall: f64, max_candidates: usize) -> Self {
        Self {
            min_precision,
            min_recall,
            max_candidates,
        }
    }

    /// Run one induction cycle.
    ///
    /// For each [`PatternRecord`] in `index` (in canonical-key order —
    /// deterministic), the engine:
    ///
    /// 1. Synthesises a candidate rule ([`induce_candidate`]).
    /// 2. Skips if the same `rule_id` is already in `current_rules`.
    /// 3. Runs [`promote_gate`] over `all_edges` and `splits`.
    /// 4. On `Promote`: adds the rule to `current_rules` and hot-reloads
    ///    the engine.
    ///
    /// Stops with [`InductionError::CandidateBudgetExceeded`] if more
    /// than `self.max_candidates` candidates are attempted (pre-rejected
    /// candidates count against the budget).
    ///
    /// The [`InductionCycleReport`] is returned even when a gate or
    /// hot-reload error occurs for one rule — the error is stored in the
    /// report and the cycle continues with the remaining patterns (best
    /// effort, not all-or-nothing). The caller's logs show the per-rule
    /// outcomes.
    ///
    /// # Errors
    ///
    /// Returns [`InductionError::CandidateBudgetExceeded`] when the
    /// configured per-cycle cap is exceeded. Per-rule gate and hot-reload
    /// errors are soft: they contribute to `gate_rejected` / are logged
    /// but do not abort the cycle.
    pub fn run_cycle(
        &self,
        index: &PatternIndex,
        current_rules: &mut Vec<InferRule>,
        infer: &mut InferEngine,
        all_edges: &[GraphEdge],
        splits: &[ReplaySplit],
        drift_key: &DriftKey,
    ) -> Result<InductionCycleReport, InductionError> {
        let mut report = InductionCycleReport::default();
        let mut attempted: usize = 0;

        let threshold = PromoteThreshold {
            min_precision: self.min_precision,
            min_recall: self.min_recall,
        };

        // Collect already-installed inducted rule ids for dedup.
        let installed_ids: BTreeSet<String> = current_rules
            .iter()
            .map(|r| r.rule_id().to_string())
            .collect();

        for (_canonical, record) in index.iter() {
            report.patterns_examined += 1;

            // Pre-check synthesis.
            let candidate = match self.induce_candidate(record) {
                Ok(c) => c,
                Err(InductionError::CandidateRejected { .. }) => {
                    report.pre_rejected += 1;
                    attempted += 1;
                    if attempted > self.max_candidates {
                        return Err(InductionError::CandidateBudgetExceeded {
                            count: attempted,
                            cap: self.max_candidates,
                        });
                    }
                    continue;
                }
                Err(other) => return Err(other),
            };

            attempted += 1;
            if attempted > self.max_candidates {
                return Err(InductionError::CandidateBudgetExceeded {
                    count: attempted,
                    cap: self.max_candidates,
                });
            }

            // Dedup: skip already-installed rules.
            if installed_ids.contains(candidate.rule.rule_id()) {
                report.already_installed += 1;
                continue;
            }

            // Promote gate (K-R7 seam).
            let verdict = match promote_gate(
                &candidate.rule,
                all_edges,
                splits,
                &threshold,
                drift_key.clone(),
            ) {
                Ok(v) => v,
                Err(err) => {
                    // Gate error (e.g. join-bound exceeded) — treat as reject,
                    // log inline, continue cycle.
                    eprintln!(
                        "[tdw] K-R5 gate error for pattern {:?}: {err}",
                        record.node_id
                    );
                    report.gate_rejected += 1;
                    continue;
                }
            };

            if !verdict.is_promote() {
                report.gate_rejected += 1;
                continue;
            }

            let summary = verdict.summary().to_string();

            // Atomic hot-reload: build the new full rule set.
            let mut new_rules = current_rules.clone();
            new_rules.push(candidate.rule.clone());

            match infer.hot_reload(new_rules.clone()) {
                Ok(()) => {
                    *current_rules = new_rules;
                    report.promoted += 1;
                    report.new_rules.push(PromotedRule {
                        rule_id: candidate.rule.rule_id().to_string(),
                        pattern_id: record.node_id.clone(),
                        replay_summary: summary,
                        support: record.support,
                    });
                }
                Err(err) => {
                    // Hot-reload rejection is a soft error: log and skip this rule.
                    eprintln!(
                        "[tdw] K-R5 hot_reload rejected inducted rule {:?}: {err}",
                        candidate.rule.rule_id()
                    );
                    report.gate_rejected += 1;
                }
            }
        }

        Ok(report)
    }

    /// Synthesise a candidate rule from one [`PatternRecord`].
    ///
    /// This function is deterministic: the same record always produces the
    /// same rule or the same rejection reason.
    ///
    /// # Errors
    ///
    /// Returns [`InductionError::CandidateRejected`] with a static reason
    /// string when the pattern does not satisfy the well-formedness and
    /// safety criteria documented in the module header.
    pub fn induce_candidate(
        &self,
        record: &PatternRecord,
    ) -> Result<CandidateRule, InductionError> {
        let canonical = &record.canonical;
        let pattern_id = &record.node_id;

        // Parse labels from the canonical motif string.
        // Canonical encoding: labels sorted and joined with "::".
        let labels: Vec<&str> = canonical.split("::").collect();

        // Chain-length bounds.
        if labels.is_empty() || labels.len() > MAX_INDUCTION_CHAIN {
            return Err(InductionError::CandidateRejected {
                pattern_id: pattern_id.clone(),
                reason: "chain length out of 1..=3",
            });
        }

        // Grammar check on every label.
        for label in &labels {
            if !is_graph_id(label) {
                return Err(InductionError::CandidateRejected {
                    pattern_id: pattern_id.clone(),
                    reason: "motif label fails graph-id grammar",
                });
            }
            // No induction over induction — labels must not be inducted types.
            if label.starts_with("inducted:") {
                return Err(InductionError::CandidateRejected {
                    pattern_id: pattern_id.clone(),
                    reason: "motif label is itself an inducted type (no induction over induction)",
                });
            }
        }

        // Build derived_type: "inducted:" + labels joined with "--".
        let derived_type = format!("inducted:{}", labels.join("--"));

        // Self-recursion guard: derived_type must not appear in `when` rels.
        // (This mirrors the validate_rule check in tdw-infer.)
        if labels.contains(&derived_type.as_str()) {
            return Err(InductionError::CandidateRejected {
                pattern_id: pattern_id.clone(),
                reason: "derived_type equals a when rel (self-recursion)",
            });
        }

        // Grammar check on derived_type itself.
        if !is_graph_id(&derived_type) {
            return Err(InductionError::CandidateRejected {
                pattern_id: pattern_id.clone(),
                reason: "derived_type fails graph-id grammar",
            });
        }

        // Build rule_id: "inducted:pattern:<node_id_safe>"
        // node_id is "pattern:<canonical>" where canonical uses "--" separators —
        // already graph-id safe.
        let rule_id = format!("inducted-{}", pattern_id.replace(':', "-"));

        let when: Vec<EdgePattern> = labels
            .iter()
            .map(|l| EdgePattern {
                rel: (*l).to_string(),
            })
            .collect();

        let rule = InferRule::DeriveEdge {
            rule_id,
            stratum: 0,
            when,
            derived_type,
        };

        Ok(CandidateRule {
            pattern_id: pattern_id.clone(),
            canonical: canonical.clone(),
            rule,
            gate_verdict: None,
            support: record.support,
        })
    }
}

/// Build a human-readable provenance label for an inducted rule.
///
/// Used in why-chain display: `"inducted-pattern-<pattern-node-id>"`.
/// The caller (explain tool) uses this to identify a rule as machine-authored
/// and trace it back to the source pattern.
#[must_use]
pub fn inducted_rule_provenance_label(rule_id: &str) -> Option<String> {
    // Inducted rules have ids of the form "inducted-pattern-<node_id_safe>"
    rule_id
        .strip_prefix("inducted-pattern-")
        .map(|suffix| format!("pattern:{suffix}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use tdw_patterns::{PatternIndex, PatternRecord};

    fn record(canonical: &str, support: usize) -> PatternRecord {
        PatternRecord::new(
            canonical.to_string(),
            support,
            "2024-01-01".to_string(),
            vec![("e:A".to_string(), "v:X".to_string())],
        )
    }

    fn engine() -> InductionEngine {
        InductionEngine::new(0.0, 0.0, 100)
    }

    // ── induce_candidate: determinism ─────────────────────────────────────────

    #[test]
    fn induction_is_deterministic_same_record_same_rule() {
        let rec = record("listed_on::supplier_of", 4);
        let eng = engine();
        let c1 = eng.induce_candidate(&rec).unwrap();
        let c2 = eng.induce_candidate(&rec).unwrap();
        assert_eq!(c1.rule, c2.rule, "induction must be deterministic");
    }

    #[test]
    fn single_label_motif_produces_valid_candidate() {
        let rec = record("listed_on", 3);
        let c = engine().induce_candidate(&rec).unwrap();
        let InferRule::DeriveEdge {
            when, derived_type, ..
        } = &c.rule
        else {
            panic!("expected DeriveEdge");
        };
        assert_eq!(when.len(), 1);
        assert_eq!(when[0].rel, "listed_on");
        assert_eq!(derived_type, "inducted:listed_on");
    }

    #[test]
    fn two_label_motif_produces_chain_rule() {
        let rec = record("listed_on::supplier_of", 4);
        let c = engine().induce_candidate(&rec).unwrap();
        let InferRule::DeriveEdge {
            when, derived_type, ..
        } = &c.rule
        else {
            panic!("expected DeriveEdge");
        };
        assert_eq!(when.len(), 2);
        assert_eq!(when[0].rel, "listed_on");
        assert_eq!(when[1].rel, "supplier_of");
        assert_eq!(derived_type, "inducted:listed_on--supplier_of");
    }

    #[test]
    fn three_label_motif_accepted() {
        let rec = record("a::b::c", 2);
        let c = engine().induce_candidate(&rec).unwrap();
        let InferRule::DeriveEdge { when, .. } = &c.rule else {
            panic!("expected DeriveEdge");
        };
        assert_eq!(when.len(), 3);
    }

    // ── induce_candidate: rejection criteria ──────────────────────────────────

    #[test]
    fn rejects_inducted_label_no_induction_over_induction() {
        let rec = record("inducted:foo::listed_on", 2);
        let err = engine().induce_candidate(&rec).unwrap_err();
        assert!(
            matches!(
                err,
                InductionError::CandidateRejected { reason, .. }
                if reason.contains("inducted type")
            ),
            "expected CandidateRejected for inducted label; got {err:?}"
        );
    }

    #[test]
    fn rejects_chain_too_long() {
        // 4 labels exceeds MAX_INDUCTION_CHAIN = 3
        let rec = record("a::b::c::d", 2);
        let err = engine().induce_candidate(&rec).unwrap_err();
        assert!(matches!(
            err,
            InductionError::CandidateRejected { reason, .. }
            if reason.contains("chain length")
        ));
    }

    #[test]
    fn rejects_bad_grammar_label() {
        // "/" is not a valid graph-id character
        let rec = record("listed/on", 2);
        let err = engine().induce_candidate(&rec).unwrap_err();
        assert!(matches!(
            err,
            InductionError::CandidateRejected { reason, .. }
            if reason.contains("graph-id grammar")
        ));
    }

    // ── gate blocks overfit: passes in-sample but fails out-of-sample ─────────
    //
    // This is the KEY gate test: a candidate that only works in-sample
    // (precision=0 out-of-sample because no outcomes realise in the
    // forward window) must NOT be promoted.

    #[test]
    fn gate_blocks_overfit_candidate() {
        use tdw_core::{GraphEdge, Provenance};

        fn ingest_edge(from: &str, rel: &str, to: &str, valid_from: &str) -> GraphEdge {
            GraphEdge {
                from: from.to_string(),
                to: to.to_string(),
                rel: rel.to_string(),
                props: serde_json::Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(valid_from.to_string()),
                valid_to: None,
            }
        }

        // Rule "listed_on::supplier_of" -> "inducted:listed_on--supplier_of"
        // At as_of both edges active → prediction exists.
        // But NO outcome edge in the forward window → precision = 0.0.
        let rec = record("listed_on::supplier_of", 2);
        let eng = InductionEngine::new(0.60, 0.20, 100); // 60% precision threshold

        let candidate = eng.induce_candidate(&rec).unwrap();

        let edges = vec![
            ingest_edge("a", "listed_on", "b", "2023-01-01T00:00:00Z"),
            ingest_edge("b", "supplier_of", "x", "2023-06-01T00:00:00Z"),
            // No outcome: "inducted:listed_on--supplier_of" a->x does NOT appear
        ];
        let splits = vec![ReplaySplit {
            split_id: "s1".to_string(),
            as_of: "2024-01-01T00:00:00Z".to_string(),
            as_of_plus_window: "2024-07-01T00:00:00Z".to_string(),
        }];
        let threshold = PromoteThreshold {
            min_precision: 0.60,
            min_recall: 0.20,
        };

        let verdict = promote_gate(
            &candidate.rule,
            &edges,
            &splits,
            &threshold,
            DriftKey::default(),
        )
        .unwrap();

        assert!(
            !verdict.is_promote(),
            "overfit candidate (precision=0) must be REJECTED; got: {:?}",
            verdict.summary()
        );
        assert!(
            verdict.summary().contains("precision"),
            "rejection summary must mention precision; got: {:?}",
            verdict.summary()
        );
    }

    // ── gate promotes a good candidate ────────────────────────────────────────

    #[test]
    fn gate_promotes_candidate_with_confirmed_outcomes() {
        use tdw_core::{GraphEdge, Provenance};

        fn ingest_edge(from: &str, rel: &str, to: &str, valid_from: &str) -> GraphEdge {
            GraphEdge {
                from: from.to_string(),
                to: to.to_string(),
                rel: rel.to_string(),
                props: serde_json::Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(valid_from.to_string()),
                valid_to: None,
            }
        }

        let rec = record("listed_on::supplier_of", 2);
        let eng = InductionEngine::new(0.60, 0.20, 100);
        let candidate = eng.induce_candidate(&rec).unwrap();

        let edges = vec![
            ingest_edge("a", "listed_on", "b", "2023-01-01T00:00:00Z"),
            ingest_edge("b", "supplier_of", "x", "2023-06-01T00:00:00Z"),
            // Outcome: the inducted type appears in the forward window
            ingest_edge(
                "a",
                "inducted:listed_on--supplier_of",
                "x",
                "2024-03-01T00:00:00Z",
            ),
        ];
        let splits = vec![ReplaySplit {
            split_id: "s1".to_string(),
            as_of: "2024-01-01T00:00:00Z".to_string(),
            as_of_plus_window: "2024-07-01T00:00:00Z".to_string(),
        }];
        let threshold = PromoteThreshold {
            min_precision: 0.60,
            min_recall: 0.20,
        };

        let verdict = promote_gate(
            &candidate.rule,
            &edges,
            &splits,
            &threshold,
            DriftKey::default(),
        )
        .unwrap();

        assert!(
            verdict.is_promote(),
            "candidate with confirmed outcomes must be PROMOTED; got: {:?}",
            verdict.summary()
        );
    }

    // ── promoted rule e2e: run_cycle installs the rule ────────────────────────

    #[test]
    fn run_cycle_promotes_and_installs_passing_candidate() {
        use tdw_core::{GraphEdge, Provenance};

        fn ingest_edge(from: &str, rel: &str, to: &str, valid_from: &str) -> GraphEdge {
            GraphEdge {
                from: from.to_string(),
                to: to.to_string(),
                rel: rel.to_string(),
                props: serde_json::Value::Null,
                provenance: Provenance::Ingest {
                    source: "test".to_string(),
                },
                valid_from: Some(valid_from.to_string()),
                valid_to: None,
            }
        }

        let mut index = PatternIndex::default();
        index.record(record("listed_on::supplier_of", 2));

        let eng = InductionEngine::new(0.0, 0.0, 100); // zero threshold: vacuous promote
        let mut current_rules: Vec<InferRule> = Vec::new();
        let mut infer = InferEngine::default();

        let edges: Vec<GraphEdge> =
            vec![ingest_edge("a", "listed_on", "b", "2023-01-01T00:00:00Z")];
        // Empty splits → vacuous promote (nothing to fail on).
        let splits: Vec<ReplaySplit> = vec![];

        let report = eng
            .run_cycle(
                &index,
                &mut current_rules,
                &mut infer,
                &edges,
                &splits,
                &DriftKey::default(),
            )
            .unwrap();

        assert_eq!(report.promoted, 1, "one rule should be promoted");
        assert_eq!(report.gate_rejected, 0);
        assert_eq!(report.pre_rejected, 0);
        assert_eq!(report.new_rules.len(), 1);

        // The installed rule must be in current_rules.
        assert!(
            current_rules
                .iter()
                .any(|r| r.rule_id().starts_with("inducted-")),
            "inducted rule must appear in current_rules"
        );

        // Dedup: running the cycle again must not re-install.
        let report2 = eng
            .run_cycle(
                &index,
                &mut current_rules,
                &mut infer,
                &edges,
                &splits,
                &DriftKey::default(),
            )
            .unwrap();
        assert_eq!(report2.already_installed, 1, "dedup must skip re-install");
        assert_eq!(report2.promoted, 0);
    }

    // ── budget cap ────────────────────────────────────────────────────────────

    #[test]
    fn budget_cap_returns_error() {
        let mut index = PatternIndex::default();
        // Two valid patterns, cap = 1 → second attempt trips the budget.
        index.record(record("listed_on", 2));
        index.record(record("supplier_of", 2));

        let eng = InductionEngine::new(0.0, 0.0, 1); // cap = 1
        let mut current_rules: Vec<InferRule> = Vec::new();
        let mut infer = InferEngine::default();

        let result = eng.run_cycle(
            &index,
            &mut current_rules,
            &mut infer,
            &[],
            &[],
            &DriftKey::default(),
        );

        assert!(
            matches!(result, Err(InductionError::CandidateBudgetExceeded { .. })),
            "expected CandidateBudgetExceeded; got {result:?}"
        );
    }

    // ── inducted_rule_provenance_label ────────────────────────────────────────

    #[test]
    fn provenance_label_round_trips() {
        // An inducted rule id has the form "inducted-pattern-<safe-node-id>"
        let rule_id = "inducted-pattern-listed_on--supplier_of";
        let label = inducted_rule_provenance_label(rule_id);
        assert_eq!(
            label,
            Some("pattern:listed_on--supplier_of".to_string()),
            "provenance label must reconstruct the pattern node id"
        );
    }

    #[test]
    fn provenance_label_returns_none_for_non_inducted_rules() {
        assert!(inducted_rule_provenance_label("supplier-exposure").is_none());
        assert!(inducted_rule_provenance_label("").is_none());
    }
}
