//! Contradiction-driven temporal invalidation (knowledge-system K-M4).
//!
//! [`ContradictionDetector`] fires on every new-edge arrival (via the
//! post-ingest and post-materialize hooks already wired by K-L1).  For each
//! arriving edge whose relation is in the functional-predicate set it checks
//! whether an **active** edge with the same `(from, rel)` but a **different**
//! `to` already exists.  When it does, the old edge is **temporally
//! invalidated**: its `valid_to` is closed at the new fact's `as_of` timestamp
//! via the atomic [`GraphEngine::replace_edges`] path (history preserved —
//! point-in-time queries before the close still return the old fact).
//!
//! # What "active" means
//!
//! An edge is active at `as_of` when its `[valid_from, valid_to)` window
//! covers `as_of`: `valid_from ≤ as_of < valid_to` (open bounds pass).  The
//! check uses [`tdw_core::active_at`] directly.
//!
//! # Trust-class matrix
//!
//! | Old-edge provenance                | Action                                      |
//! |------------------------------------|---------------------------------------------|
//! | `Ingest`                           | Close `valid_to` (temporal invalidation)    |
//! | `Agent { gated: true }`            | Close `valid_to` (treated like Ingest)      |
//! | `System`                           | Close `valid_to` (platform bookkeeping)     |
//! | `Agent { gated: false }` (finding) | **Surface conflict** — never auto-close     |
//! | `Manual`                           | **Surface conflict** — never auto-close     |
//! | `Rule`                             | **Skip** — B7 retraction handles derived    |
//!
//! # Provenance on closures
//!
//! When an edge is closed, its props are annotated with
//! `invalidated_by: "<new-edge-ref>"` where the ref is
//! `"<from>-><rel>-><to>@<as_of>"`.  The [`GraphEngine::replace_edges`] call
//! rewrites the old edge in-place (same `from`, `rel`, `valid_from`; new
//! `valid_to` + updated props).  A `tdw.kg.why` query on the old edge will
//! surface the `invalidated_by` annotation in its props.
//!
//! # Idempotency / same-`to` re-assertion
//!
//! A new fact whose `to` equals the existing active edge's `to` is a
//! **re-assertion** — it is silently skipped.  No invalidation, no conflict.
//!
//! # Derived-edge non-interference
//!
//! Edges with `Provenance::Rule` are **never touched** by this detector.
//! Rule-derived edges are managed by [`tdw_infer::InferEngine::retract`] (B7
//! machinery).  Double-touching them would corrupt the derivation index.

use std::sync::Arc;

use serde_json::json;
use tdw_core::{GraphEdge, GraphEngine, Provenance, active_at};
use tdw_taxonomy::FunctionalPredicateSet;

/// Page size for graph-edge scans inside the contradiction detector.
const PAGE: usize = 256;

/// What happened to one arriving edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidationOutcome {
    /// The old edge was closed at `as_of`.
    Invalidated {
        /// The `from` node (same for old and new edge).
        from: String,
        /// The functional relation.
        rel: String,
        /// The old `to` node that was superseded.
        old_to: String,
        /// The new `to` node that superseded it.
        new_to: String,
        /// The timestamp the old edge was closed at.
        closed_at: String,
    },
    /// The old edge has user or manual provenance — conflict surfaced, not
    /// auto-resolved.
    Conflict {
        /// The `from` node.
        from: String,
        /// The functional relation.
        rel: String,
        /// The old `to` node.
        old_to: String,
        /// The new `to` node that contradicts it.
        new_to: String,
        /// The old edge's provenance class (for logging).
        old_provenance_class: &'static str,
    },
    /// The new edge is a re-assertion of the existing value — no action.
    Idempotent,
    /// The arriving edge's relation is not in the functional-predicate set —
    /// no contradiction check performed.
    NotFunctional,
}

/// One run's aggregate report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContradictionReport {
    /// Edges closed (old facts temporally invalidated).
    pub invalidated: usize,
    /// Edges that created unresolvable conflicts (user/manual provenance on
    /// the old edge) — surfaced as loud logs, not auto-closed.
    pub conflicts: usize,
}

/// Errors from the contradiction detector.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContradictionError {
    /// A graph engine call failed.
    #[error("graph error: {0}")]
    Graph(String),
}

/// The contradiction detector.
///
/// Constructed from a [`FunctionalPredicateSet`] (taxonomy defaults +
/// operator extras from `[knowledge.contradiction]`); the graph engine is
/// passed per-call so the detector can be shared across the ingest and
/// materialize hooks.
#[derive(Debug, Clone)]
pub struct ContradictionDetector {
    predicates: FunctionalPredicateSet,
}

impl ContradictionDetector {
    /// Construct from the given functional-predicate set.
    #[must_use]
    pub const fn new(predicates: FunctionalPredicateSet) -> Self {
        Self { predicates }
    }

    /// Construct from taxonomy defaults only (no operator extension).
    #[must_use]
    pub fn default_predicates() -> Self {
        Self {
            predicates: FunctionalPredicateSet::default(),
        }
    }

    /// The functional-predicate set this detector uses.
    #[must_use]
    pub const fn predicates(&self) -> &FunctionalPredicateSet {
        &self.predicates
    }

    /// Check one arriving edge for functional-predicate contradiction and, if
    /// found, close the superseded edge's validity window.
    ///
    /// `as_of` is the arrival timestamp (RFC 3339 UTC, same format the graph
    /// uses for `valid_from`/`valid_to`).  It is used as:
    /// - the `as_of` point for the "is there an active conflicting edge?"
    ///   check, and
    /// - the `valid_to` written onto the superseded edge.
    ///
    /// # Errors
    ///
    /// Returns [`ContradictionError::Graph`] if the graph engine rejects a
    /// `edges` scan or `replace_edges` call.
    pub async fn check_and_invalidate(
        &self,
        graph: &Arc<dyn GraphEngine>,
        arriving: &GraphEdge,
        as_of: &str,
    ) -> Result<InvalidationOutcome, ContradictionError> {
        // Fast path: skip non-functional relations immediately.
        if !self.predicates.is_functional(&arriving.rel) {
            return Ok(InvalidationOutcome::NotFunctional);
        }

        // Scan all edges of this relation type for an active (from, rel, ≠to)
        // pair.  The graph's `edges` API pages by relation type; we scan until
        // we find a conflict or exhaust the relation.
        let mut offset = 0_usize;
        loop {
            let page = graph
                .edges(Some(&arriving.rel), offset, PAGE)
                .await
                .map_err(|e| ContradictionError::Graph(e.to_string()))?;
            let page_len = page.len();

            for existing in page {
                // Must match the same subject.
                if existing.from != arriving.from {
                    continue;
                }
                // Must be an active edge at as_of (closed edges are historical
                // records; they must not trigger idempotency or conflict checks).
                if !active_at(
                    existing.valid_from.as_deref(),
                    existing.valid_to.as_deref(),
                    as_of,
                ) {
                    continue;
                }
                // Same (from, rel, to) among active edges = re-assertion — skip.
                if existing.to == arriving.to {
                    return Ok(InvalidationOutcome::Idempotent);
                }
                // Found an active conflicting edge.  Apply trust-class rules.
                return self.resolve(graph, existing, arriving, as_of).await;
            }

            if page_len < PAGE {
                break;
            }
            offset += PAGE;
        }

        // No active conflicting edge found — nothing to do.
        Ok(InvalidationOutcome::Idempotent)
    }

    /// Process a batch of arriving edges, returning the aggregate report.
    ///
    /// Each edge is checked independently; errors on one edge are collected and
    /// returned as the first error encountered (the batch is best-effort up to
    /// that point).
    ///
    /// # Errors
    ///
    /// Returns the first [`ContradictionError`] encountered.
    pub async fn check_batch(
        &self,
        graph: &Arc<dyn GraphEngine>,
        arriving: &[GraphEdge],
        as_of: &str,
    ) -> Result<ContradictionReport, ContradictionError> {
        let mut report = ContradictionReport::default();
        for edge in arriving {
            match self.check_and_invalidate(graph, edge, as_of).await? {
                InvalidationOutcome::Invalidated {
                    ref from,
                    ref rel,
                    ref old_to,
                    ref new_to,
                    ref closed_at,
                } => {
                    eprintln!(
                        "tdw-infer contradiction: ({from})-[{rel}]->({old_to}) \
                         superseded by ({from})-[{rel}]->({new_to}); \
                         old edge closed at {closed_at}"
                    );
                    report.invalidated += 1;
                }
                InvalidationOutcome::Conflict {
                    ref from,
                    ref rel,
                    ref old_to,
                    ref new_to,
                    old_provenance_class,
                } => {
                    eprintln!(
                        "tdw-infer contradiction CONFLICT (not auto-resolved): \
                         ({from})-[{rel}]->({old_to}) [{old_provenance_class}] \
                         vs new ({from})-[{rel}]->({new_to}); \
                         manual review required"
                    );
                    report.conflicts += 1;
                }
                InvalidationOutcome::Idempotent | InvalidationOutcome::NotFunctional => {}
            }
        }
        Ok(report)
    }

    /// Scan the entire graph for functional-predicate contradictions and close
    /// any superseded edges found.
    ///
    /// For every predicate `P` in the functional set, scans all `(from, P, *)`
    /// edges grouped by `from`.  When more than one ACTIVE edge exists for the
    /// same `(from, P)` pair, all but the newest `valid_from` are closed at
    /// `as_of`.  This is the post-batch sweep path called by
    /// `fire_contradiction_after_ingest` and `fire_contradiction_after_materialize`.
    ///
    /// # Errors
    ///
    /// Returns the first [`ContradictionError`] encountered.
    pub async fn scan_all_functional(
        &self,
        graph: &Arc<dyn GraphEngine>,
        as_of: &str,
    ) -> Result<ContradictionReport, ContradictionError> {
        let mut report = ContradictionReport::default();
        // Iterate over every functional predicate in sorted order for
        // determinism.  For each, collect all active edges grouped by `from`.
        for rel in self.predicates.iter_sorted() {
            let mut offset = 0_usize;
            // Group active edges by `from` so we can detect multi-`to` conflicts.
            let mut by_from: std::collections::BTreeMap<String, Vec<GraphEdge>> =
                std::collections::BTreeMap::new();
            loop {
                let page = graph
                    .edges(Some(rel), offset, PAGE)
                    .await
                    .map_err(|e| ContradictionError::Graph(e.to_string()))?;
                let page_len = page.len();
                for edge in page {
                    if active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), as_of) {
                        by_from.entry(edge.from.clone()).or_default().push(edge);
                    }
                }
                if page_len < PAGE {
                    break;
                }
                offset += PAGE;
            }

            // For any `from` with multiple active edges (multi-`to`), keep the
            // newest `valid_from` and close the rest.  "Newest" = lexicographically
            // largest `valid_from` (or None = open start, kept as-is if only one
            // has a None start — determinism: prefer `Some` over `None` when
            // both are present, last-writer-wins among `Some`s).
            for (from, mut active_edges) in by_from {
                if active_edges.len() <= 1 {
                    continue;
                }
                // Sort descending by valid_from (None < any Some for our purposes:
                // a None-start edge is treated as the oldest, not the newest).
                active_edges.sort_by(|a, b| match (&b.valid_from, &a.valid_from) {
                    (Some(bv), Some(av)) => bv.cmp(av),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                });
                // The first element is the "newest" (winner); all others are
                // superseded.
                let winner = active_edges[0].clone();
                for superseded in active_edges.into_iter().skip(1) {
                    let outcome = self.resolve(graph, superseded, &winner, as_of).await?;
                    match &outcome {
                        InvalidationOutcome::Invalidated { old_to, new_to, .. } => {
                            eprintln!(
                                "tdw-infer contradiction scan: ({from})-[{rel}]->({old_to}) \
                                 superseded by ({from})-[{rel}]->({new_to}); \
                                 closed at {as_of}"
                            );
                            report.invalidated += 1;
                        }
                        InvalidationOutcome::Conflict {
                            old_to,
                            new_to,
                            old_provenance_class,
                            ..
                        } => {
                            eprintln!(
                                "tdw-infer contradiction CONFLICT (not auto-resolved): \
                                 ({from})-[{rel}]->({old_to}) [{old_provenance_class}] \
                                 vs ({from})-[{rel}]->({new_to}); manual review required"
                            );
                            report.conflicts += 1;
                        }
                        InvalidationOutcome::Idempotent | InvalidationOutcome::NotFunctional => {}
                    }
                }
            }
        }
        Ok(report)
    }

    /// Apply the trust-class matrix to a found conflict and either close the
    /// old edge or surface it as a conflict.
    async fn resolve(
        &self,
        graph: &Arc<dyn GraphEngine>,
        mut existing: GraphEdge,
        arriving: &GraphEdge,
        as_of: &str,
    ) -> Result<InvalidationOutcome, ContradictionError> {
        let new_edge_ref = format!(
            "{}->{}->{new_to}@{as_of}",
            arriving.from,
            arriving.rel,
            new_to = arriving.to
        );

        match &existing.provenance {
            // Rule-derived edges: skip entirely — B7 retraction is the right
            // primitive.  Double-touching them corrupts the derivation index.
            Provenance::Rule { .. } => Ok(InvalidationOutcome::Idempotent),

            // User finding (gated: false) and Manual: surface as conflict,
            // never auto-close.
            Provenance::Agent { gated: false, .. } => Ok(InvalidationOutcome::Conflict {
                from: existing.from,
                rel: existing.rel,
                old_to: existing.to,
                new_to: arriving.to.clone(),
                old_provenance_class: "agent:ungated",
            }),
            Provenance::Manual { .. } => Ok(InvalidationOutcome::Conflict {
                from: existing.from,
                rel: existing.rel,
                old_to: existing.to,
                new_to: arriving.to.clone(),
                old_provenance_class: "manual",
            }),

            // Ingest, gated-Agent, System: close the old edge.
            Provenance::Ingest { .. }
            | Provenance::Agent { gated: true, .. }
            | Provenance::System { .. } => {
                let old_to = existing.to.clone();
                let from = existing.from.clone();
                let rel = existing.rel.clone();

                // Annotate props with supersession lineage so tdw.kg.why
                // surfaces it: merge the existing props object with the
                // invalidated_by annotation.
                let mut props = match existing.props.clone() {
                    serde_json::Value::Object(map) => map,
                    serde_json::Value::Null => serde_json::Map::new(),
                    other => {
                        let mut m = serde_json::Map::new();
                        m.insert("_original".to_string(), other);
                        m
                    }
                };
                props.insert("invalidated_by".to_string(), json!(new_edge_ref));
                existing.props = serde_json::Value::Object(props);

                // Close the validity window at as_of.  If the edge had no
                // valid_to yet (open-ended), set it; if it already had one
                // that is later than as_of, bring it in (should not happen
                // normally, but we never widen a window).
                existing.valid_to = Some(as_of.to_string());

                // Atomic replace: fetch all (from, rel, *) edges for this
                // specific subject, replace the matching one (by identity:
                // from+rel+to+valid_from), write the updated set back.
                // Filter to only this subject's edges — `replace_edges`
                // validates that every edge in the slice has the same
                // (from, rel) as the call arguments.
                let all_edges = scan_rel_full(graph, &rel).await?;
                let identity = edge_identity(&existing);
                let updated: Vec<GraphEdge> = all_edges
                    .into_iter()
                    .filter(|e| e.from == from)
                    .map(|e| {
                        if edge_identity(&e) == identity {
                            existing.clone()
                        } else {
                            e
                        }
                    })
                    .collect();

                graph
                    .replace_edges(&from, &rel, updated)
                    .await
                    .map_err(|e| ContradictionError::Graph(e.to_string()))?;

                Ok(InvalidationOutcome::Invalidated {
                    from,
                    rel,
                    old_to,
                    new_to: arriving.to.clone(),
                    closed_at: as_of.to_string(),
                })
            }
        }
    }
}

/// Scan ALL edges of one relation type (all pages).
async fn scan_rel_full(
    graph: &Arc<dyn GraphEngine>,
    rel: &str,
) -> Result<Vec<GraphEdge>, ContradictionError> {
    let mut all = Vec::new();
    let mut offset = 0_usize;
    loop {
        let page = graph
            .edges(Some(rel), offset, PAGE)
            .await
            .map_err(|e| ContradictionError::Graph(e.to_string()))?;
        let page_len = page.len();
        all.extend(page);
        if page_len < PAGE {
            break;
        }
        offset += PAGE;
    }
    Ok(all)
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
    use std::sync::Arc;

    use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance};
    use tdw_taxonomy::EntityKind;

    use super::{ContradictionDetector, InvalidationOutcome};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn ts(s: &str) -> String {
        s.to_string()
    }

    /// Build a minimal GraphNode for a test entity.
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

    fn ingest_edge(from: &str, rel: &str, to: &str, valid_from: Option<&str>) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Ingest {
                source: "test".to_string(),
            },
            valid_from: valid_from.map(str::to_string),
            valid_to: None,
        }
    }

    fn user_edge(from: &str, rel: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Agent {
                agent_id: "analyst".to_string(),
                gated: false,
            },
            valid_from: None,
            valid_to: None,
        }
    }

    fn manual_edge(from: &str, rel: &str, to: &str) -> GraphEdge {
        GraphEdge {
            from: from.to_string(),
            to: to.to_string(),
            rel: rel.to_string(),
            props: serde_json::Value::Null,
            provenance: Provenance::Manual {
                approved_by: "operator".to_string(),
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
            props: serde_json::Value::Null,
            provenance: Provenance::Rule {
                rule_id: "r1".to_string(),
                version: 1,
            },
            valid_from: None,
            valid_to: None,
        }
    }

    /// Seed the graph with nodes and edges, return the engine.
    async fn seeded_graph(
        nodes: &[&str],
        edges: &[GraphEdge],
    ) -> Arc<tdw_storage_graph::InMemoryGraphEngine> {
        let graph = Arc::new(tdw_storage_graph::InMemoryGraphEngine::default());
        graph
            .upsert_nodes(nodes.iter().map(|id| node(id)).collect())
            .await
            .unwrap();
        if !edges.is_empty() {
            graph.upsert_edges(edges.to_vec()).await.unwrap();
        }
        graph
    }

    // -----------------------------------------------------------------------
    // Canonical fixture: CEO supersession
    // -----------------------------------------------------------------------

    /// The canonical K-M4 fixture:
    ///
    /// 1. CEO(company:ACME) = person:alice at t1
    /// 2. Ingest CEO(company:ACME) = person:bob at t2
    ///    → alice-edge validity closed at t2
    ///    → as_of t1.5 query returns alice (history preserved)
    ///    → current (as_of t3) query returns bob
    ///    → why on old alice edge shows `invalidated_by` prop
    #[tokio::test]
    async fn canonical_ceo_supersession() {
        let t1 = "2024-01-01T00:00:00Z";
        let t2 = "2024-06-01T00:00:00Z";
        let t3 = "2025-01-01T00:00:00Z";
        let t_mid = "2024-03-01T00:00:00Z"; // t1.5: between t1 and t2

        let alice_edge = ingest_edge("company:ACME", "ceo_of", "person:alice", Some(t1));

        let graph = seeded_graph(
            &["company:ACME", "person:alice", "person:bob"],
            &[alice_edge.clone()],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let arriving_bob = ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2));

        let outcome = detector
            .check_and_invalidate(&graph, &arriving_bob, t2)
            .await
            .expect("no graph error");

        assert!(
            matches!(
                outcome,
                InvalidationOutcome::Invalidated {
                    ref from,
                    ref rel,
                    ref old_to,
                    ref new_to,
                    ref closed_at,
                } if from == "company:ACME"
                  && rel == "ceo_of"
                  && old_to == "person:alice"
                  && new_to == "person:bob"
                  && closed_at == t2
            ),
            "expected Invalidated outcome, got {outcome:?}"
        );

        // --- Point-in-time query: as_of t_mid → alice still active ----------
        let edges_mid: Vec<_> = graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.from == "company:ACME"
                    && tdw_core::active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), t_mid)
            })
            .collect();
        assert_eq!(edges_mid.len(), 1, "exactly one ceo_of active at t_mid");
        assert_eq!(edges_mid[0].to, "person:alice", "alice active at t_mid");

        // --- Current query: as_of t3 → bob active (after we add bob's edge) -
        graph
            .upsert_edges(vec![arriving_bob.clone()])
            .await
            .unwrap();
        let edges_current: Vec<_> = graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| {
                e.from == "company:ACME"
                    && tdw_core::active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), t3)
            })
            .collect();
        assert_eq!(edges_current.len(), 1, "exactly one ceo_of active at t3");
        assert_eq!(edges_current[0].to, "person:bob", "bob active at t3");

        // --- History preserved: alice edge is closed, not deleted ------------
        let all_ceo: Vec<_> = graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.from == "company:ACME")
            .collect();
        let alice_final = all_ceo
            .iter()
            .find(|e| e.to == "person:alice")
            .expect("alice edge still exists (history preserved)");
        assert_eq!(
            alice_final.valid_to.as_deref(),
            Some(t2),
            "alice edge closed at t2"
        );

        // --- Why on old edge shows invalidated_by ---------------------------
        let inv_by = alice_final
            .props
            .get("invalidated_by")
            .and_then(|v| v.as_str())
            .expect("invalidated_by annotation present");
        assert!(
            inv_by.contains("person:bob"),
            "invalidated_by names the superseding fact: {inv_by}"
        );
    }

    // -----------------------------------------------------------------------
    // Same-to re-assertion is idempotent
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn same_to_reassertion_is_idempotent() {
        let t1 = "2024-01-01T00:00:00Z";
        let graph = seeded_graph(
            &["company:ACME", "person:alice"],
            &[ingest_edge(
                "company:ACME",
                "ceo_of",
                "person:alice",
                Some(t1),
            )],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let same_edge = ingest_edge("company:ACME", "ceo_of", "person:alice", Some(t1));

        let outcome = detector
            .check_and_invalidate(&graph, &same_edge, t1)
            .await
            .unwrap();
        assert_eq!(outcome, InvalidationOutcome::Idempotent);
    }

    // -----------------------------------------------------------------------
    // User-provenance edges are NEVER auto-invalidated — surfaced as conflict
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn user_provenance_edge_surfaced_as_conflict_not_auto_closed() {
        let t2 = "2024-06-01T00:00:00Z";

        let graph = seeded_graph(
            &["company:ACME", "person:alice", "person:bob"],
            &[user_edge("company:ACME", "ceo_of", "person:alice")],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let arriving = ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2));

        let outcome = detector
            .check_and_invalidate(&graph, &arriving, t2)
            .await
            .unwrap();

        assert!(
            matches!(
                outcome,
                InvalidationOutcome::Conflict {
                    old_provenance_class: "agent:ungated",
                    ..
                }
            ),
            "user edge must surface as conflict, not be auto-closed: {outcome:?}"
        );

        // Verify the old edge is UNCHANGED (valid_to still None).
        let edges = graph.edges(Some("ceo_of"), 0, 256).await.unwrap();
        let alice = edges
            .iter()
            .find(|e| e.to == "person:alice")
            .expect("alice edge still present");
        assert!(
            alice.valid_to.is_none(),
            "user-authored edge must not be auto-closed"
        );
    }

    // -----------------------------------------------------------------------
    // Manual-provenance edges are surfaced as conflict
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn manual_provenance_edge_surfaced_as_conflict() {
        let t2 = "2024-06-01T00:00:00Z";
        let graph = seeded_graph(
            &["company:ACME", "person:alice", "person:bob"],
            &[manual_edge("company:ACME", "ceo_of", "person:alice")],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let arriving = ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2));

        let outcome = detector
            .check_and_invalidate(&graph, &arriving, t2)
            .await
            .unwrap();
        assert!(
            matches!(
                outcome,
                InvalidationOutcome::Conflict {
                    old_provenance_class: "manual",
                    ..
                }
            ),
            "manual edge must surface as conflict: {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // Rule-derived edges are not touched
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rule_derived_edge_not_invalidated() {
        let t2 = "2024-06-01T00:00:00Z";
        let graph = seeded_graph(
            &["company:ACME", "person:alice", "person:bob"],
            &[rule_edge("company:ACME", "ceo_of", "person:alice")],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let arriving = ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2));

        let outcome = detector
            .check_and_invalidate(&graph, &arriving, t2)
            .await
            .unwrap();
        // Rule-derived edges are idempotent-skipped by the detector.
        assert_eq!(
            outcome,
            InvalidationOutcome::Idempotent,
            "rule-derived edges must not be touched"
        );
    }

    // -----------------------------------------------------------------------
    // Non-functional relation returns NotFunctional immediately
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn non_functional_rel_skipped() {
        let graph = seeded_graph(&["a:1", "b:2"], &[]).await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let edge = ingest_edge("a:1", "mentions", "b:2", None);

        let outcome = detector
            .check_and_invalidate(&graph, &edge, "2024-01-01T00:00:00Z")
            .await
            .unwrap();
        assert_eq!(outcome, InvalidationOutcome::NotFunctional);
    }

    // -----------------------------------------------------------------------
    // Re-assert A at t3 after supersession: B closed, A re-opened as new edge
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reassert_original_after_supersession_creates_new_edge() {
        let t1 = "2024-01-01T00:00:00Z";
        let t2 = "2024-06-01T00:00:00Z";
        let t3 = "2025-01-01T00:00:00Z";

        let alice_edge = ingest_edge("company:ACME", "ceo_of", "person:alice", Some(t1));
        let graph = seeded_graph(
            &["company:ACME", "person:alice", "person:bob"],
            &[alice_edge],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();

        // Step 1: bob supersedes alice at t2.
        let bob_edge = ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2));
        detector
            .check_and_invalidate(&graph, &bob_edge, t2)
            .await
            .unwrap();
        graph.upsert_edges(vec![bob_edge]).await.unwrap();

        // Step 2: alice re-asserted at t3 (new edge, not resurrection).
        let alice_t3 = ingest_edge("company:ACME", "ceo_of", "person:alice", Some(t3));
        let outcome = detector
            .check_and_invalidate(&graph, &alice_t3, t3)
            .await
            .unwrap();

        // Bob's edge is now closed at t3.
        assert!(
            matches!(
                outcome,
                InvalidationOutcome::Invalidated {
                    ref old_to,
                    ref new_to,
                    ..
                } if old_to == "person:bob" && new_to == "person:alice"
            ),
            "bob should be superseded when alice re-asserted: {outcome:?}"
        );

        // The original alice-edge (t1..t2) is distinct from the new alice-edge
        // (t3..open) — history has two alice records.
        graph.upsert_edges(vec![alice_t3]).await.unwrap();
        let all: Vec<_> = graph
            .edges(Some("ceo_of"), 0, 256)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.from == "company:ACME" && e.to == "person:alice")
            .collect();
        assert_eq!(all.len(), 2, "two alice edges: original and re-asserted");
        let original = all
            .iter()
            .find(|e| e.valid_from.as_deref() == Some(t1))
            .unwrap();
        let reasserted = all
            .iter()
            .find(|e| e.valid_from.as_deref() == Some(t3))
            .unwrap();
        assert_eq!(
            original.valid_to.as_deref(),
            Some(t2),
            "original alice closed at t2"
        );
        assert!(
            reasserted.valid_to.is_none(),
            "re-asserted alice is open-ended"
        );
    }

    // -----------------------------------------------------------------------
    // ContradictionReport aggregates correctly
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn check_batch_aggregates_counts() {
        let t1 = "2024-01-01T00:00:00Z";
        let t2 = "2024-06-01T00:00:00Z";

        let graph = seeded_graph(
            &[
                "company:ACME",
                "person:alice",
                "person:bob",
                "company:BETA",
                "person:carol",
                "person:dave",
            ],
            &[
                // One functional conflict (ingest → auto-close)
                ingest_edge("company:ACME", "ceo_of", "person:alice", Some(t1)),
                // One user-provenance conflict
                user_edge("company:BETA", "ceo_of", "person:carol"),
            ],
        )
        .await;
        let graph: Arc<dyn tdw_core::GraphEngine> = graph;

        let detector = ContradictionDetector::default_predicates();
        let batch = vec![
            ingest_edge("company:ACME", "ceo_of", "person:bob", Some(t2)),
            ingest_edge("company:BETA", "ceo_of", "person:dave", Some(t2)),
        ];

        let report = detector.check_batch(&graph, &batch, t2).await.unwrap();
        assert_eq!(report.invalidated, 1, "one ingest edge invalidated");
        assert_eq!(report.conflicts, 1, "one user conflict surfaced");
    }
}
