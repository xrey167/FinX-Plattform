//! Memory consolidation — the pure decision logic of the human-memory-inspired
//! background process.
//!
//! A `memory` entity carries a [`crate::Retention`] tier (working → core). Over time, a
//! surviving memory is *consolidated* into a longer-lived tier; an ephemeral working
//! buffer that ages out is expired. This module is the pure planner: given memories paired
//! with their age in days, it returns the actions to apply. The scheduling/applying *loop*
//! is a runtime concern (call [`consolidation_plan`] periodically and act on the result).

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
}
