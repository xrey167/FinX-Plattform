//! Per-fact confidence aggregation (knowledge-system K-R6).
//!
//! Every graph edge gets a queryable trust score derived entirely from stored,
//! structured data — no LLM inference, no probabilistic ranking.  The formula
//! is **deterministic, monotone, and versioned** so any two callers computing
//! the same edge arrive at the same number.
//!
//! # Formula — version 1
//!
//! ```text
//! confidence = clamp(extraction × source_reliability × corroboration × contradiction, 0.0, 1.0)
//! ```
//!
//! Each component is bounded `[0.0, 1.0]`:
//!
//! | Component           | Symbol              | Range   | Source                                   |
//! |---------------------|---------------------|---------|------------------------------------------|
//! | Extraction          | `extraction`        | 0..=1   | K-M1 field (`extraction_confidence` prop); absent → `NEUTRAL` |
//! | Source reliability  | `source_reliability`| 0..=1   | K-R3 seam; caller-supplied optional; absent → `NEUTRAL` |
//! | Corroboration       | `corroboration`     | 1..=1.5 | Independent-source count (formula below) |
//! | Contradiction bonus | `contradiction`     | 0.85..=1.0 | Survived K-M4 check → 1.0; invalidated → 0.85 |
//!
//! ## Corroboration factor
//!
//! ```text
//! corroboration(n) = 1.0 + min(n - 1, MAX_CORROBORATION_BONUS) × CORROBORATION_STEP
//! ```
//!
//! where `n` = number of **independent** sources (distinct `Provenance::Ingest { source }`
//! values) that assert the same `(from, rel, to)` fact, `MAX_CORROBORATION_BONUS = 5`,
//! and `CORROBORATION_STEP = 0.1`.  This caps the factor at `1.5`.
//!
//! ## Independence definition
//!
//! Two edges are independent when they carry distinct `source` strings inside
//! `Provenance::Ingest { source }`.  Only `Ingest` provenance counts; `Rule`,
//! `Agent`, `Manual`, and `System` edges do not contribute to the corroboration
//! count.  The same source re-ingested (same `source` string) counts as one
//! origin regardless of how many graph edges it produced.
//!
//! ## Survived-contradiction bonus
//!
//! An edge that was temporally invalidated by K-M4 carries `invalidated_by` in
//! its `props`.  An active, non-invalidated edge with an open `valid_to` gets a
//! bonus factor of `1.0` (neutral); an invalidated edge gets `0.85`.
//!
//! ## Monotonicity guarantee
//!
//! All four components are non-decreasing with evidence: adding more independent
//! sources can only increase the corroboration factor; an edge surviving
//! contradiction detection never loses the bonus.  The `clamp` prevents
//! overflow beyond `1.0`.
//!
//! # Compute strategy — read-time
//!
//! Confidence is computed **at read-time** from the graph rather than stamped
//! on write.  Rationale:
//!
//! 1. The corroboration count changes each time a new independent source is
//!    ingested; a stamp-on-write approach would require updating every
//!    previously-written edge's confidence when a corroborating edge arrives —
//!    an O(n) write fan-out that conflicts with the `replace_edges` contract.
//! 2. The graph is the single source of truth; read-time derivation keeps
//!    provenance and the score in sync without a dual-write path.
//! 3. The corroboration scan is **bounded**: it counts only edges with the
//!    same `(from, rel, to)` triple — a small set in practice (the
//!    `MAX_CORROBORATION_CAP` ceiling prevents pathological scans even for
//!    heavily-duplicated facts).
//!
//! # K-M3 seam (tdw.kg.answer — not yet built)
//!
//! When `tdw.kg.answer` is built, it should read the `confidence` field on
//! `ConfidenceScore` and use it to weight answer candidates.  The seam is:
//! pass a `&[ConfidenceScore]` alongside the retrieved edges; weight by
//! `score.confidence`.
//!
//! # K-R3 seam (source-reliability — not yet built)
//!
//! Source reliability is a future input.  The aggregation accepts it as an
//! `Option<f64>` defaulting to [`NEUTRAL`] when absent.  When K-R3 ships,
//! callers pass the provider-specific reliability score here; the formula is
//! unchanged.
//!
//! # K-M1 seam (extraction confidence — not yet built)
//!
//! Extraction confidence is read from the edge's `props["extraction_confidence"]`
//! field as an `f64` in `[0.0, 1.0]`.  When K-M1 ships and stamps this field on
//! extraction, it flows into the formula automatically.  Absent → [`NEUTRAL`].

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{GraphEdge, Provenance};

// ── Public constants ───────────────────────────────────────────────────────────

/// Formula version.  Bump when the aggregation formula changes so callers can
/// detect stale cached scores.
pub const CONFIDENCE_FORMULA_VERSION: u32 = 1;

/// Neutral component value — used when a component is absent or unknown.
/// Chosen as `1.0` (multiplicative identity) so an unknown component has no
/// effect on the score, rather than pulling it down.
pub const NEUTRAL: f64 = 1.0;

/// Maximum independent-source bonus steps (caps corroboration at `1.0 + 5 × 0.1 = 1.5`).
pub const MAX_CORROBORATION_BONUS: u32 = 5;

/// Per-source corroboration increment.
pub const CORROBORATION_STEP: f64 = 0.1;

/// Confidence multiplier for a fact that survived K-M4 contradiction detection
/// (open `valid_to`, no `invalidated_by` in props).
pub const SURVIVED_CONTRADICTION_BONUS: f64 = 1.0;

/// Confidence multiplier for a fact that was invalidated by K-M4
/// (carries `"invalidated_by"` in props).
///
/// Note: a closed `valid_to` alone does **not** trigger this penalty — a
/// legitimately-expired temporal window does not carry `"invalidated_by"`.
/// Only K-M4's explicit annotation is authoritative; see [`survived_contradiction`].
pub const INVALIDATED_PENALTY: f64 = 0.85;

/// Hard ceiling on the corroboration scan.
///
/// Prevents pathological full-graph scans when a (from, rel, to) triple has
/// hundreds of duplicate edges. The first `MAX_CORROBORATION_CAP` edges with
/// the matching triple are examined; beyond that the count is capped.
pub const MAX_CORROBORATION_CAP: usize = 64;

/// Default confidence weight in hybrid retrieval ranking.
///
/// Low so relevance (RRF score) dominates; set to `0.0` to disable.
/// Configurable per query via [`crate::ConfidenceRankingWeight`] — see
/// `tdw-retrieve`.
pub const DEFAULT_CONFIDENCE_WEIGHT: f64 = 0.05;

// ── Types ──────────────────────────────────────────────────────────────────────

/// The decomposed confidence score for one graph edge.
///
/// All fields are `f64` in `[0.0, 1.0]` except `corroboration_sources` which
/// is the raw independent-source count used to derive the corroboration factor.
/// The `confidence` field is the final aggregated score after clamping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfidenceScore {
    /// Formula version — [`CONFIDENCE_FORMULA_VERSION`].
    pub formula_version: u32,
    /// Extraction confidence read from `props["extraction_confidence"]`.
    /// [`NEUTRAL`] when absent (K-M1 seam — field not yet stamped).
    pub extraction: f64,
    /// Source reliability from the K-R3 seam.
    /// [`NEUTRAL`] when absent (K-R3 not yet built).
    pub source_reliability: f64,
    /// Number of independent sources (distinct `Provenance::Ingest { source }` values)
    /// that assert the same `(from, rel, to)` fact.
    pub corroboration_sources: u32,
    /// The corroboration factor derived from `corroboration_sources`
    /// (`1.0 + min(n-1, MAX_CORROBORATION_BONUS) × CORROBORATION_STEP`).
    pub corroboration_factor: f64,
    /// Whether the edge survived K-M4 contradiction detection.
    /// `true` → factor [`SURVIVED_CONTRADICTION_BONUS`]; `false` → [`INVALIDATED_PENALTY`].
    pub survived_contradiction: bool,
    /// The contradiction factor applied.
    pub contradiction_factor: f64,
    /// Final aggregated confidence: `clamp(extraction × source_reliability × corroboration_factor × contradiction_factor, 0.0, 1.0)`.
    pub confidence: f64,
}

/// Lightweight input to the aggregation — avoids cloning the full edge set.
///
/// Callers construct this from a `GraphEdge` via [`EdgeConfidenceInput::from_edge`].
#[derive(Clone, Debug)]
pub struct EdgeConfidenceInput<'a> {
    /// The `from` node id of the fact being scored.
    pub from: &'a str,
    /// The relationship type.
    pub rel: &'a str,
    /// The `to` node id.
    pub to: &'a str,
    /// The edge provenance (determines independence category).
    pub provenance: &'a Provenance,
    /// The edge props (read for `extraction_confidence` and `invalidated_by`).
    pub props: &'a serde_json::Value,
    /// The `valid_to` bound (open = `None`; closed = K-M4 closed this edge).
    pub valid_to: Option<&'a str>,
}

impl<'a> EdgeConfidenceInput<'a> {
    /// Borrow from a [`GraphEdge`].
    #[must_use]
    pub fn from_edge(edge: &'a GraphEdge) -> Self {
        Self {
            from: &edge.from,
            rel: &edge.rel,
            to: &edge.to,
            provenance: &edge.provenance,
            props: &edge.props,
            valid_to: edge.valid_to.as_deref(),
        }
    }
}

// ── Aggregation ────────────────────────────────────────────────────────────────

/// Compute confidence for one edge given a bounded slice of **all** edges that
/// share the same `(from, rel)` pair.
///
/// The `candidates` slice is the corroboration pool: this function scans it for
/// edges whose `to` equals the subject's `to`, counting distinct independent
/// sources.  The slice must be pre-filtered to `from == subject.from && rel ==
/// subject.rel` by the caller; it need not be sorted.  Only up to
/// [`MAX_CORROBORATION_CAP`] candidates are examined — beyond that the count
/// is capped, preventing a pathological scan.
///
/// `source_reliability` is the K-R3 seam — pass `None` (becomes [`NEUTRAL`])
/// until K-R3 is built.
///
/// # Formula
///
/// ```text
/// confidence = clamp(extraction × source_reliability × corroboration_factor × contradiction_factor, 0.0, 1.0)
/// ```
///
/// See module-level documentation for component definitions.
#[must_use]
pub fn compute_confidence(
    subject: &EdgeConfidenceInput<'_>,
    candidates: &[&GraphEdge],
    source_reliability: Option<f64>,
) -> ConfidenceScore {
    // ── Component (a): extraction confidence ──────────────────────────────────
    // K-M1 seam: read from props["extraction_confidence"]; absent → NEUTRAL.
    let extraction = subject
        .props
        .get("extraction_confidence")
        .and_then(serde_json::Value::as_f64)
        .map_or(NEUTRAL, |v| v.clamp(0.0, 1.0));

    // ── Component (b): source reliability ────────────────────────────────────
    // K-R3 seam: caller-supplied optional; absent → NEUTRAL.
    let source_reliability = source_reliability.map_or(NEUTRAL, |v| v.clamp(0.0, 1.0));

    // ── Component (c): independent-corroboration count ────────────────────────
    // Count distinct source strings among Ingest-provenance edges that assert
    // the same (from, rel, to) triple, bounded by MAX_CORROBORATION_CAP.
    let corroboration_sources = count_independent_sources(subject, candidates);
    let corroboration_factor = corroboration_factor(corroboration_sources);

    // ── Component (d): survived-contradiction bonus ───────────────────────────
    // An edge is considered invalidated when:
    //   - its props carry an "invalidated_by" field (K-M4 annotation), OR
    //   - its valid_to is closed (non-None) AND it has invalidated_by.
    // An edge without "invalidated_by" and with open valid_to survived.
    let survived_contradiction = survived_contradiction(subject);
    let contradiction_factor = if survived_contradiction {
        SURVIVED_CONTRADICTION_BONUS
    } else {
        INVALIDATED_PENALTY
    };

    // ── Final score ───────────────────────────────────────────────────────────
    let raw = extraction * source_reliability * corroboration_factor * contradiction_factor;
    let confidence = raw.clamp(0.0, 1.0);

    ConfidenceScore {
        formula_version: CONFIDENCE_FORMULA_VERSION,
        extraction,
        source_reliability,
        corroboration_sources,
        corroboration_factor,
        survived_contradiction,
        contradiction_factor,
        confidence,
    }
}

/// Corroboration factor from an independent-source count `n`.
///
/// `f(n) = 1.0 + min(max(n, 1) - 1, MAX_CORROBORATION_BONUS) × CORROBORATION_STEP`
///
/// `f(0) = f(1) = 1.0` (zero or one source → no bonus, no penalty; `saturating_sub`
/// treats `n=0` identically to `n=1`); `f(6+) = 1.5` (capped at [`MAX_CORROBORATION_BONUS`]
/// bonus steps).  This is the only place where 0 and 1 are equated — the
/// counter in [`count_independent_sources`] returns the raw count and does not
/// substitute 1 for 0.
#[must_use]
pub fn corroboration_factor(n: u32) -> f64 {
    let bonus_steps = n.saturating_sub(1).min(MAX_CORROBORATION_BONUS);
    1.0 + f64::from(bonus_steps) * CORROBORATION_STEP
}

/// Count the number of **independent** sources asserting `(from, rel, to)`.
///
/// Only `Provenance::Ingest { source }` edges contribute.  Distinct `source`
/// strings are independent; re-ingesting the same source is idempotent.
///
/// Edges are **first filtered** to the matching `to` target and then capped at
/// [`MAX_CORROBORATION_CAP`].  Filtering before capping is essential: without
/// it, unrelated edges for different targets consume cap budget and silently
/// under-count real corroborators (the exact K-X7 bug class).
#[must_use]
pub fn count_independent_sources(
    subject: &EdgeConfidenceInput<'_>,
    candidates: &[&GraphEdge],
) -> u32 {
    let mut seen_sources: BTreeSet<&str> = BTreeSet::new();
    // Filter to the same (from, rel, to) triple FIRST, then cap.  Capping
    // before filtering would allow noise edges for other targets to exhaust
    // the budget, hiding real corroborators.
    for edge in candidates
        .iter()
        .filter(|e| e.to == subject.to)
        .take(MAX_CORROBORATION_CAP)
    {
        if let Provenance::Ingest { source } = &edge.provenance {
            seen_sources.insert(source.as_str());
        }
    }
    // A subject with no Ingest edges (e.g. Rule/Agent) has 0 independent
    // sources; corroboration_factor treats 0 the same as 1 (no bonus, no
    // penalty — the factor is 1.0 at n ≤ 1).
    u32::try_from(seen_sources.len()).unwrap_or(u32::MAX)
}

/// Whether `subject` survived K-M4 contradiction detection.
///
/// An edge is invalidated when its `props` carries an `"invalidated_by"` field
/// (written by [`tdw_infer::contradiction`] on temporal close).  An edge without
/// that field is considered to have survived.
///
/// The `valid_to` being closed is a necessary but not sufficient signal — a
/// legitimately-expired window (temporal close for a different reason) does not
/// carry `invalidated_by`.  The annotation is authoritative.
#[must_use]
pub fn survived_contradiction(subject: &EdgeConfidenceInput<'_>) -> bool {
    subject
        .props
        .get("invalidated_by")
        .is_none_or(serde_json::Value::is_null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphEdge, Provenance};
    use serde_json::{Value, json};

    // ── Fixtures ───────────────────────────────────────────────────────────────

    fn ingest_edge(from: &str, rel: &str, to: &str, source: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: Value::Null,
            provenance: Provenance::Ingest {
                source: source.to_string(),
            },
            valid_from: None,
            valid_to: None,
        }
    }

    fn ingest_edge_with_props(
        from: &str,
        rel: &str,
        to: &str,
        source: &str,
        props: Value,
    ) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props,
            provenance: Provenance::Ingest {
                source: source.to_string(),
            },
            valid_from: None,
            valid_to: None,
        }
    }

    fn rule_edge(from: &str, rel: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: Value::Null,
            provenance: Provenance::Rule {
                rule_id: "r1".to_string(),
                version: 1,
            },
            valid_from: None,
            valid_to: None,
        }
    }

    fn edge_refs(edges: &[GraphEdge]) -> Vec<&GraphEdge> {
        edges.iter().collect()
    }

    // ── corroboration_factor ───────────────────────────────────────────────────

    #[test]
    fn corroboration_factor_is_neutral_for_single_source() {
        // f(1) = 1.0 (no bonus): a single-source fact gets no penalty, no bonus.
        assert!((corroboration_factor(1) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn corroboration_factor_is_neutral_for_zero_sources() {
        // f(0) = 1.0: zero Ingest edges → same as single-source (no penalty).
        assert!((corroboration_factor(0) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn corroboration_factor_increases_monotonically() {
        let factors: Vec<f64> = (0u32..=7).map(corroboration_factor).collect();
        for window in factors.windows(2) {
            assert!(
                window[1] >= window[0],
                "factor must be non-decreasing: {factors:?}"
            );
        }
    }

    #[test]
    fn corroboration_factor_caps_at_1_5() {
        // f(MAX_CORROBORATION_BONUS + 1 == 6) = 1.5; f(100) also 1.5.
        assert!((corroboration_factor(6) - 1.5).abs() < f64::EPSILON);
        assert!((corroboration_factor(100) - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn corroboration_factor_at_two_sources() {
        // f(2) = 1.0 + 1 × 0.1 = 1.1
        assert!((corroboration_factor(2) - 1.1).abs() < f64::EPSILON);
    }

    // ── count_independent_sources ──────────────────────────────────────────────

    #[test]
    fn same_source_reingest_does_not_inflate_corroboration() {
        // Two edges from the same source → count = 1, not 2.
        let e1 = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let e2 = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let pool = [e1, e2];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput {
            from: "company:ACME",
            rel: "ceo_of",
            to: "person:alice",
            provenance: &Provenance::Ingest {
                source: "provider:sec".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };
        assert_eq!(count_independent_sources(&subject, &candidates), 1);
    }

    #[test]
    fn distinct_sources_count_independently() {
        // Three distinct sources → count = 3.
        let e1 = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let e2 = ingest_edge(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:bloomberg",
        );
        let e3 = ingest_edge(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:refinitiv",
        );
        let pool = [e1, e2, e3];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput {
            from: "company:ACME",
            rel: "ceo_of",
            to: "person:alice",
            provenance: &Provenance::Ingest {
                source: "provider:sec".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };
        assert_eq!(count_independent_sources(&subject, &candidates), 3);
    }

    #[test]
    fn rule_and_agent_edges_do_not_count_as_independent_sources() {
        // Only Ingest provenance counts; Rule/Agent do not.
        let ingest = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let rule = rule_edge("company:ACME", "ceo_of", "person:alice");
        let agent = GraphEdge {
            from: "company:ACME".to_string(),
            to: "person:alice".to_string(),
            rel: "ceo_of".to_string(),
            props: Value::Null,
            provenance: Provenance::Agent {
                agent_id: "analyst-bot".to_string(),
                gated: true,
            },
            valid_from: None,
            valid_to: None,
        };
        let pool = [ingest, rule, agent];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput {
            from: "company:ACME",
            rel: "ceo_of",
            to: "person:alice",
            provenance: &Provenance::Ingest {
                source: "provider:sec".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };
        // Only 1 Ingest source.
        assert_eq!(count_independent_sources(&subject, &candidates), 1);
    }

    #[test]
    fn edges_for_different_to_do_not_count() {
        // A corroborating edge pointing at a different `to` must not inflate the count.
        let alice = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let bob = ingest_edge("company:ACME", "ceo_of", "person:bob", "provider:bloomberg");
        let pool = [alice, bob];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput {
            from: "company:ACME",
            rel: "ceo_of",
            to: "person:alice",
            provenance: &Provenance::Ingest {
                source: "provider:sec".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };
        assert_eq!(count_independent_sources(&subject, &candidates), 1);
    }

    // ── survived_contradiction ─────────────────────────────────────────────────

    #[test]
    fn active_edge_without_invalidated_by_survives() {
        let subject = EdgeConfidenceInput {
            from: "a:1",
            rel: "r",
            to: "b:2",
            provenance: &Provenance::Ingest {
                source: "s".to_string(),
            },
            props: &json!({}),
            valid_to: None,
        };
        assert!(survived_contradiction(&subject));
    }

    #[test]
    fn edge_with_invalidated_by_does_not_survive() {
        let props =
            json!({ "invalidated_by": "company:ACME->ceo_of->person:bob@2024-06-01T00:00:00Z" });
        let subject = EdgeConfidenceInput {
            from: "company:ACME",
            rel: "ceo_of",
            to: "person:alice",
            provenance: &Provenance::Ingest {
                source: "provider:sec".to_string(),
            },
            props: &props,
            valid_to: Some("2024-06-01T00:00:00Z"),
        };
        assert!(!survived_contradiction(&subject));
    }

    // ── compute_confidence — component isolation ───────────────────────────────

    #[test]
    fn single_source_uncorroborated_no_extras_yields_neutral() {
        // All components neutral: extraction=1.0, source_reliability=None(→1.0),
        // corroboration=1 source → factor 1.0, survived → 1.0.
        // confidence = 1.0 × 1.0 × 1.0 × 1.0 = 1.0.
        let edge = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let pool = [edge.clone()];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput::from_edge(&edge);
        let score = compute_confidence(&subject, &candidates, None);
        assert_eq!(score.formula_version, CONFIDENCE_FORMULA_VERSION);
        assert!(
            (score.extraction - 1.0).abs() < f64::EPSILON,
            "extraction should be neutral"
        );
        assert!((score.source_reliability - 1.0).abs() < f64::EPSILON);
        assert_eq!(score.corroboration_sources, 1);
        assert!((score.corroboration_factor - 1.0).abs() < f64::EPSILON);
        assert!(score.survived_contradiction);
        assert!((score.contradiction_factor - 1.0).abs() < f64::EPSILON);
        assert!((score.confidence - 1.0).abs() < 1e-9);
    }

    #[test]
    fn extraction_confidence_from_props_is_used() {
        // props["extraction_confidence"] = 0.7 → extraction = 0.7.
        let edge = ingest_edge_with_props(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:sec",
            json!({ "extraction_confidence": 0.7 }),
        );
        let pool = [edge.clone()];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput::from_edge(&edge);
        let score = compute_confidence(&subject, &candidates, None);
        assert!((score.extraction - 0.7).abs() < f64::EPSILON);
        // confidence = 0.7 × 1.0 × 1.0 × 1.0 = 0.7
        assert!((score.confidence - 0.7).abs() < 1e-9);
    }

    #[test]
    fn source_reliability_seam_scales_confidence() {
        // source_reliability = 0.5 → confidence = 1.0 × 0.5 × 1.0 × 1.0 = 0.5.
        let edge = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let pool = [edge.clone()];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput::from_edge(&edge);
        let score = compute_confidence(&subject, &candidates, Some(0.5));
        assert!((score.source_reliability - 0.5).abs() < f64::EPSILON);
        assert!((score.confidence - 0.5).abs() < 1e-9);
    }

    #[test]
    fn three_independent_sources_raise_corroboration() {
        // 3 distinct sources → factor = 1.0 + 2 × 0.1 = 1.2.
        // confidence = 1.0 × 1.0 × 1.2 × 1.0 = 1.0 (clamped).
        let e1 = ingest_edge("company:ACME", "ceo_of", "person:alice", "provider:sec");
        let e2 = ingest_edge(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:bloomberg",
        );
        let e3 = ingest_edge(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:refinitiv",
        );
        let pool = [e1.clone(), e2, e3];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput::from_edge(&e1);
        let score = compute_confidence(&subject, &candidates, None);
        assert_eq!(score.corroboration_sources, 3);
        assert!((score.corroboration_factor - 1.2).abs() < f64::EPSILON);
        // Clamped at 1.0 — cannot exceed the ceiling.
        assert!((score.confidence - 1.0).abs() < 1e-9);
    }

    #[test]
    fn invalidated_edge_gets_penalty() {
        // invalidated_by annotation → contradiction_factor = 0.85.
        let props =
            json!({ "invalidated_by": "company:ACME->ceo_of->person:bob@2024-06-01T00:00:00Z" });
        let edge = ingest_edge_with_props(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:sec",
            props,
        );
        let pool = [edge.clone()];
        let candidates = edge_refs(&pool);
        let subject = EdgeConfidenceInput::from_edge(&edge);
        let score = compute_confidence(&subject, &candidates, None);
        assert!(!score.survived_contradiction);
        assert!((score.contradiction_factor - INVALIDATED_PENALTY).abs() < f64::EPSILON);
        assert!((score.confidence - INVALIDATED_PENALTY).abs() < 1e-9);
    }

    // ── The key honesty contract ───────────────────────────────────────────────

    #[test]
    fn single_source_uncorroborated_vs_three_source_corroborated_survived_differ_visibly() {
        // Single source with low extraction confidence (0.4) and low source
        // reliability (0.5): score = 0.4 × 0.5 × 1.0 × 1.0 = 0.2.
        let single = ingest_edge_with_props(
            "company:ACME",
            "ceo_of",
            "person:alice",
            "provider:sec",
            json!({ "extraction_confidence": 0.4 }),
        );
        let single_pool = [single.clone()];
        let single_refs = edge_refs(&single_pool);
        let single_input = EdgeConfidenceInput::from_edge(&single);
        // source_reliability = 0.5 (simulating a low-trust source).
        let single_score = compute_confidence(&single_input, &single_refs, Some(0.5));

        // Three independent sources + extraction_confidence=0.8 + survived +
        // source_reliability=0.9:
        // score = 0.8 × 0.9 × (1.0 + 2 × 0.1) × 1.0 = 0.8 × 0.9 × 1.2 = 0.864.
        let e1 = ingest_edge_with_props(
            "company:ACME",
            "sector",
            "sector:tech",
            "provider:sec",
            json!({ "extraction_confidence": 0.8 }),
        );
        let e2 = ingest_edge(
            "company:ACME",
            "sector",
            "sector:tech",
            "provider:bloomberg",
        );
        let e3 = ingest_edge(
            "company:ACME",
            "sector",
            "sector:tech",
            "provider:refinitiv",
        );
        let corroborated_pool = [e1.clone(), e2, e3];
        let corroborated_refs = edge_refs(&corroborated_pool);
        let corroborated_input = EdgeConfidenceInput::from_edge(&e1);
        // source_reliability = 0.9 (trusted source).
        let corroborated_score =
            compute_confidence(&corroborated_input, &corroborated_refs, Some(0.9));

        // The corroborated-survived fact must score VISIBLY higher.
        assert!(
            corroborated_score.confidence > single_score.confidence,
            "corroborated (conf={}) must beat single-source (conf={}) visibly",
            corroborated_score.confidence,
            single_score.confidence
        );
        // And the difference must be meaningful (at least 20 percentage points).
        assert!(
            corroborated_score.confidence - single_score.confidence >= 0.20,
            "difference must be at least 0.20: corroborated={} single={}",
            corroborated_score.confidence,
            single_score.confidence
        );
    }

    // ── Bounded-scan test ──────────────────────────────────────────────────────

    #[test]
    fn corroboration_count_is_capped_at_max_corroboration_cap() {
        // Build MAX_CORROBORATION_CAP + 10 distinct sources — count must not exceed
        // MAX_CORROBORATION_BONUS steps (the score caps, the scan caps).
        let edges: Vec<GraphEdge> = (0..MAX_CORROBORATION_CAP + 10)
            .map(|i| ingest_edge("a:1", "r", "b:2", &format!("provider:src-{i}")))
            .collect();
        let refs = edge_refs(&edges);
        let subject = EdgeConfidenceInput {
            from: "a:1",
            rel: "r",
            to: "b:2",
            provenance: &Provenance::Ingest {
                source: "provider:src-0".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };
        let count = count_independent_sources(&subject, &refs);
        // Only MAX_CORROBORATION_CAP edges are examined.
        assert!(
            count as usize <= MAX_CORROBORATION_CAP,
            "count {count} must not exceed MAX_CORROBORATION_CAP {MAX_CORROBORATION_CAP}"
        );
        // The factor must be clamped at 1.5.
        let factor = corroboration_factor(count);
        assert!(
            factor <= 1.5 + f64::EPSILON,
            "factor {factor} must not exceed 1.5"
        );
    }

    #[test]
    fn noise_edges_beyond_cap_do_not_hide_real_corroborators() {
        // Reproduce the K-X7 bug class: 64 noise edges pointing to a DIFFERENT
        // target ("b:noise") followed by 3 real corroborators for "b:real".
        //
        // Before the fix (cap-before-filter), the 64 noise edges would exhaust
        // the cap and return count=0 for "b:real", yielding corroboration_factor=1.0.
        //
        // After the fix (filter-before-cap), the noise edges are filtered out
        // first, only the 3 real corroborators enter the capped iterator, and
        // count=3 → corroboration_factor = 1.0 + 2 × 0.1 = 1.2.
        let mut edges: Vec<GraphEdge> = (0..MAX_CORROBORATION_CAP)
            .map(|i| ingest_edge("a:1", "r", "b:noise", &format!("provider:noise-{i}")))
            .collect();
        edges.push(ingest_edge("a:1", "r", "b:real", "provider:src-A"));
        edges.push(ingest_edge("a:1", "r", "b:real", "provider:src-B"));
        edges.push(ingest_edge("a:1", "r", "b:real", "provider:src-C"));

        let refs = edge_refs(&edges);
        let subject = EdgeConfidenceInput {
            from: "a:1",
            rel: "r",
            to: "b:real",
            provenance: &Provenance::Ingest {
                source: "provider:src-A".to_string(),
            },
            props: &Value::Null,
            valid_to: None,
        };

        let count = count_independent_sources(&subject, &refs);
        assert_eq!(
            count, 3,
            "filter-before-cap: 3 real corroborators behind 64 noise edges must all be counted; got {count}"
        );
        let factor = corroboration_factor(count);
        assert!(
            (factor - 1.2).abs() < 1e-9,
            "3 sources → factor = 1.0 + 2×0.1 = 1.2; got {factor}"
        );
    }

    // ── Monotonicity ───────────────────────────────────────────────────────────

    #[test]
    fn adding_sources_never_lowers_confidence() {
        // Build a baseline with 1 source, then add sources one by one and
        // check that confidence is non-decreasing.
        let mut prev = 0.0_f64;
        for n in 1u32..=7 {
            let edges: Vec<GraphEdge> = (0..n)
                .map(|i| ingest_edge("a:1", "r", "b:2", &format!("provider:src-{i}")))
                .collect();
            let refs = edge_refs(&edges);
            let subject = EdgeConfidenceInput {
                from: "a:1",
                rel: "r",
                to: "b:2",
                provenance: &Provenance::Ingest {
                    source: "provider:src-0".to_string(),
                },
                props: &Value::Null,
                valid_to: None,
            };
            let score = compute_confidence(&subject, &refs, None);
            assert!(
                score.confidence >= prev - f64::EPSILON,
                "confidence must be non-decreasing: n={n} score={} prev={}",
                score.confidence,
                prev
            );
            prev = score.confidence;
        }
    }
}
