//! Inference rule forms and their hot-reload validation (knowledge-system B7).
//!
//! Two monotone rule shapes — [`InferRule::DeriveEdge`] (chained edge joins) and
//! [`InferRule::PropagateTag`] (tag flow along edges) — each carrying a `rule_id` and a
//! `stratum`. Validation mirrors `tdw-tag-rules`: grammar on ids/types, bounded chain and
//! hop lengths, and STRATIFICATION — a rule may only consume a derived edge type produced
//! by a STRICTLY LOWER stratum. The strict-lower contract is what makes the fixpoint in
//! [`crate::InferEngine::run_full`] terminate.

use serde::{Deserialize, Serialize};
use tdw_core::is_graph_id;

use crate::InferError;

/// One hop in a [`InferRule::DeriveEdge`] chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgePattern {
    /// The relationship type this hop matches (graph id grammar).
    pub rel: String,
}

/// An inference rule.
///
/// Both forms are monotone (they only ADD facts), so a stratified rule set reaches a
/// fixpoint over the finite fact universe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InferRule {
    /// Derive an edge `A -derived_type-> C` from a head-to-tail chain of existing edges.
    ///
    /// `when = [r1, r2]` matches `A -r1-> B -r2-> C`; the derived edge runs from the
    /// chain's first `from` to the last `to`. Self-loops (`A == C`) are never derived.
    DeriveEdge {
        /// Stable rule identifier (identifier grammar).
        rule_id: String,
        /// Stratum: a derived-type input must come from a strictly lower stratum.
        stratum: u8,
        /// The edge chain to match, length `1..=3`, joined head-to-tail.
        when: Vec<EdgePattern>,
        /// The relationship type of the derived edge (graph id grammar).
        derived_type: String,
    },
    /// Propagate a tag from entities holding it to entities reached along given edges.
    ///
    /// Entities holding `tag` (or any taxonomy descendant when `include_descendants`)
    /// propagate the BASE `tag` to entities reached along `along` edge types in the given
    /// direction, up to `max_hops`.
    PropagateTag {
        /// Stable rule identifier (identifier grammar).
        rule_id: String,
        /// Stratum: a derived-type reference in `along` must come from a strictly lower stratum.
        stratum: u8,
        /// The base tag to propagate (tag id grammar).
        tag: String,
        /// Also seed from taxonomy descendants of `tag` (subsumption).
        include_descendants: bool,
        /// Edge types to follow, non-empty (graph id grammar).
        along: Vec<String>,
        /// Follow outgoing edges (`true`) or incoming (`false`).
        outbound: bool,
        /// Hop budget, `1..=4`.
        max_hops: u8,
    },
}

/// Lower and upper bounds on a [`InferRule::DeriveEdge`] chain length.
pub const MIN_CHAIN: usize = 1;
/// Upper bound on a [`InferRule::DeriveEdge`] chain length.
pub const MAX_CHAIN: usize = 3;
/// Upper bound on a [`InferRule::PropagateTag`] hop budget.
pub const MAX_TAG_HOPS: u8 = 4;

impl InferRule {
    /// The rule's identifier.
    #[must_use]
    pub fn rule_id(&self) -> &str {
        match self {
            Self::DeriveEdge { rule_id, .. } | Self::PropagateTag { rule_id, .. } => rule_id,
        }
    }

    /// The rule's stratum.
    #[must_use]
    pub const fn stratum(&self) -> u8 {
        match self {
            Self::DeriveEdge { stratum, .. } | Self::PropagateTag { stratum, .. } => *stratum,
        }
    }

    /// The derived edge type this rule PRODUCES, if it produces an edge.
    #[must_use]
    pub fn produced_type(&self) -> Option<&str> {
        match self {
            Self::DeriveEdge { derived_type, .. } => Some(derived_type),
            Self::PropagateTag { .. } => None,
        }
    }

    /// Every relationship type this rule CONSUMES as input.
    ///
    /// A `DeriveEdge` consumes each `when` rel; a `PropagateTag` consumes each `along`
    /// edge type. Both feed the stratification check.
    #[must_use]
    pub fn consumed_types(&self) -> Vec<&str> {
        match self {
            Self::DeriveEdge { when, .. } => {
                when.iter().map(|pattern| pattern.rel.as_str()).collect()
            }
            Self::PropagateTag { along, .. } => along.iter().map(String::as_str).collect(),
        }
    }
}

/// Shape-level validation of one rule (grammars, lengths, hop bounds, no self-recursion).
///
/// Cross-rule stratification is a SET-level check done in
/// [`super::InferEngine::hot_reload`]; this validates a single rule in isolation.
///
/// # Errors
///
/// Returns the first violation as an [`InferError`].
pub fn validate_rule(rule: &InferRule) -> Result<(), InferError> {
    if !is_identifier(rule.rule_id()) {
        return Err(InferError::InvalidRule {
            rule_id: rule.rule_id().to_string(),
            reason: "rule_id grammar",
        });
    }
    match rule {
        InferRule::DeriveEdge {
            rule_id,
            when,
            derived_type,
            ..
        } => {
            if when.len() < MIN_CHAIN || when.len() > MAX_CHAIN {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "chain length must be 1..=3",
                });
            }
            for pattern in when {
                if !is_graph_id(&pattern.rel) {
                    return Err(InferError::InvalidRule {
                        rule_id: rule_id.clone(),
                        reason: "when rel grammar",
                    });
                }
            }
            if !is_graph_id(derived_type) {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "derived_type grammar",
                });
            }
            // A rule that derives a type one of its OWN patterns consumes would
            // feed itself — self-recursion the stratification argument cannot
            // bound. Reject it outright.
            if when.iter().any(|pattern| &pattern.rel == derived_type) {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "rule consumes the type it derives (self-recursion)",
                });
            }
        }
        InferRule::PropagateTag {
            rule_id,
            tag,
            along,
            max_hops,
            ..
        } => {
            if !is_tag_id(tag) {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "tag grammar",
                });
            }
            if along.is_empty() {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "along must be non-empty",
                });
            }
            for edge_type in along {
                if !is_graph_id(edge_type) {
                    return Err(InferError::InvalidRule {
                        rule_id: rule_id.clone(),
                        reason: "along edge-type grammar",
                    });
                }
            }
            if *max_hops < 1 || *max_hops > MAX_TAG_HOPS {
                return Err(InferError::InvalidRule {
                    rule_id: rule_id.clone(),
                    reason: "max_hops must be 1..=4",
                });
            }
        }
    }
    Ok(())
}

/// The rule-id / kind identifier grammar (matches `tdw-tag-rules`).
fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

/// The tag-id grammar (matches `tdw-tags`): non-empty, contains `:`.
fn is_tag_id(value: &str) -> bool {
    !value.is_empty()
        && value.contains(':')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derive(rule_id: &str, stratum: u8, rels: &[&str], derived: &str) -> InferRule {
        InferRule::DeriveEdge {
            rule_id: rule_id.to_string(),
            stratum,
            when: rels
                .iter()
                .map(|rel| EdgePattern {
                    rel: (*rel).to_string(),
                })
                .collect(),
            derived_type: derived.to_string(),
        }
    }

    #[test]
    fn validates_chain_bounds_and_grammar() {
        assert!(validate_rule(&derive("ok", 0, &["a", "b"], "c")).is_ok());
        assert!(matches!(
            validate_rule(&derive("ok", 0, &[], "c")),
            Err(InferError::InvalidRule { .. })
        ));
        assert!(matches!(
            validate_rule(&derive("ok", 0, &["a", "b", "c", "d"], "e")),
            Err(InferError::InvalidRule { .. })
        ));
        assert!(matches!(
            validate_rule(&derive("ok", 0, &["a/bad"], "c")),
            Err(InferError::InvalidRule { .. })
        ));
        assert!(matches!(
            validate_rule(&derive("bad/id", 0, &["a"], "c")),
            Err(InferError::InvalidRule { .. })
        ));
    }

    #[test]
    fn rejects_self_recursive_derive() {
        // when consumes "loop" and derived_type is also "loop".
        assert!(matches!(
            validate_rule(&derive("r", 0, &["loop", "x"], "loop")),
            Err(InferError::InvalidRule { .. })
        ));
    }

    fn propagate(tag: &str, along: &[&str], max_hops: u8) -> InferRule {
        InferRule::PropagateTag {
            rule_id: "p".to_string(),
            stratum: 0,
            tag: tag.to_string(),
            include_descendants: true,
            along: along.iter().map(|edge| (*edge).to_string()).collect(),
            outbound: true,
            max_hops,
        }
    }

    #[test]
    fn validates_propagate_tag_bounds() {
        assert!(validate_rule(&propagate("risk:sanctioned", &["owned_by"], 2)).is_ok());
        assert!(matches!(
            validate_rule(&propagate("risk:sanctioned", &[], 2)),
            Err(InferError::InvalidRule { .. })
        ));
        assert!(matches!(
            validate_rule(&propagate("risk:sanctioned", &["owned_by"], 5)),
            Err(InferError::InvalidRule { .. })
        ));
        assert!(matches!(
            validate_rule(&propagate("notag", &["owned_by"], 2)),
            Err(InferError::InvalidRule { .. })
        ));
    }
}
