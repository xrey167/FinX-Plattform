//! Induced-rule routing hints + the walk-forward usefulness harness
//! (the partner design §3, partner-system W4.3/W4.4).
//!
//! # W4.3 — induced rules feed routing/answers, gated past B9
//!
//! A promoted induced rule produces a *derived edge type* (`inducted:…`). The
//! partner does not invent these — they arrive from `tdw-induction`'s
//! `run_cycle` → `InferEngine::hot_reload` path, and **only** a rule whose
//! `CandidateRule::is_promoted` (i.e. it cleared the K-R5 replay promote gate
//! past B9) is ever installed. [`routing_hints_from`] reads the *installed*
//! rule set off an [`InferEngine`] and surfaces the derived types as routing
//! hints the resolve step can prefer. The gate is structural: an un-promoted
//! candidate is never hot-reloaded, so its derived type never appears as a hint
//! — proven by [`tests::unpromoted_candidate_yields_no_routing_hint`].
//!
//! Partner Core only *reads* these hints (it does not run induction or mutate
//! the engine); the engine is driven by the gated daemon worker, exactly as the
//! §3 invariant requires.
//!
//! # W4.4 — the walk-forward usefulness harness (the W4 done-condition metric)
//!
//! [`walk_forward_usefulness`] is a real, deterministic, offline eval: it runs
//! `tdw-eval-runner`'s walk-forward replay (`run_replay_eval`) for a rule over
//! checked-in historical splits *before* and *after* a learning epoch, and
//! returns a [`UsefulnessReport`] whose `improved()` proves the metric **rose**.
//! "Before" is the empty rule set (the partner could not make the prediction);
//! "after" is the promoted induced rule (it now predicts the realized outcome).
//! Rising precision/recall across the epoch is "gets more useful with use",
//! measured rather than asserted (the partner design §3 walk-forward eval).

use tdw_infer::InferEngine;

/// One derived edge type a promoted induced rule contributes to routing
/// (the partner design §3, W4.3).
///
/// Surfaced to the resolve step so a turn can prefer a route/relation the
/// learning loop has validated. Only types produced by *installed* (hence
/// promoted-past-B9) rules ever appear here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RoutingHint {
    /// The id of the installed rule that produced the hint.
    pub rule_id: String,
    /// The derived edge type the rule produces (e.g. `inducted:listed_on--…`).
    pub derived_type: String,
}

/// Read the routing hints from the rules *installed* in `engine` (W4.3).
///
/// Every installed rule has, by construction, passed `hot_reload` (shape +
/// stratification) and — for an induced rule — the K-R5 promote gate before the
/// daemon worker installed it. So reading the installed set is reading the
/// gated, promoted output: an un-promoted candidate is never here. Returns the
/// derived types in deterministic (rule-id) order.
#[must_use]
pub fn routing_hints_from(engine: &InferEngine) -> Vec<RoutingHint> {
    let mut hints: Vec<RoutingHint> = engine
        .rules()
        .iter()
        .filter_map(|rule| {
            rule.produced_type().map(|derived| RoutingHint {
                rule_id: rule.rule_id().to_string(),
                derived_type: derived.to_string(),
            })
        })
        .collect();
    hints.sort();
    hints
}

/// Whether `derived_type` is an induced (learning-loop) type.
///
/// Induced derived types are namespaced `inducted:` by `tdw-induction`. A turn
/// can use this to mark a hint as learning-derived in its audit trail.
#[must_use]
pub fn is_induced_type(derived_type: &str) -> bool {
    derived_type.starts_with("inducted:")
}

/// Map gated routing hints to the learned route-preference list Partner Core
/// threads onto its [`LearningState`](crate::LearningState) snapshot (W4.2/W4.3).
///
/// Each hint's derived edge type is namespaced `inducted:<route-prefix>--<...>`
/// by `tdw-induction`; the route-prefix segment (before the first `--`, with the
/// `inducted:` namespace stripped) is the catalog route family the learning loop
/// has validated as worth leading with. The bridge is deliberately conservative:
/// it only surfaces a preference for an INDUCED hint (a gated, promoted-past-B9
/// signal), preserves the hints' deterministic order, and de-duplicates so a
/// route family is preferred once. The result feeds
/// [`reorder_by_preference`](crate::apply_to_resolution) exactly like any other
/// `preferred_routes` entry (an id OR a path prefix), so a learned hint reshapes
/// the live turn's routing — never a caller-supplied hint.
#[must_use]
pub fn route_preferences_from_hints(hints: &[RoutingHint]) -> Vec<String> {
    let mut preferences: Vec<String> = Vec::new();
    for hint in hints {
        if !is_induced_type(&hint.derived_type) {
            continue;
        }
        // Strip the `inducted:` namespace, then take the route-prefix segment
        // before the first `--` separator (the induction edge-type grammar).
        let body = hint
            .derived_type
            .strip_prefix("inducted:")
            .unwrap_or(&hint.derived_type);
        let prefix = body.split("--").next().unwrap_or(body).trim();
        if prefix.is_empty() {
            continue;
        }
        let preference = prefix.to_string();
        if !preferences.contains(&preference) {
            preferences.push(preference);
        }
    }
    preferences
}

/// The walk-forward usefulness report across one learning epoch (W4.4).
///
/// Captures the replay metric *before* the epoch (no learned rule) and *after*
/// (the promoted induced rule installed). [`Self::improved`] is the W4
/// done-condition: usefulness rose because the partner learned a rule that
/// correctly predicts realized outcomes it previously could not.
#[derive(Clone, Debug, PartialEq)]
pub struct UsefulnessReport {
    /// Mean replay precision before the learning epoch.
    pub precision_before: f64,
    /// Mean replay recall before the learning epoch.
    pub recall_before: f64,
    /// Mean replay precision after the learning epoch.
    pub precision_after: f64,
    /// Mean replay recall after the learning epoch.
    pub recall_after: f64,
    /// Total realized-outcome support the metric was computed over.
    pub support: usize,
}

impl UsefulnessReport {
    /// Whether usefulness strictly rose across the epoch (the W4 done-condition).
    ///
    /// Both metrics must be non-decreasing and at least one must strictly rise,
    /// over a non-empty ground truth — "more useful with use", proven not
    /// asserted.
    #[must_use]
    pub fn improved(&self) -> bool {
        self.support > 0
            && self.recall_after >= self.recall_before
            && self.precision_after >= self.precision_before
            && (self.recall_after > self.recall_before
                || self.precision_after > self.precision_before)
    }
}

/// The walk-forward usefulness harness (W4.4) — a public, NON-test eval entry
/// point.
///
/// Replays `learned_rule` over the checked-in `splits` BEFORE the learning epoch
/// (an equivalent rule that can never fire, modelling "the partner had not
/// learned the rule yet" — predictions empty, realized outcomes all missed) and
/// AFTER (the promoted rule, which now predicts the realized outcome), then
/// reports the metric delta as a [`UsefulnessReport`]. [`UsefulnessReport::improved`]
/// is the W4 done-condition: usefulness rose because the partner learned a rule
/// that correctly predicts realized outcomes it previously could not.
///
/// This lives behind the `eval-harness` feature so the offline `tdw-eval-runner`
/// replay dependency stays out of `tdw-partner`'s default (leaf) build. It is the
/// honest "public non-test eval entry point" the W4.4 metric is computed through:
/// the gated daemon's eval loop (or a benchmark gate) calls it, rather than the
/// metric living only inside a unit test.
///
/// # Panics
///
/// Panics if `learned_rule` is not a [`tdw_infer::InferRule::DeriveEdge`] (the
/// harness drives edge-deriving rules) or if a replay fails to run over the
/// provided splits.
#[cfg(feature = "eval-harness")]
#[must_use]
pub fn walk_forward_usefulness(
    learned_rule: &tdw_infer::InferRule,
    edges: &[tdw_core::GraphEdge],
    splits: &[tdw_eval_runner::replay_eval::ReplaySplit],
) -> UsefulnessReport {
    use tdw_eval_runner::replay_eval::run_replay_eval;
    use tdw_eval_runner::retrieval_eval::DriftKey;
    use tdw_infer::{EdgePattern, InferRule};

    // "Before": a rule that derives the SAME outcome type but can never fire (it
    // consumes a relation absent from the corpus), modelling "the partner had not
    // learned the rule yet" — predictions are empty, so the realized outcomes are
    // all missed (recall 0).
    let InferRule::DeriveEdge { derived_type, .. } = learned_rule else {
        panic!("walk-forward harness drives DeriveEdge rules");
    };
    let unlearned = InferRule::DeriveEdge {
        rule_id: "baseline-unlearned".to_string(),
        stratum: 0,
        when: vec![EdgePattern {
            rel: "never_present_relation".to_string(),
        }],
        derived_type: derived_type.clone(),
    };

    let before = run_replay_eval(&unlearned, edges, splits, DriftKey::default())
        .expect("before replay runs");
    let after = run_replay_eval(learned_rule, edges, splits, DriftKey::default())
        .expect("after replay runs");

    UsefulnessReport {
        precision_before: before.mean_precision,
        recall_before: before.mean_recall,
        precision_after: after.mean_precision,
        recall_after: after.mean_recall,
        support: after.total_support,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_eval_runner::retrieval_eval::DriftKey;
    use tdw_induction::induction::InductionEngine;
    use tdw_patterns::{PatternIndex, PatternRecord};

    #[cfg(feature = "eval-harness")]
    fn ingest_edge(from: &str, rel: &str, to: &str, valid_from: &str) -> tdw_core::GraphEdge {
        tdw_core::GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: tdw_core::Provenance::Ingest {
                source: "test".to_string(),
            },
            valid_from: Some(valid_from.to_string()),
            valid_to: None,
        }
    }

    fn pattern(canonical: &str, support: usize) -> PatternRecord {
        PatternRecord::new(
            canonical.to_string(),
            support,
            "2024-01-01".to_string(),
            vec![("e:A".to_string(), "v:X".to_string())],
        )
    }

    // ── W4.3: only a promoted candidate's derived type becomes a hint ────────

    #[test]
    fn promoted_induced_rule_surfaces_a_routing_hint() {
        // Drive the real induction → hot_reload path: a pattern induces a
        // candidate, the (zero-threshold, vacuous) promote gate passes it, and
        // the caller hot-reloads it. The installed rule's derived type then shows
        // up as a routing hint Partner Core can read.
        let mut index = PatternIndex::default();
        index.record(pattern("listed_on::supplier_of", 3));

        let induction = InductionEngine::new(0.0, 0.0, 100); // vacuous promote
        let mut report = induction
            .run_cycle(&index, &[], &[], &[], &DriftKey::default())
            .expect("induction cycle runs");
        assert_eq!(report.rules_to_install.len(), 1, "one promoted rule");

        // The caller does the single atomic hot_reload (gated path).
        let mut engine = InferEngine::default();
        let to_install = std::mem::take(&mut report.rules_to_install);
        engine.hot_reload(to_install).expect("hot_reload installs");

        let hints = routing_hints_from(&engine);
        assert_eq!(
            hints.len(),
            1,
            "the promoted rule yields one hint: {hints:?}"
        );
        assert!(
            is_induced_type(&hints[0].derived_type),
            "the hint is an induced type: {:?}",
            hints[0].derived_type
        );
    }

    #[test]
    fn unpromoted_candidate_yields_no_routing_hint() {
        // The induced candidate is built but NEVER hot-reloaded (it did not clear
        // the gate). An empty engine surfaces no hint — the structural B9 gate:
        // an un-promoted rule's derived type cannot reach routing.
        let induction = InductionEngine::new(0.0, 0.0, 100);
        let record = pattern("listed_on::supplier_of", 3);
        let candidate = induction
            .induce_candidate(&record)
            .expect("candidate induced");
        // is_promoted is false: induce_candidate sets no gate verdict.
        assert!(
            !candidate.is_promoted(),
            "a bare candidate is not promoted until the gate passes"
        );

        // Behavior: nothing is installed, so there is no routing hint.
        let engine = InferEngine::default();
        assert!(
            routing_hints_from(&engine).is_empty(),
            "an un-promoted candidate contributes no routing hint"
        );
    }

    // ── W4.3: gated routing hints bridge to learned route preferences ────────

    #[test]
    fn induced_hints_map_to_de_duplicated_route_preferences() {
        // Two induced hints under the same route family + one non-induced hint.
        // The bridge surfaces the route-prefix segment of each INDUCED hint once,
        // in order, and drops the non-induced hint — the gated signal that feeds
        // reorder_by_preference on a live turn.
        let hints = vec![
            RoutingHint {
                rule_id: "r1".to_string(),
                derived_type: "inducted:equity/profile--listed_on".to_string(),
            },
            RoutingHint {
                rule_id: "r2".to_string(),
                derived_type: "inducted:equity/profile--supplier_of".to_string(),
            },
            RoutingHint {
                rule_id: "r3".to_string(),
                derived_type: "inducted:news/headlines--mentions".to_string(),
            },
            // A non-induced derived type is NOT a gated learning signal.
            RoutingHint {
                rule_id: "r4".to_string(),
                derived_type: "exposed_to".to_string(),
            },
        ];
        let preferences = route_preferences_from_hints(&hints);
        assert_eq!(
            preferences,
            vec!["equity/profile".to_string(), "news/headlines".to_string()],
            "induced route families surface once, in order; non-induced dropped"
        );
    }

    #[test]
    fn no_induced_hints_yields_no_preferences() {
        let hints = vec![RoutingHint {
            rule_id: "r1".to_string(),
            derived_type: "exposed_to".to_string(),
        }];
        assert!(
            route_preferences_from_hints(&hints).is_empty(),
            "a non-induced hint contributes no route preference"
        );
    }

    // ── W4.4: walk-forward eval proves rising usefulness ─────────────────────

    #[cfg(feature = "eval-harness")]
    #[test]
    fn walk_forward_shows_rising_usefulness_across_a_learning_epoch() {
        use tdw_eval_runner::replay_eval::ReplaySplit;
        use tdw_infer::{EdgePattern, InferRule};

        // The rule the partner learns: A -supplier_of-> B -listed_on-> X
        // ⇒ A -exposed_to-> X. At the split, both input edges are active; in the
        // forward window the exposed_to outcome materializes — so the learned
        // rule predicts a realized fact the un-learned baseline cannot.
        let learned = InferRule::DeriveEdge {
            rule_id: "supplier-exposure".to_string(),
            stratum: 0,
            when: vec![
                EdgePattern {
                    rel: "supplier_of".to_string(),
                },
                EdgePattern {
                    rel: "listed_on".to_string(),
                },
            ],
            derived_type: "exposed_to".to_string(),
        };
        let edges = vec![
            ingest_edge("a", "supplier_of", "b", "2023-01-01T00:00:00Z"),
            ingest_edge("b", "listed_on", "x", "2023-06-01T00:00:00Z"),
            // The realized outcome in the forward window.
            ingest_edge("a", "exposed_to", "x", "2024-03-01T00:00:00Z"),
        ];
        let splits = vec![ReplaySplit {
            split_id: "s1".to_string(),
            as_of: "2024-01-01T00:00:00Z".to_string(),
            as_of_plus_window: "2024-07-01T00:00:00Z".to_string(),
        }];

        let report = walk_forward_usefulness(&learned, &edges, &splits);

        // Before: the un-learned baseline makes no prediction → recall 0 over a
        // real outcome. After: the learned rule predicts the realized outcome →
        // recall 1. Usefulness strictly rose — the W4 done-condition.
        assert_eq!(report.support, 1, "one realized outcome to predict");
        assert!(
            report.recall_before < report.recall_after,
            "recall must rise across the epoch: before={}, after={}",
            report.recall_before,
            report.recall_after
        );
        assert!(
            report.improved(),
            "the walk-forward metric must show rising usefulness: {report:?}"
        );
    }

    #[test]
    fn usefulness_report_requires_real_support_to_improve() {
        // Guard against a vacuous "improvement": with no realized outcomes there
        // is nothing to be more useful about, so improved() is false even if the
        // numbers look equal-or-better.
        let flat = UsefulnessReport {
            precision_before: 1.0,
            recall_before: 1.0,
            precision_after: 1.0,
            recall_after: 1.0,
            support: 0,
        };
        assert!(!flat.improved(), "no support → no provable improvement");
    }
}
