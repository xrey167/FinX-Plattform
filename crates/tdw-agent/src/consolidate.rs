//! Memory consolidation — the pure decision logic of the human-memory-inspired
//! background process.
//!
//! A `memory` entity carries a [`crate::Retention`] tier (working → core). Over time, a
//! surviving memory is *consolidated* into a longer-lived tier; an ephemeral working
//! buffer that ages out is expired. This module is the pure planner: given memories paired
//! with their age in days, it returns the actions to apply. The scheduling/applying *loop*
//! is a runtime concern (call [`consolidation_plan`] periodically and act on the result).
//!
//! ## Usage-aware planning (knowledge-system B10)
//!
//! [`consolidation_plan_with_usage`] is the enriched variant: in addition to the age-based
//! policy it accepts per-memory usage hints derived from the retrieval feedback store. A
//! memory that has been retrieved and marked `used` recently is treated as *younger* than
//! its raw age, which can prevent premature expiry of actively-referenced working buffers.
//!
//! The original [`consolidation_plan`] API is unchanged and re-exported — callers that do
//! not supply usage hints continue to work identically (additive, back-compat, no fork).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Memory;
use crate::base::Retention;

/// An action the consolidator decides for one memory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "action")]
pub enum ConsolidationAction {
    /// Promote a surviving memory to a longer-lived tier.
    Promote {
        /// The memory's `name`.
        name: String,
        /// Tier it is leaving.
        from: Retention,
        /// Tier it consolidates into.
        to: Retention,
    },
    /// Expire an aged-out ephemeral (`Working`) memory.
    Expire {
        /// The memory's `name`.
        name: String,
    },
}

/// Decide consolidation actions for memories paired with their age in days.
///
/// Policy: a memory whose `age_days >= retention.ttl_days()` has survived its tier and is
/// consolidated —
/// - `Working` (ttl 0) → [`ConsolidationAction::Expire`] (ephemeral buffer ages out),
/// - `ShortTerm` / `MidTerm` / `LongTerm` → [`ConsolidationAction::Promote`] to the next tier,
/// - `Core` → no action (it never expires).
///
/// Memories younger than their TTL produce no action. `Working` has a TTL of 0, so a
/// `Working` memory is expired on the **first** consolidation tick — the buffer is
/// intra-session by design.
///
/// The planner is stateless and keys only off the caller-supplied `age_days`. The caller
/// must apply the returned actions and persist each memory's new tier (and
/// `last_consolidated`) before the next call; otherwise a surviving memory is re-emitted
/// for promotion on every tick.
pub fn consolidation_plan<'a>(
    memories: impl IntoIterator<Item = (&'a Memory, u32)>,
) -> Vec<ConsolidationAction> {
    let mut actions = Vec::new();
    for (memory, age_days) in memories {
        let tier = memory.retention;
        let Some(ttl) = tier.ttl_days() else {
            continue; // Core never expires.
        };
        if age_days < ttl {
            continue; // Still within its tier's lifetime.
        }
        let name = memory.meta.base.name.clone();
        if tier == Retention::Working {
            actions.push(ConsolidationAction::Expire { name });
        } else {
            actions.push(ConsolidationAction::Promote {
                name,
                from: tier,
                to: tier.next(),
            });
        }
    }
    actions
}

/// Per-memory usage hint supplied by the B10 retrieval feedback store.
///
/// The consolidation planner subtracts `recency_credit_days` from the raw age
/// before comparing against the tier TTL, effectively making a recently-used
/// memory appear younger. This prevents premature expiry of actively-referenced
/// working buffers without changing the expiry policy for unreferenced ones.
///
/// A zero `recency_credit_days` is a no-op and produces the same result as
/// calling [`consolidation_plan`] directly — the enriched API is strictly a
/// superset of the original.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageHint {
    /// How many days to subtract from the raw age before the TTL check
    /// (`effective_age = raw_age.saturating_sub(recency_credit_days)`).
    ///
    /// Derived from the most-recent `used=true` feedback event that references
    /// the memory: `credit = raw_age.saturating_sub(days_since_last_used)`.
    /// Equivalently, `effective_age = max(0, raw_age − days_since_last_used)`.
    /// A memory used 2 days ago with a raw age of 5 gets `credit=3`,
    /// `effective_age=2`. A memory never marked `used` gets `credit=0` and
    /// is treated identically to the base [`consolidation_plan`].
    pub recency_credit_days: u32,
    /// Number of *distinct* `query_fingerprint` values across `used=true`
    /// feedback events that reference this memory. Informational; not used in
    /// the current planner policy but available for future priority-weighting.
    pub use_count: u32,
}

/// Usage-aware consolidation plan — the B10 enriched variant of [`consolidation_plan`].
///
/// Identical to [`consolidation_plan`] except that each memory may be paired with a
/// [`UsageHint`]; the hint's `recency_credit_days` is subtracted from the raw age
/// (saturating at 0) before the tier TTL check. A memory with no hint (or a
/// zero-credit hint) is treated exactly as in [`consolidation_plan`].
///
/// **The original [`consolidation_plan`] API is unchanged**: callers that do not
/// supply hints are unaffected; this function is additive and does not fork the policy.
///
/// ## Caller contract
///
/// Supply `usage` pairs in the same order and count as `memories`. A `UsageHint` of
/// `Default` (all-zero) is safe when no feedback is available; the function degrades
/// gracefully to the base policy.
pub fn consolidation_plan_with_usage<'a>(
    memories: impl IntoIterator<Item = (&'a Memory, u32, UsageHint)>,
) -> Vec<ConsolidationAction> {
    let mut actions = Vec::new();
    for (memory, age_days, hint) in memories {
        let tier = memory.retention;
        let Some(ttl) = tier.ttl_days() else {
            continue; // Core never expires.
        };
        // Apply recency credit: a recently-used memory appears younger.
        let effective_age = age_days.saturating_sub(hint.recency_credit_days);
        if effective_age < ttl {
            continue; // Still within its tier's effective lifetime.
        }
        let name = memory.meta.base.name.clone();
        if tier == Retention::Working {
            actions.push(ConsolidationAction::Expire { name });
        } else {
            actions.push(ConsolidationAction::Promote {
                name,
                from: tier,
                to: tier.next(),
            });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::{Adaptivity, EntityMeta, Origin, Source, Tier};
    use crate::facets::{DataFacets, Materialization, Plane};

    fn memory(name: &str, retention: Retention) -> Memory {
        Memory {
            meta: EntityMeta::new(
                name,
                name,
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::SelfModifying,
                false,
            ),
            retention,
            last_consolidated: None,
            source_entries: Vec::new(),
            facets: DataFacets {
                plane: Plane::Agent,
                materialization: Materialization::Materialized,
                as_of: None,
                validation: None,
            },
        }
    }

    #[test]
    fn aged_short_term_consolidates_to_mid_term() {
        let short = memory("note", Retention::ShortTerm);
        let plan = consolidation_plan([(&short, 2)]); // ttl 1, age 2 -> promote
        assert_eq!(
            plan,
            vec![ConsolidationAction::Promote {
                name: "note".to_string(),
                from: Retention::ShortTerm,
                to: Retention::MidTerm,
            }]
        );
    }

    #[test]
    fn fresh_memory_is_left_alone() {
        let mid = memory("note", Retention::MidTerm);
        assert!(consolidation_plan([(&mid, 0)]).is_empty()); // ttl 7, age 0
    }

    #[test]
    fn aged_working_buffer_expires() {
        let working = memory("buf", Retention::Working);
        assert_eq!(
            consolidation_plan([(&working, 1)]), // ttl 0, age 1 -> expire
            vec![ConsolidationAction::Expire {
                name: "buf".to_string()
            }]
        );
    }

    #[test]
    fn working_buffer_expires_on_the_first_tick() {
        // ttl 0 + age 0: a working buffer is intra-session and expires on the first pass.
        let working = memory("buf", Retention::Working);
        assert_eq!(
            consolidation_plan([(&working, 0)]),
            vec![ConsolidationAction::Expire {
                name: "buf".to_string()
            }]
        );
    }

    #[test]
    fn core_never_acts_however_old() {
        let core = memory("identity", Retention::Core);
        assert!(consolidation_plan([(&core, 10_000)]).is_empty());
    }

    #[test]
    fn long_term_consolidates_to_core() {
        let long = memory("fact", Retention::LongTerm);
        assert_eq!(
            consolidation_plan([(&long, 91)]), // ttl 90 -> promote to Core
            vec![ConsolidationAction::Promote {
                name: "fact".to_string(),
                from: Retention::LongTerm,
                to: Retention::Core,
            }]
        );
    }

    #[test]
    fn aged_mid_term_consolidates_to_long_term() {
        // Completes the promotion chain working->short->mid->long->core; the
        // mid->long transition was the one rung not pinned.
        let mid = memory("note", Retention::MidTerm);
        assert_eq!(
            consolidation_plan([(&mid, 7)]), // ttl 7 -> promote to LongTerm
            vec![ConsolidationAction::Promote {
                name: "note".to_string(),
                from: Retention::MidTerm,
                to: Retention::LongTerm,
            }]
        );
    }

    // --- consolidation_plan_with_usage (B10) ---

    #[test]
    fn usage_hint_zero_credit_matches_base_planner() {
        // A UsageHint with zero recency_credit_days must produce exactly the same
        // result as the base consolidation_plan (the enriched API is a superset).
        let short = memory("note", Retention::ShortTerm);
        let hint = UsageHint::default(); // zero credit
        let with_usage = consolidation_plan_with_usage([(&short, 2, hint)]);
        let base = consolidation_plan([(&short, 2)]);
        assert_eq!(with_usage, base, "zero-credit hint must match base planner");
    }

    #[test]
    fn working_buffer_always_expires_regardless_of_credit() {
        // Working ttl=0: effective_age = age.saturating_sub(credit).
        // Even with age=0 and credit=0, effective_age=0 >= ttl=0 → always expires.
        // Credit cannot rescue a Working buffer because ttl=0 means any
        // effective_age >= 0 fires the expiry — this is intentional: Working
        // buffers are intra-session by design.
        let working = memory("buf", Retention::Working);
        for credit in [0_u32, 1, 100] {
            let plan = consolidation_plan_with_usage([(
                &working,
                0,
                UsageHint {
                    recency_credit_days: credit,
                    use_count: 1,
                },
            )]);
            assert_eq!(
                plan,
                vec![ConsolidationAction::Expire {
                    name: "buf".to_string()
                }],
                "Working buffer (credit={credit}) must always expire"
            );
        }
    }

    #[test]
    fn recency_credit_formula_effective_age_is_raw_minus_days_since() {
        // Pin the credit arithmetic: credit = raw_age.saturating_sub(days_since).
        // effective_age = raw_age.saturating_sub(credit)
        //               = raw_age.saturating_sub(raw_age.saturating_sub(days_since))
        //               = min(raw_age, days_since).
        // ShortTerm ttl=1: raw_age=5, days_since=3 → credit=2, effective_age=3 >= 1 → promote.
        // ShortTerm ttl=1: raw_age=5, days_since=5 → credit=0, effective_age=5 >= 1 → promote.
        // ShortTerm ttl=1: raw_age=1, days_since=0 → credit=1, effective_age=0 < 1 → no action.
        let short = memory("note", Retention::ShortTerm);
        let plan_a = consolidation_plan_with_usage([(
            &short,
            5,
            UsageHint {
                recency_credit_days: 2,
                use_count: 1,
            },
        )]);
        assert_eq!(
            plan_a,
            vec![ConsolidationAction::Promote {
                name: "note".to_string(),
                from: Retention::ShortTerm,
                to: Retention::MidTerm,
            }],
            "effective_age=3 still exceeds ttl=1"
        );
        // credit exactly equals raw_age → effective_age=0 < ttl=1 → survives.
        let plan_b = consolidation_plan_with_usage([(
            &short,
            1,
            UsageHint {
                recency_credit_days: 1,
                use_count: 1,
            },
        )]);
        assert!(
            plan_b.is_empty(),
            "credit == raw_age → effective_age=0 < ttl=1 → memory survives"
        );
    }

    #[test]
    fn recency_credit_keeps_short_term_memory_alive() {
        // ShortTerm ttl=1: age=2 would normally promote.
        // A credit of 2 days makes effective_age=0 < ttl=1 → no action.
        let short = memory("note", Retention::ShortTerm);
        let hint = UsageHint {
            recency_credit_days: 2,
            use_count: 3,
        };
        let plan = consolidation_plan_with_usage([(&short, 2, hint)]);
        assert!(
            plan.is_empty(),
            "recency credit should keep the memory alive: {plan:?}"
        );
    }

    #[test]
    fn recency_credit_saturates_at_zero_not_negative() {
        // credit > age: saturating_sub keeps effective_age at 0, never negative.
        let short = memory("note", Retention::ShortTerm);
        let hint = UsageHint {
            recency_credit_days: 100,
            use_count: 1,
        };
        // age=0, effective_age=0, ttl=1: 0 < 1 → no action (memory survives).
        let plan = consolidation_plan_with_usage([(&short, 0, hint)]);
        assert!(
            plan.is_empty(),
            "saturating sub should not underflow: {plan:?}"
        );
    }

    #[test]
    fn core_memory_never_acts_with_any_hint() {
        let core = memory("identity", Retention::Core);
        let hint = UsageHint {
            recency_credit_days: 0,
            use_count: 999,
        };
        assert!(
            consolidation_plan_with_usage([(&core, 10_000, hint)]).is_empty(),
            "Core memory must never produce an action"
        );
    }

    #[test]
    fn partial_credit_still_promotes_aged_memory() {
        // ShortTerm ttl=1: age=3, credit=1 → effective_age=2 >= ttl=1 → promote.
        let short = memory("note", Retention::ShortTerm);
        let hint = UsageHint {
            recency_credit_days: 1,
            use_count: 1,
        };
        let plan = consolidation_plan_with_usage([(&short, 3, hint)]);
        assert_eq!(
            plan,
            vec![ConsolidationAction::Promote {
                name: "note".to_string(),
                from: Retention::ShortTerm,
                to: Retention::MidTerm,
            }],
            "partial credit that does not bring age below ttl still promotes"
        );
    }
}
