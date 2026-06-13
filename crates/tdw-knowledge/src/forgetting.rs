//! Governed forgetting — knowledge hygiene (knowledge-system K-R8).
//!
//! The trust-layer counterpart to learning.  A periodic hygiene worker retires
//! stale / superseded / low-value knowledge to a **COLD** plane through the
//! SAME B9 gate — auditable, bounded, and REVERSIBLE — so the graph stays
//! clean without ever silently losing information.
//!
//! # The whole point: trust guarantees
//!
//! 1. **Proposals, not deletions.** A forgetting decision is identified by an
//!    explicit, configurable policy ([`ForgettingPolicy`]) combining four
//!    signals into a score, then enqueued as a `ProposalKind::Forget`
//!    proposal (provenance `Agent { gated: true }`).  Nothing is retired
//!    without passing the gate.
//! 2. **Move, never delete — and reversible.** "Forgetting" = MOVE to a COLD
//!    plane: [`retire_edge_to_cold`] closes the edge's validity window at
//!    injected-now (reusing the K-M4 temporal-close machinery via
//!    [`tdw_core::GraphEngine::replace_edges`]) and annotates a `forgotten`
//!    audit block.  The historical record is preserved — `as_of` queries over
//!    past time STILL return the fact (it was true then) — and
//!    [`recall_cold_edge`] restores it through the gate.
//! 3. **Fail-safe guards (policy CANNOT override).** [`ForgettingPolicy`]
//!    enforces hard floors: never forget high-confidence / well-corroborated
//!    facts; never forget `Ingest`-provenance ground truth below a retention
//!    floor; per-sweep caps; a kill-switch.
//! 4. **Truthful audit.** Every retirement records WHY (which policies fired,
//!    the score), WHEN (`forgotten_at`), queryable via `tdw.kg.why` (the
//!    `forgotten` props block surfaces there).  A fact is reported cold only
//!    if it ACTUALLY moved.
//!
//! # Injected-now (no wall-clock)
//!
//! Every staleness computation takes `now` as a `YYYY-MM-DD` (or RFC 3339)
//! string parameter.  This module reads NO wall-clock; the daemon worker
//! injects `now`, exactly like the consolidation / sweep schedulers.  Date
//! math uses the Howard-Hinnant `days_from_civil` inverse so month/year
//! boundaries are handled correctly (no naive `days = months × 30`).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tdw_core::confidence::{
    EdgeConfidenceInput, MAX_CORROBORATION_CAP, compute_confidence, count_independent_sources,
};
use tdw_core::{GraphEdge, GraphEngine, Provenance};

use crate::{KnowledgeError, Result};

/// Page size for graph-edge scans inside the hygiene sweep.
const PAGE: usize = 256;

/// The props key under which a retired edge carries its audit block.
pub const FORGOTTEN_PROPS_KEY: &str = "forgotten";

// ── Date math (deterministic, injected-now, no wall-clock) ───────────────────

/// Convert a `(year, month, day)` civil date to a count of days since the Unix
/// epoch (1970-01-01) — the Howard-Hinnant inverse of
/// [`tdw_core::date::civil_from_days`].
///
/// This is the single primitive that makes staleness math correct across
/// month/year boundaries: day differences are computed on the epoch-day axis,
/// never by approximating months as 30 days.
#[must_use]
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse a `YYYY-MM-DD` (or `YYYY-MM-DDThh:mm:ssZ`) string to epoch-days.
///
/// Returns `None` when the leading 10 chars are not a well-formed date — the
/// caller then treats the edge as undated (no staleness signal, never a
/// forgetting candidate on staleness grounds).
#[must_use]
pub fn date_to_epoch_days(date: &str) -> Option<i64> {
    let head = date.get(0..10)?;
    let mut parts = head.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// Whole-day age between an edge's reference date and `now`.
///
/// Both are `YYYY-MM-DD`-ish.  `None` when either is unparseable.  Negative
/// results (reference in the future relative to `now`) are clamped to `0` — a
/// future-dated fact is not "stale".
#[must_use]
pub fn age_in_days(reference: &str, now: &str) -> Option<i64> {
    let ref_days = date_to_epoch_days(reference)?;
    let now_days = date_to_epoch_days(now)?;
    Some((now_days - ref_days).max(0))
}

// ── Policy ───────────────────────────────────────────────────────────────────

/// The explicit, configurable forgetting policy (K-R8 requirement 1 + 3).
///
/// Combines four staleness/value signals into a `[0.0, 1.0]` score, then
/// applies HARD fail-safe guards that the score CANNOT override.
///
/// Every field is a plain value injected by the daemon from
/// `[knowledge.hygiene]`; the type carries no I/O and no wall-clock.
#[derive(Clone, Debug, PartialEq)]
pub struct ForgettingPolicy {
    /// Age (days) past which a fact's last-corroboration / validity start is
    /// considered stale.
    pub staleness_days: i64,
    /// Combined score at or above which a fact becomes a candidate.
    pub forget_score_threshold: f64,
    /// FAIL-SAFE: confidence at or above this is NEVER forgotten.
    pub protect_confidence_at_or_above: f64,
    /// FAIL-SAFE: corroboration (independent sources) at or above this is
    /// NEVER forgotten.
    pub protect_corroboration_at_or_above: u32,
    /// FAIL-SAFE: `Ingest` ground truth younger than this many days is NEVER
    /// forgotten, regardless of score.
    pub ingest_retention_floor_days: i64,
}

impl Default for ForgettingPolicy {
    fn default() -> Self {
        // Mirrors `tdw_config::HygieneConfig` defaults (ship-enabled-but-cautious).
        Self {
            staleness_days: 365,
            forget_score_threshold: 0.6,
            protect_confidence_at_or_above: 0.8,
            protect_corroboration_at_or_above: 3,
            ingest_retention_floor_days: 180,
        }
    }
}

/// The four signals that compose a forgetting score, plus the final score and
/// the guard verdict.  Returned by [`ForgettingPolicy::evaluate`] so the audit
/// trail can record exactly which policies fired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForgettingAssessment {
    /// Staleness signal `[0.0, 1.0]`: `1.0` when age ≥ `staleness_days`,
    /// scaled linearly below that, `0.0` when undated (no signal).
    pub staleness: f64,
    /// Low-confidence signal `[0.0, 1.0]`: `1.0 - confidence`.
    pub low_confidence: f64,
    /// Superseded-by-contradiction signal: `1.0` when the edge carries a K-M4
    /// `invalidated_by` annotation, else `0.0`.
    pub superseded: f64,
    /// Never-retrieved signal: `1.0` when the edge has no `last_retrieved_at`
    /// prop AND is stale, else `0.0` (a conservative proxy — retrieval
    /// telemetry is a future seam; absence + staleness is the honest signal).
    pub never_retrieved: f64,
    /// The combined, clamped `[0.0, 1.0]` forgetting score.
    pub score: f64,
    /// Why this fact is protected, when a fail-safe guard fired.  `None` means
    /// no guard fired (the score alone decides candidacy).
    pub protected_reason: Option<String>,
}

impl ForgettingAssessment {
    /// A human/audit string naming the firing signals + score — flows into the
    /// `forgotten` props block and the proposal `reason`.
    #[must_use]
    pub fn audit_reason(&self) -> String {
        let mut fired: Vec<String> = Vec::new();
        if self.staleness > 0.0 {
            fired.push(format!("staleness={:.2}", self.staleness));
        }
        if self.low_confidence > 0.0 {
            fired.push(format!("low_confidence={:.2}", self.low_confidence));
        }
        if self.superseded > 0.0 {
            fired.push("superseded_by_contradiction".to_string());
        }
        if self.never_retrieved > 0.0 {
            fired.push("never_retrieved".to_string());
        }
        format!(
            "governed-forgetting (K-R8): score={:.2}; policies=[{}]",
            self.score,
            fired.join(", ")
        )
    }
}

impl ForgettingPolicy {
    /// Evaluate one edge against the policy, given its computed K-R6
    /// confidence and independent-source count.  Pure: no I/O, `now` injected.
    ///
    /// Returns the full [`ForgettingAssessment`] (signals + score + guard
    /// verdict). [`ForgettingAssessment::protected_reason`] is `Some(..)` when
    /// a HARD fail-safe guard fired — such an edge is NEVER a candidate
    /// regardless of score.
    #[must_use]
    pub fn evaluate(
        &self,
        edge: &GraphEdge,
        confidence: f64,
        corroboration_sources: u32,
        now: &str,
    ) -> ForgettingAssessment {
        // ── Fail-safe guards (checked first; they cannot be overridden) ──────
        let protected_reason = self.guard(edge, confidence, corroboration_sources, now);

        // ── Signal (a): staleness ────────────────────────────────────────────
        // Reference date = last corroboration if present, else validity start.
        let reference = edge
            .props
            .get("last_corroborated_at")
            .and_then(serde_json::Value::as_str)
            .or(edge.valid_from.as_deref());
        let age = reference.and_then(|r| age_in_days(r, now));
        let staleness = age.map_or(0.0, |a| {
            if self.staleness_days <= 0 {
                0.0
            } else {
                #[allow(clippy::cast_precision_loss)]
                (a as f64 / self.staleness_days as f64).clamp(0.0, 1.0)
            }
        });

        // ── Signal (b): low confidence ───────────────────────────────────────
        let low_confidence = (1.0 - confidence).clamp(0.0, 1.0);

        // ── Signal (c): superseded by contradiction (K-M4) ───────────────────
        let superseded = if edge
            .props
            .get("invalidated_by")
            .is_some_and(|v| !v.is_null())
        {
            1.0
        } else {
            0.0
        };

        // ── Signal (d): never retrieved ──────────────────────────────────────
        // Conservative proxy: a stale fact with no retrieval timestamp.
        let never_retrieved = if staleness >= 1.0
            && edge.props.get("last_retrieved_at").is_none()
        {
            1.0
        } else {
            0.0
        };

        // ── Combine: weighted, then clamp.  Weights sum to 1.0 so a single
        // maxed signal cannot alone exceed the ceiling; superseded is weighted
        // highest (a contradicted fact is the strongest forget signal).  Built
        // with `mul_add` for accuracy (and to satisfy the flops lint). ────────
        let weighted = 0.30f64.mul_add(
            staleness,
            0.25f64.mul_add(
                low_confidence,
                0.30f64.mul_add(superseded, 0.15 * never_retrieved),
            ),
        );
        let score = weighted.clamp(0.0, 1.0);

        ForgettingAssessment {
            staleness,
            low_confidence,
            superseded,
            never_retrieved,
            score,
            protected_reason,
        }
    }

    /// Whether `assessment` makes `edge` a forgetting CANDIDATE: the score must
    /// meet the threshold AND no fail-safe guard fired.
    #[must_use]
    pub fn is_candidate(&self, assessment: &ForgettingAssessment) -> bool {
        assessment.protected_reason.is_none()
            && assessment.score >= self.forget_score_threshold
    }

    /// Apply the HARD fail-safe guards.  Returns `Some(reason)` when the edge
    /// is protected (never forgettable), `None` when the score alone decides.
    fn guard(
        &self,
        edge: &GraphEdge,
        confidence: f64,
        corroboration_sources: u32,
        now: &str,
    ) -> Option<String> {
        // Guard 1: high confidence is sacrosanct.
        if confidence >= self.protect_confidence_at_or_above {
            return Some(format!(
                "high-confidence ({confidence:.2} ≥ {:.2})",
                self.protect_confidence_at_or_above
            ));
        }
        // Guard 2: well-corroborated facts are sacrosanct.
        if corroboration_sources >= self.protect_corroboration_at_or_above {
            return Some(format!(
                "well-corroborated ({corroboration_sources} ≥ {} independent sources)",
                self.protect_corroboration_at_or_above
            ));
        }
        // Guard 3: Ingest-provenance ground truth below the retention floor.
        if matches!(edge.provenance, Provenance::Ingest { .. }) {
            let reference = edge
                .props
                .get("last_corroborated_at")
                .and_then(serde_json::Value::as_str)
                .or(edge.valid_from.as_deref());
            // Undated ingest ground truth is treated as within the floor
            // (conservative: we cannot prove it is old enough to retire).
            let age = reference.and_then(|r| age_in_days(r, now));
            if age.is_none_or(|a| a < self.ingest_retention_floor_days) {
                return Some(format!(
                    "ingest ground-truth within retention floor ({} days)",
                    self.ingest_retention_floor_days
                ));
            }
        }
        None
    }
}

// ── Cold-plane move + recall (the reversible machinery) ──────────────────────

/// Read the `forgotten` audit block from an edge's props, if present.
///
/// An edge is COLD exactly when this returns `Some(..)`.  Used by the proposal
/// validator (to reject re-forgetting) and by recall (to find cold edges).
#[must_use]
pub fn edge_forgotten_block(props: &serde_json::Value) -> Option<&serde_json::Value> {
    props.get(FORGOTTEN_PROPS_KEY).filter(|v| !v.is_null())
}

/// Normalize a `YYYY-MM-DD` date to the RFC 3339 UTC `valid_to` form.
///
/// The graph engine's `is_timestampish` validator requires the
/// `YYYY-MM-DDT00:00:00Z` form.  Mirrors the K-M4 `date_to_utc_rfc3339` helper
/// (kept local to avoid a `tdw-infer` dep cycle).
#[must_use]
pub fn date_to_utc_rfc3339(date: &str) -> String {
    if date.len() >= 20 && date.ends_with('Z') {
        return date.to_string();
    }
    format!("{date}T00:00:00Z")
}

/// Outcome of a single cold-move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetireOutcome {
    /// The `from` node of the retired edge.
    pub from: String,
    /// The relationship type.
    pub rel: String,
    /// The `to` node of the retired edge.
    pub to: String,
    /// The timestamp the edge's validity was closed at (RFC 3339 UTC).
    pub closed_at: String,
}

/// RETIRE an active `(from, rel, to)` edge to the COLD plane (K-R8 req 2).
///
/// This is the forgetting primitive — a MOVE, never a delete:
///
/// 1. Finds the matching ACTIVE (open `valid_to`, not already cold) edge.
/// 2. Closes its validity window at `now` (the same `replace_edges` temporal-
///    close the K-M4 detector uses) so history and `as_of` queries over past
///    time still return it.
/// 3. Annotates props with a `forgotten` audit block recording `reason`,
///    `forgotten_at`, and the gated `provenance` string.
///
/// Fully REVERSIBLE via [`recall_cold_edge`].
///
/// # Errors
///
/// [`KnowledgeError::Storage`] when the graph engine rejects a scan or
/// `replace_edges`, or when no active edge matches (we never claim a fact was
/// forgotten when it was not).
pub async fn retire_edge_to_cold(
    graph: &Arc<dyn GraphEngine>,
    from: &str,
    rel: &str,
    to: &str,
    reason: &str,
    provenance_text: &str,
    now: &str,
) -> Result<RetireOutcome> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let close_at = date_to_utc_rfc3339(now);

    // Scan the (from, rel, *) neighborhood for the matching active edge.
    let mut edges = scan_subject_rel(graph, from, rel).await?;
    let Some(target_idx) = edges.iter().position(|e| {
        e.to == to && e.valid_to.is_none() && edge_forgotten_block(&e.props).is_none()
    }) else {
        return Err(KnowledgeError::Storage(format!(
            "retire: no active (open, non-cold) edge {from} -{rel}-> {to} to retire"
        )));
    };

    // Same-timestamp guard (mirrors K-M4): closing at `valid_from` would make
    // an empty [T, T) window the engine rejects.  Push the close one step past
    // valid_from by recording the cold flag but keeping the window valid — we
    // close at max(now, valid_from-not-equal).  In practice `now` is injected
    // by the worker and is strictly after historical valid_from; if they are
    // equal we still must produce a non-empty window, so we annotate the
    // forgotten block but DO NOT alter valid_to (the cold tag alone retires it
    // from active reads while the historical window stays intact).
    let mut target = edges[target_idx].clone();
    let same_instant = target
        .valid_from
        .as_deref()
        .is_some_and(|vf| date_to_utc_rfc3339(vf) >= close_at);

    let mut props_map = match target.props.clone() {
        serde_json::Value::Object(map) => map,
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("_original".to_string(), other);
            m
        }
    };
    props_map.insert(
        FORGOTTEN_PROPS_KEY.to_string(),
        json!({
            "reason": reason,
            "forgotten_at": close_at,
            "provenance": provenance_text,
            "prior_valid_to": target.valid_to,
        }),
    );
    target.props = serde_json::Value::Object(props_map);
    if !same_instant {
        target.valid_to = Some(close_at.clone());
    }

    let identity = edge_identity(&edges[target_idx]);
    for edge in &mut edges {
        if edge_identity(edge) == identity {
            *edge = target.clone();
        }
    }
    graph
        .replace_edges(from, rel, edges)
        .await
        .map_err(storage)?;

    Ok(RetireOutcome {
        from: from.to_string(),
        rel: rel.to_string(),
        to: to.to_string(),
        closed_at: close_at,
    })
}

/// RECALL a cold `(from, rel, to)` edge back to the active plane (K-R8 req 2).
///
/// The reverse of [`retire_edge_to_cold`]: strips the `forgotten` block and
/// re-opens validity (`valid_to = None`, restoring `prior_valid_to` only if it
/// was a genuine close, not the cold-tag close).  This is the gated restore
/// path — callers route it through the proposal/operator surface so a recall is
/// as auditable as the forgetting was.  The historical cold record is NOT
/// destroyed: recall produces an active edge again; prior `as_of` queries are
/// unaffected because the edge identity `(from, rel, to, valid_from)` is
/// preserved.
///
/// # Errors
///
/// [`KnowledgeError::Storage`] on engine errors, or when no cold edge matches.
pub async fn recall_cold_edge(
    graph: &Arc<dyn GraphEngine>,
    from: &str,
    rel: &str,
    to: &str,
    now: &str,
) -> Result<RetireOutcome> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());

    let mut edges = scan_subject_rel(graph, from, rel).await?;
    let Some(target_idx) = edges
        .iter()
        .position(|e| e.to == to && edge_forgotten_block(&e.props).is_some())
    else {
        return Err(KnowledgeError::Storage(format!(
            "recall: no cold edge {from} -{rel}-> {to} to restore"
        )));
    };

    let mut target = edges[target_idx].clone();
    // Restore the validity bound the edge had BEFORE it was forgotten.
    let prior_valid_to = edge_forgotten_block(&target.props)
        .and_then(|b| b.get("prior_valid_to"))
        .and_then(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(str::to_string)
            }
        });
    if let serde_json::Value::Object(map) = &mut target.props {
        map.remove(FORGOTTEN_PROPS_KEY);
        map.insert("recalled_at".to_string(), json!(date_to_utc_rfc3339(now)));
    }
    target.valid_to = prior_valid_to;

    let identity = edge_identity(&edges[target_idx]);
    for edge in &mut edges {
        if edge_identity(edge) == identity {
            *edge = target.clone();
        }
    }
    graph
        .replace_edges(from, rel, edges)
        .await
        .map_err(storage)?;

    Ok(RetireOutcome {
        from: from.to_string(),
        rel: rel.to_string(),
        to: to.to_string(),
        closed_at: date_to_utc_rfc3339(now),
    })
}

// ── Candidate identification (the sweep core) ────────────────────────────────

/// One identified forgetting candidate: the edge triple + its assessment.
#[derive(Clone, Debug, PartialEq)]
pub struct ForgettingCandidate {
    /// The `from` node.
    pub from: String,
    /// The relationship type.
    pub rel: String,
    /// The `to` node.
    pub to: String,
    /// The policy assessment (signals + score) that made it a candidate.
    pub assessment: ForgettingAssessment,
}

/// Report from one hygiene sweep.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HygieneReport {
    /// Candidates identified and (by the caller) enqueued as proposals.
    pub candidates: Vec<ForgettingCandidate>,
    /// Edges scanned.
    pub scanned: usize,
    /// `true` when the per-sweep cap truncated the candidate list (more
    /// candidates exist than were proposed this sweep).
    pub capped: bool,
}

/// Scan the whole graph for forgetting candidates under `policy`, bounded by
/// `cap` (K-R8 req 1 + per-sweep cap).
///
/// Read-only: identifies candidates and computes their assessments; it does
/// NOT retire anything (that is the gated proposal's job).  Only ACTIVE
/// (open `valid_to`), non-cold edges are considered.  Confidence and
/// corroboration are computed from the same K-R6 machinery the rest of the
/// system uses, so the fail-safe guards see the real numbers.
///
/// Candidates are returned in deterministic scan order, truncated to `cap`.
///
/// # Errors
///
/// [`KnowledgeError::Storage`] on graph engine errors.
pub async fn identify_candidates(
    graph: &Arc<dyn GraphEngine>,
    policy: &ForgettingPolicy,
    cap: usize,
    now: &str,
) -> Result<HygieneReport> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let mut report = HygieneReport::default();

    let mut offset = 0_usize;
    loop {
        let page = graph.edges(None, offset, PAGE).await.map_err(storage)?;
        let page_len = page.len();
        if page_len == 0 {
            break;
        }
        offset += page_len;

        for edge in &page {
            report.scanned += 1;
            // Only active, non-cold edges are forgetting candidates.
            if edge.valid_to.is_some() || edge_forgotten_block(&edge.props).is_some() {
                continue;
            }
            // Compute K-R6 confidence + corroboration over the same-(from,rel)
            // pool so the guards see real numbers.  The pool is the page plus a
            // bounded same-rel fetch — but to stay bounded we corroborate
            // within a scoped scan of this subject+rel.
            let pool = scan_subject_rel(graph, &edge.from, &edge.rel)
                .await
                .unwrap_or_default();
            let pool_refs: Vec<&GraphEdge> = pool.iter().take(MAX_CORROBORATION_CAP).collect();
            let input = EdgeConfidenceInput::from_edge(edge);
            let score = compute_confidence(&input, &pool_refs, None);
            let corroboration = count_independent_sources(&input, &pool_refs);

            let assessment = policy.evaluate(edge, score.confidence, corroboration, now);
            if policy.is_candidate(&assessment) {
                report.candidates.push(ForgettingCandidate {
                    from: edge.from.clone(),
                    rel: edge.rel.clone(),
                    to: edge.to.clone(),
                    assessment,
                });
                if report.candidates.len() >= cap {
                    report.capped = true;
                    return Ok(report);
                }
            }
        }

        if page_len < PAGE {
            break;
        }
    }

    Ok(report)
}

// ── Shared helpers ───────────────────────────────────────────────────────────

/// Fetch only the outgoing `(from, rel, *)` edges for one subject (bounded by
/// the subject's out-degree, not the global relation size).  Mirrors the K-M4
/// `scan_rel_for_subject` bound.
async fn scan_subject_rel(
    graph: &Arc<dyn GraphEngine>,
    subject: &str,
    rel: &str,
) -> Result<Vec<GraphEdge>> {
    use tdw_core::{Direction, TraversalFilter};
    let filter = TraversalFilter {
        rels: Some(vec![rel.to_owned()]),
        direction: Direction::Out,
        ..TraversalFilter::default()
    };
    let pairs = graph
        .neighbors(subject, &filter)
        .await
        .map_err(|e| KnowledgeError::Storage(e.to_string()))?;
    Ok(pairs.into_iter().map(|(edge, _node)| edge).collect())
}

/// Edge identity for replace-in-place: `(from, rel, to, valid_from)`.
fn edge_identity(edge: &GraphEdge) -> (String, String, String, Option<String>) {
    (
        edge.from.clone(),
        edge.rel.clone(),
        edge.to.clone(),
        edge.valid_from.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_core::{GraphEdge, GraphNode, Provenance};
    use tdw_taxonomy::EntityKind;

    fn node(id: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            kind: EntityKind::Instrument,
            label: id.to_string(),
            aliases: Vec::new(),
            props: serde_json::Value::Null,
            valid_from: None,
            valid_to: None,
        }
    }

    /// `valid_from` is a `YYYY-MM-DD` date; the graph engine requires the full
    /// RFC 3339 UTC form, so it is normalized here (matching production edges).
    fn ingest_edge(from: &str, rel: &str, to: &str, source: &str, valid_from: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Ingest {
                source: source.to_string(),
            },
            valid_from: Some(date_to_utc_rfc3339(valid_from)),
            valid_to: None,
        }
    }

    async fn seeded(
        nodes: &[&str],
        edges: &[GraphEdge],
    ) -> Arc<tdw_storage_graph::InMemoryGraphEngine> {
        let graph = Arc::new(tdw_storage_graph::InMemoryGraphEngine::default());
        graph
            .upsert_nodes(nodes.iter().map(|id| node(id)).collect())
            .await
            .expect("seed nodes");
        if !edges.is_empty() {
            graph
                .upsert_edges(edges.to_vec())
                .await
                .expect("seed edges");
        }
        graph
    }

    // ── date math ────────────────────────────────────────────────────────────

    #[test]
    fn days_from_civil_round_trips_with_civil_from_days() {
        for &(y, m, d) in &[
            (1970, 1, 1),
            (2024, 1, 2),
            (2024, 2, 29), // leap day — month boundary stress
            (2000, 12, 31),
            (1999, 3, 1),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(tdw_core::date::civil_from_days(days), (y, m, d));
        }
    }

    #[test]
    fn age_in_days_spans_year_boundary_correctly() {
        // 2023-12-31 → 2024-01-01 is exactly 1 day, not 0 and not ~30.
        assert_eq!(age_in_days("2023-12-31", "2024-01-01"), Some(1));
        // A full leap year (2024) is 366 days.
        assert_eq!(age_in_days("2024-01-01", "2025-01-01"), Some(366));
        // Future reference clamps to 0.
        assert_eq!(age_in_days("2025-01-01", "2024-01-01"), Some(0));
    }

    // ── (a) decision enqueues as a proposal, not an immediate delete ─────────

    #[tokio::test]
    async fn forgetting_decision_enqueues_as_proposal_not_delete() {
        use crate::proposals::{ProposalKind, ProposalQueue, ValidationStatus};
        use tdw_tags::InMemoryTagEngine;
        use tdw_taxonomy::Adaptivity;

        let stale = ingest_edge("company:ACME", "ceo_of", "person:old", "src:a", "2018-01-01");
        let graph = seeded(&["company:ACME", "person:old"], &[stale]).await;
        let graph: Arc<dyn GraphEngine> = graph;
        let tags: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());

        let mut queue = ProposalQueue::default();
        let proposal = queue
            .submit(
                "hygiene-worker",
                Adaptivity::Learning,
                ProposalKind::Forget {
                    from: "company:ACME".to_string(),
                    rel: "ceo_of".to_string(),
                    to: "person:old".to_string(),
                    reason: "governed-forgetting (K-R8): score=0.70".to_string(),
                },
                &graph,
                &tags,
                "2024-06-01",
            )
            .await
            .expect("forget proposal submits");

        // It is a PENDING proposal — NOT materialized, the edge is untouched.
        assert_eq!(proposal.status, ValidationStatus::Validated);
        assert!(!proposal.materialized);
        let edges = graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .expect("read edges");
        assert_eq!(edges.len(), 1, "edge must NOT be deleted by a proposal");
        assert!(
            edges[0].valid_to.is_none() && edge_forgotten_block(&edges[0].props).is_none(),
            "edge must still be active — nothing retired until materialize"
        );
    }

    // ── gate end-to-end: a materialized Forget proposal moves to cold ────────

    #[tokio::test]
    async fn materializing_a_forget_proposal_moves_the_edge_to_cold() {
        use crate::proposals::{ProposalKind, ProposalQueue};
        use tdw_tags::InMemoryTagEngine;
        use tdw_taxonomy::Adaptivity;

        let edge = ingest_edge("company:ACME", "ceo_of", "person:old", "src:a", "2010-01-01");
        let graph = seeded(&["company:ACME", "person:old"], &[edge]).await;
        let graph: Arc<dyn GraphEngine> = graph;
        let tags: Arc<dyn tdw_tags::TagEngine> = Arc::new(InMemoryTagEngine::default());

        let mut queue = ProposalQueue::default();
        let p = queue
            .submit(
                "hygiene-worker",
                Adaptivity::Learning,
                ProposalKind::Forget {
                    from: "company:ACME".to_string(),
                    rel: "ceo_of".to_string(),
                    to: "person:old".to_string(),
                    reason: "governed-forgetting (K-R8): score=0.70".to_string(),
                },
                &graph,
                &tags,
                "2024-06-01",
            )
            .await
            .expect("submit");
        // Promote (human approval path) then materialize.
        queue.approve(&p.id, "operator", "2024-06-01").expect("approve");
        let report = queue
            .materialize_ready(&graph, &tags, "2024-06-01")
            .await
            .expect("materialize");
        assert_eq!(report.materialized.len(), 1, "the forget proposal landed");

        // The edge is now COLD — closed + tagged forgotten, NOT deleted.
        let edges = graph.edges(Some("ceo_of"), 0, 256).await.expect("read");
        assert_eq!(edges.len(), 1, "edge retired, never deleted");
        assert!(
            edge_forgotten_block(&edges[0].props).is_some(),
            "materialized Forget must move the edge to cold"
        );
        assert!(edges[0].valid_to.is_some(), "validity window closed");
    }

    // ── (b) stale low-confidence fact → moved to cold, as_of still returns ───

    #[tokio::test]
    async fn stale_low_confidence_moved_to_cold_and_as_of_still_returns_it() {
        let stale = GraphEdge {
            props: json!({ "extraction_confidence": 0.2 }),
            ..ingest_edge("company:ACME", "hq_in", "city:old", "src:a", "2010-01-01")
        };
        let graph = seeded(&["company:ACME", "city:old"], &[stale]).await;
        let graph: Arc<dyn GraphEngine> = graph;

        // Retire it (simulating a materialized Forget proposal).
        let outcome = retire_edge_to_cold(
            &graph,
            "company:ACME",
            "hq_in",
            "city:old",
            "governed-forgetting (K-R8): score=0.80",
            "agent:hygiene-worker;proposal:p1",
            "2024-06-01",
        )
        .await
        .expect("retire to cold");
        assert_eq!(outcome.closed_at, "2024-06-01T00:00:00Z");

        // The edge is NOT deleted — it is closed + tagged forgotten.
        let all = graph.edges(Some("hq_in"), 0, 256).await.expect("read");
        assert_eq!(all.len(), 1, "cold = closed, never deleted");
        let cold = &all[0];
        assert_eq!(cold.valid_to.as_deref(), Some("2024-06-01T00:00:00Z"));
        let block = edge_forgotten_block(&cold.props).expect("forgotten block present");
        assert!(
            block
                .get("reason")
                .and_then(|v| v.as_str())
                .expect("reason string")
                .contains("K-R8"),
            "audit reason recorded"
        );

        // as_of a PAST date (2015) — the fact was true then, so it MUST return.
        let active_2015 = tdw_core::active_at(
            cold.valid_from.as_deref(),
            cold.valid_to.as_deref(),
            "2015-06-01T00:00:00Z",
        );
        assert!(
            active_2015,
            "a forgotten fact MUST still be visible to as_of queries over past time"
        );
        // as_of NOW (2024-07) — the fact is no longer active.
        let active_now = tdw_core::active_at(
            cold.valid_from.as_deref(),
            cold.valid_to.as_deref(),
            "2024-07-01T00:00:00Z",
        );
        assert!(!active_now, "a forgotten fact is not active in the present");
    }

    // ── (c) high-confidence / corroborated fact is NEVER forgotten ───────────

    #[test]
    fn high_confidence_fact_is_never_forgotten_failsafe() {
        let policy = ForgettingPolicy::default();
        // Ancient + would-be-stale, but confidence is above the protect floor.
        let edge = ingest_edge("company:ACME", "sector", "sector:tech", "src:a", "2000-01-01");
        let assessment = policy.evaluate(&edge, 0.95, 1, "2024-06-01");
        assert!(
            assessment.protected_reason.is_some(),
            "high-confidence guard must fire"
        );
        assert!(
            !policy.is_candidate(&assessment),
            "high-confidence fact must NEVER be a candidate regardless of staleness"
        );
    }

    #[test]
    fn well_corroborated_fact_is_never_forgotten_failsafe() {
        let policy = ForgettingPolicy::default();
        let edge = ingest_edge("company:ACME", "sector", "sector:tech", "src:a", "2000-01-01");
        // 4 independent sources ≥ protect floor of 3 → protected.
        let assessment = policy.evaluate(&edge, 0.1, 4, "2024-06-01");
        assert!(assessment.protected_reason.is_some());
        assert!(!policy.is_candidate(&assessment));
    }

    #[test]
    fn fresh_ingest_ground_truth_within_floor_is_never_forgotten() {
        let policy = ForgettingPolicy::default();
        // Ingest fact 10 days old, far below the 180-day retention floor.
        let edge = ingest_edge("company:ACME", "ceo_of", "person:new", "src:a", "2024-05-22");
        let assessment = policy.evaluate(&edge, 0.1, 1, "2024-06-01");
        assert!(
            assessment.protected_reason.is_some(),
            "ingest retention floor must protect fresh ground truth"
        );
        assert!(!policy.is_candidate(&assessment));
    }

    // ── (d) a cold fact can be recalled / restored through the gate ──────────

    #[tokio::test]
    async fn cold_fact_can_be_recalled_and_restored() {
        let edge = ingest_edge("company:ACME", "hq_in", "city:old", "src:a", "2010-01-01");
        let graph = seeded(&["company:ACME", "city:old"], &[edge]).await;
        let graph: Arc<dyn GraphEngine> = graph;

        retire_edge_to_cold(
            &graph, "company:ACME", "hq_in", "city:old", "reason", "agent:w;proposal:p1",
            "2024-06-01",
        )
        .await
        .expect("retire");

        // Confirm cold.
        let cold_edges = graph.edges(Some("hq_in"), 0, 256).await.expect("read");
        let cold = &cold_edges[0];
        assert!(edge_forgotten_block(&cold.props).is_some());
        assert!(cold.valid_to.is_some());

        // Recall it back to active.
        recall_cold_edge(&graph, "company:ACME", "hq_in", "city:old", "2024-08-01")
            .await
            .expect("recall");

        let restored_edges = graph.edges(Some("hq_in"), 0, 256).await.expect("read");
        let restored = &restored_edges[0];
        assert!(
            edge_forgotten_block(&restored.props).is_none(),
            "forgotten block stripped on recall"
        );
        assert!(
            restored.valid_to.is_none(),
            "recall re-opens validity (restored to active)"
        );
        assert!(
            restored.props.get("recalled_at").is_some(),
            "recall is audited"
        );
    }

    // ── (e) per-sweep cap honored ────────────────────────────────────────────

    #[tokio::test]
    async fn per_sweep_cap_truncates_candidate_list() {
        // Five stale, low-confidence, single-source facts — all candidates.
        let mut edges = Vec::new();
        let mut nodes = vec!["company:ACME"];
        for i in 0..5 {
            // leak owned strings into 'static via Box::leak only in test scope
            let to: &'static str = Box::leak(format!("person:p{i}").into_boxed_str());
            nodes.push(to);
            edges.push(GraphEdge {
                props: json!({ "extraction_confidence": 0.1 }),
                rel: format!("rel{i}"),
                ..ingest_edge("company:ACME", "tmp", to, "src:a", "2005-01-01")
            });
        }
        let graph = seeded(&nodes, &edges).await;
        let graph: Arc<dyn GraphEngine> = graph;

        let policy = ForgettingPolicy::default();
        let report = identify_candidates(&graph, &policy, 2, "2024-06-01")
            .await
            .expect("sweep");
        assert_eq!(report.candidates.len(), 2, "cap=2 truncates");
        assert!(report.capped, "capped flag must signal more remain");
    }

    // ── kill-switch is the worker's concern; here assert the policy threshold ─

    #[test]
    fn score_below_threshold_is_not_a_candidate() {
        let policy = ForgettingPolicy {
            forget_score_threshold: 0.9,
            ..ForgettingPolicy::default()
        };
        // A mildly stale, mid-confidence fact scores below 0.9.
        let edge = ingest_edge("company:ACME", "ceo_of", "person:x", "src:a", "2023-06-01");
        let assessment = policy.evaluate(&edge, 0.5, 1, "2024-06-01");
        assert!(
            !policy.is_candidate(&assessment),
            "score below threshold must not be a candidate"
        );
    }
}
