#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Pattern → rule induction through the promote gate (knowledge-system K-R5).
//!
//! # What this crate does
//!
//! Given a mined [`PatternRecord`] (from `tdw-patterns`, knowledge-system K-R4),
//! this crate:
//!
//! 1. **Induces a candidate [`InferRule`]** whose chain matches the motif shape.
//!    A motif `"rel_a::rel_b"` induces a `DeriveEdge` rule with `when =
//!    [rel_a, rel_b]`. Induction is **deterministic**: the same `PatternRecord`
//!    always produces the same candidate rule.
//!
//! 2. **Runs the promote gate** (K-R7: [`tdw_eval_runner::replay_eval::promote_gate`])
//!    over the historical splits. A candidate that does NOT pass out-of-sample
//!    replay validation is **not promoted — ever**. This is the integrity core.
//!
//! 3. **Installs promoted rules** via [`tdw_infer::InferEngine::hot_reload`] with
//!    provenance `derived:pattern:<pattern_id>`. The rule's `rule_id` encodes the
//!    source pattern so the why-chain can trace: derived edge → induced rule →
//!    pattern → supporting instances → replay verdict.
//!
//! # Safety properties
//!
//! - **Default OFF**: [`InductionConfig::enabled`] defaults to `false`. The
//!   operator must flip it on explicitly. A loud status note is emitted on
//!   every tick when disabled.
//! - **Stratification guard**: induced rules are always assigned `stratum = 0`
//!   and their `derived_type` is checked to not equal any `when` rel (no
//!   self-recursion). Cycles across rules are caught by
//!   [`tdw_infer::InferEngine::hot_reload`]'s existing stratification validator.
//! - **No induction over induction**: the derived type carries the `inducted:`
//!   prefix, which is excluded from the motif label set during mining (the
//!   `pattern_instance_of` exclusion posture from K-R4 extended here).
//! - **Bounded candidates**: at most [`InductionConfig::max_candidates_per_cycle`]
//!   candidates are processed per cycle (hard cap, not silent truncation).
//! - **Kill-switch**: disabling [`InductionConfig::enabled`] stops new promotions.
//!   Setting [`InductionConfig::retire_induced_on_disable`] additionally retires
//!   all previously induced rules from the engine.
//!
//! # Provenance chain (why-chain)
//!
//! An edge derived by an inducted rule carries:
//! ```text
//! Provenance::Rule { rule_id: "inducted:pattern:<canonical>", version: N }
//! ```
//! The `tdw.kg.why` tool surfaces this as a `rule_derived` step, then traces
//! the rule id back to the pattern node and its supporting instances via the
//! `inducted_rule_provenance` step appended in `knowledge_explain_tools.rs`.
//!
//! # Status note (enabled = false default)
//!
//! When [`InductionConfig::enabled`] is `false` the composition root emits:
//! ```text
//! [tdw] NOTICE: K-R5 rule induction is DISABLED (knowledge.induction.enabled = false).
//!               Set enabled = true in your daemon TOML to activate self-authored rules.
//! ```

pub mod config;
pub mod induction;

pub use config::InductionConfig;
pub use induction::{
    CandidateRule, InductionCycleReport, InductionEngine, InductionError, PromotedRule,
    inducted_rule_provenance_label,
};
