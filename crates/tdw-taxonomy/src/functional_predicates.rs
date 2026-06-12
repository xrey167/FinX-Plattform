//! Functional-predicate metadata for contradiction-driven temporal invalidation
//! (knowledge-system K-M4).
//!
//! A **functional predicate** (in the database-theory sense) is a relation
//! where, at any given point in time, a subject node has **at most one current
//! object**.  When a new fact `(S, R, O2)` arrives for a functional `R` and an
//! active fact `(S, R, O1)` already exists with `O1 ≠ O2`, the old fact is
//! **superseded**: its validity window is closed at the new fact's arrival
//! timestamp.  History is preserved — a point-in-time query before the close
//! still returns `O1`; the current view returns `O2`.
//!
//! # Taxonomy-level defaults
//!
//! The set below enumerates relations that are *structurally* functional in the
//! platform's domain model.  These defaults are compiled in and need no config;
//! they reflect widely accepted domain semantics (a company has exactly one
//! current CEO, a security has one primary listing exchange at a time, etc.).
//!
//! | Predicate           | Rationale                                                   |
//! |---------------------|-------------------------------------------------------------|
//! | `ceo_of`            | A person is CEO of one company at a time (per subject)     |
//! | `cfo_of`            | Same for CFO                                               |
//! | `primary_exchange`  | An instrument has one primary exchange at a time           |
//! | `domicile`          | A legal entity has one domicile at a time                  |
//! | `fiscal_year_end`   | A company has one fiscal-year-end month at a time          |
//! | `sector`            | Primary sector classification (one per entity at a time)   |
//! | `industry`          | Primary industry classification                            |
//! | `currency`          | Reporting/trading currency (one per entity at a time)      |
//! | `parent_company`    | Ultimate parent (one at a time — spin-offs create new edges)|
//! | `credit_rating`     | Issuer credit rating from a given agency (per issuer)      |
//!
//! # Operator extension
//!
//! Operators extend the set via `[knowledge.contradiction]` in their daemon
//! TOML by listing additional relation names in `extra_functional_rels`.
//! Relation names must satisfy the graph-id grammar (`[A-Za-z0-9:._-]+`).
//! Unknown names are **loudly rejected at config validation** — a typo in a
//! functional-predicate list would silently skip invalidation, which is harder
//! to debug than an explicit error.
//!
//! [`FunctionalPredicateSet::from_config`] merges the taxonomy defaults with
//! the operator extension.  The result is an immutable, hash-indexed set used
//! by the contradiction detector hot-path.
//!
//! # Trust-class matrix — what IS and IS NOT auto-invalidated
//!
//! | Edge provenance                      | Auto-invalidated? | Rationale                                         |
//! |--------------------------------------|-------------------|---------------------------------------------------|
//! | `Provenance::Ingest { .. }`          | **Yes**           | Feed facts supersede prior feed facts             |
//! | `Provenance::Rule { .. }`            | **No** (skip)     | Derived edges are handled by B7 retraction, not   |
//! |                                      |                   | temporal invalidation — `retract()` is the right    |
//! |                                      |                   | primitive for derived edges (double-touch risk)   |
//! | `Provenance::Agent { gated: false }` | **No — surfaced** | User findings are never auto-invalidated; a       |
//! |  (user finding)                      | as conflict       | contradiction is emitted as a loud log + conflict |
//! |                                      |                   | counter so the analyst can review                 |
//! | `Provenance::Agent { gated: true }`  | **Yes**           | Gated/operator-approved agent writes follow the   |
//! |  (operator-gated agent write)        |                   | same trust class as feed ingestion                |
//! | `Provenance::Manual { .. }`          | **No — surfaced** | Manual decisions are operator-curated; a          |
//! |                                      | as conflict       | contradiction is surfaced as a conflict, not       |
//! |                                      |                   | auto-resolved                                     |
//! | `Provenance::System { .. }`          | **Yes**           | Platform bookkeeping edges follow Ingest trust    |
//!
//! # Non-functional overlaps
//!
//! When a new edge arrives for a relation that is NOT in the functional-predicate
//! set, no supersession is attempted.  Detecting non-functional overlaps cheaply
//! (e.g. "same `from`/`rel` but different validity windows") is the caller's
//! responsibility; the detector reports only functional-predicate contradictions.

use std::collections::HashSet;

/// The compiled functional-predicate set: taxonomy defaults merged with any
/// operator extension from `[knowledge.contradiction]`.
///
/// Construct via [`FunctionalPredicateSet::default`] (taxonomy defaults only)
/// or [`FunctionalPredicateSet::from_config`] (defaults + operator extras).
#[derive(Clone, Debug)]
pub struct FunctionalPredicateSet {
    rels: HashSet<String>,
}

/// The taxonomy-default functional predicates (see module-level table).
///
/// Changing this list is a **breaking schema change** — new defaults silently
/// start invalidating edges in every deployment.  Changes must be documented
/// and shipped with a migration guide.
pub const TAXONOMY_DEFAULTS: &[&str] = &[
    "ceo_of",
    "cfo_of",
    "primary_exchange",
    "domicile",
    "fiscal_year_end",
    "sector",
    "industry",
    "currency",
    "parent_company",
    "credit_rating",
];

impl Default for FunctionalPredicateSet {
    /// Returns the taxonomy-default functional predicate set (no operator
    /// extension).  Use [`FunctionalPredicateSet::from_config`] to merge in
    /// operator-supplied extras.
    fn default() -> Self {
        Self {
            rels: TAXONOMY_DEFAULTS
                .iter()
                .map(|&rel| rel.to_string())
                .collect(),
        }
    }
}

impl FunctionalPredicateSet {
    /// Build the set from taxonomy defaults merged with operator-supplied extras.
    ///
    /// `extra` must only contain relation names that pass the graph-id grammar
    /// (`[A-Za-z0-9:._-]+`, non-empty) — callers MUST validate before calling
    /// (see [`crate::validate_functional_rel`]).  This constructor does a
    /// best-effort dedup but does NOT re-validate; validation belongs at config
    /// load time where errors can be reported loudly.
    #[must_use]
    pub fn from_config(extra: &[String]) -> Self {
        let mut rels: HashSet<String> = TAXONOMY_DEFAULTS
            .iter()
            .map(|&rel| rel.to_string())
            .collect();
        rels.extend(extra.iter().cloned());
        Self { rels }
    }

    /// Whether `rel` is a functional predicate in this set.
    #[must_use]
    pub fn is_functional(&self, rel: &str) -> bool {
        self.rels.contains(rel)
    }

    /// The number of predicates in the set (defaults + operator extras).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rels.len()
    }

    /// Whether the set is empty (always false after construction from defaults).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rels.is_empty()
    }

    /// Iterate over every predicate in sorted order (deterministic; useful for
    /// logging and tests).
    pub fn iter_sorted(&self) -> impl Iterator<Item = &str> {
        let mut v: Vec<&str> = self.rels.iter().map(String::as_str).collect();
        v.sort_unstable();
        // Collect into a Vec to avoid returning a dangling borrow from the
        // temporary sort buffer.  The lifetime of `self` outlives the iterator.
        v.into_iter()
    }
}

/// Validate one relation name for the operator-extension list.
///
/// Returns `Ok(())` when the name is a valid graph id, `Err(reason)` otherwise.
/// This is the config-validation gate — an invalid name in
/// `[knowledge.contradiction].extra_functional_rels` is a hard config error.
///
/// # Errors
///
/// Returns a human-readable reason string when the name is empty or contains
/// characters outside the graph-id grammar.
pub fn validate_functional_rel(rel: &str) -> Result<(), &'static str> {
    if rel.is_empty() {
        return Err("functional relation name must not be empty");
    }
    if !rel
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err("functional relation name must match [A-Za-z0-9:._-]+ (graph-id grammar)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_contains_all_taxonomy_defaults() {
        let set = FunctionalPredicateSet::default();
        for rel in TAXONOMY_DEFAULTS {
            assert!(
                set.is_functional(rel),
                "taxonomy default {rel:?} missing from default set"
            );
        }
        assert_eq!(set.len(), TAXONOMY_DEFAULTS.len());
    }

    #[test]
    fn from_config_merges_extras_and_deduplicates() {
        let extra = vec![
            "ceo_of".to_string(), // already in defaults → dedup
            "board_chair".to_string(),
        ];
        let set = FunctionalPredicateSet::from_config(&extra);
        assert!(set.is_functional("board_chair"));
        assert!(set.is_functional("ceo_of")); // still there
        // len = defaults + 1 new (ceo_of deduped)
        assert_eq!(set.len(), TAXONOMY_DEFAULTS.len() + 1);
    }

    #[test]
    fn is_functional_returns_false_for_unknown_rel() {
        let set = FunctionalPredicateSet::default();
        assert!(!set.is_functional("mentions"));
        assert!(!set.is_functional("described_by"));
        assert!(!set.is_functional(""));
    }

    #[test]
    fn iter_sorted_is_deterministic() {
        let a = FunctionalPredicateSet::default();
        let b = FunctionalPredicateSet::default();
        let va: Vec<_> = a.iter_sorted().collect();
        let vb: Vec<_> = b.iter_sorted().collect();
        assert_eq!(va, vb);
        // Must actually be sorted.
        let mut sorted = va.clone();
        sorted.sort_unstable();
        assert_eq!(va, sorted);
    }

    #[test]
    fn validate_functional_rel_accepts_valid_and_rejects_invalid() {
        assert!(validate_functional_rel("ceo_of").is_ok());
        assert!(validate_functional_rel("primary_exchange").is_ok());
        assert!(validate_functional_rel("my.custom:rel-v1").is_ok());
        assert!(validate_functional_rel("").is_err());
        assert!(validate_functional_rel("bad rel").is_err()); // space
        assert!(validate_functional_rel("bad/path").is_err()); // slash
    }
}
