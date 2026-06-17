#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Rule-based forward-chaining inference over the graph + tag substrate (knowledge-system B7).
//!
//! K-M4 adds [`contradiction`] (contradiction-driven temporal invalidation):
//! when a new fact arrives for a functional predicate and an active conflicting
//! fact exists, the old fact's validity window is closed (history preserved).
//!
//! [`InferEngine`] holds a stratified, versioned, hot-reloadable rule set
//! ([`InferRule`]) and materializes derived facts:
//!
//! - [`InferRule::DeriveEdge`] joins a head-to-tail edge chain into a new edge, written via
//!   [`GraphEngine::upsert_edges`] with [`Provenance::Rule`].
//! - [`InferRule::PropagateTag`] flows a tag along edges, written via [`TagEngine::assign`]
//!   with provenance `derived:rule:<id>@v<version>`.
//!
//! # Termination
//!
//! Rules are monotone (they only ADD facts) and stratified — a rule may consume a derived
//! edge type only from a STRICTLY LOWER stratum (enforced at [`InferEngine::hot_reload`]).
//! Over a finite fact universe a monotone stratified set reaches a fixpoint.
//! [`RunLimits`] is defense-in-depth: exceeding either bound is an ERROR
//! ([`InferError::DerivedLimitExceeded`] / [`InferError::IterationLimitExceeded`]), never
//! silent truncation.
//!
//! # Honesty about the implementation
//!
//! [`InferEngine::run_full`] is a FULL re-scan per iteration with a no-new-facts
//! termination check — NOT semi-naive evaluation. It is correct and terminating for the
//! monotone stratified universe; it is simply not incremental within a stratum.
//!
//! [`InferEngine::run_incremental`] re-fires only rules whose inputs intersect the change
//! set. It UNDER-APPROXIMATES retraction: it never removes facts that a deletion should
//! have invalidated — the documented safe fallback for true retraction is a full re-run
//! from a clean graph.
//!
//! [`InferEngine::retract`] deletes derived EDGES whose support transitively includes a
//! retracted fact. It CANNOT remove derived TAG assignments: `tdw-tags` assignments are
//! append-only and there is no unassign primitive. Such tags are reported in
//! [`RetractReport::unremovable_tags`]; the caller's documented fallback is a full re-run
//! from a clean graph.

pub mod contradiction;
pub mod index;
pub mod rule;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tdw_core::{Direction, GraphEdge, GraphEngine, Provenance, TraversalFilter};
use tdw_tags::{TagAssignment, TagEngine};
use thiserror::Error;

pub use index::{Derivation, DerivationIndex, edge_key, tag_key};
pub use rule::{EdgePattern, InferRule, validate_rule};

/// Page size for the [`GraphEngine::edges`] scan that backs chain matching.
const EDGE_PAGE: usize = 256;

/// Defense-in-depth limits on one inference run.
///
/// Exceeding EITHER bound is an [`InferError`], never silent truncation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunLimits {
    /// Maximum stratum-iterations across the whole run.
    pub max_iterations: usize,
    /// Maximum NEW facts (edges + tags) the run may derive.
    pub max_derived: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_iterations: 32,
            max_derived: 10_000,
        }
    }
}

/// What one inference run produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InferReport {
    /// New edges derived this run.
    pub derived_edges: usize,
    /// New tag assignments derived this run.
    pub assigned_tags: usize,
    /// Stratum-iterations performed.
    pub iterations: usize,
    /// The rule-set version that fired.
    pub version: u64,
}

/// What one [`InferEngine::retract`] removed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetractReport {
    /// Derived-edge keys whose graph edge was NOT deleted: the triple was
    /// absent, or every matching edge carried non-`Rule` provenance (a base
    /// fact this engine must never destroy). The index entry is still
    /// dropped. Also the safety net against a forged
    /// [`DerivationIndex::from_json`]: a restored index cannot make
    /// `retract` delete base edges.
    pub preserved_base_edges: Vec<String>,
    /// Derived edges deleted from the graph.
    pub removed_edges: usize,
    /// Derived TAGS that could NOT be removed (append-only store, no unassign).
    /// The caller's documented fallback is a full re-run from a clean graph.
    pub unremovable_tags: Vec<String>,
}

/// The inputs that changed since the last run — the trigger set for
/// [`InferEngine::run_incremental`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    /// Edge relationship types whose facts changed.
    pub edge_types: BTreeSet<String>,
    /// Tags whose assignments changed.
    pub tags: BTreeSet<String>,
}

/// Inference errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InferError {
    /// A rule failed shape or grammar validation.
    #[error("invalid rule {rule_id}: {reason}")]
    InvalidRule {
        /// The offending rule id.
        rule_id: String,
        /// What was wrong.
        reason: &'static str,
    },
    /// Two rules violate the strict-lower stratification contract.
    #[error(
        "unstratifiable: rule {consumer} consumes derived type {derived_type} produced by {producer} at stratum {producer_stratum} (consumer stratum {consumer_stratum})"
    )]
    Unstratifiable {
        /// The consuming rule.
        consumer: String,
        /// The consumer's stratum.
        consumer_stratum: u8,
        /// The producing rule.
        producer: String,
        /// The producer's stratum.
        producer_stratum: u8,
        /// The derived type at issue.
        derived_type: String,
    },
    /// A chain join materialized more candidates than the engine's memory
    /// bound (8 × `max_derived`, floor 4096) — a fan-out explosion guard.
    #[error("chain-join materialization bound exceeded: {bound}")]
    JoinBoundExceeded {
        /// The bound that was exceeded.
        bound: usize,
    },
    /// The run derived more facts than [`RunLimits::max_derived`].
    #[error("max_derived limit exceeded: {limit}")]
    DerivedLimitExceeded {
        /// The limit that was hit.
        limit: usize,
    },
    /// The run performed more stratum-iterations than [`RunLimits::max_iterations`].
    #[error("max_iterations limit exceeded: {limit}")]
    IterationLimitExceeded {
        /// The limit that was hit.
        limit: usize,
    },
    /// A graph engine call failed.
    #[error("graph error: {0}")]
    Graph(String),
    /// A tag engine call failed.
    #[error("tag error: {0}")]
    Tag(String),
    /// (De)serialization of the derivation index failed.
    #[error("serde error: {0}")]
    Serde(String),
}

/// The forward-chaining inference engine.
///
/// # User-authored fact exclusion (K-X6)
///
/// `exclude_user_authored` (default `true`) prevents `Provenance::Agent {
/// gated: false }` edges — the edges written by the `tdw.kg.finding` /
/// `tdw.kg.link` tools — from participating in `DeriveEdge` chain matching.
/// The boundary is intentional:
///
/// * **Auto-TAGGING of findings via `PropagateTag` rules is allowed** — it
///   aids retrieval and mirrors how other graph nodes are tagged.  A
///   `PropagateTag` rule walks outbound edges FROM a tag-holder TO reachable
///   nodes; this traversal does NOT consume the finding's own edges as
///   chain-join inputs, so the exclusion does not affect it.
///
/// * **`DeriveEdge` derivations from user-authored edges are blocked** by
///   default.  A user finding that happens to mention the same entities a
///   `DeriveEdge` rule matches would silently mint new derived facts — facts
///   the operator never sanctioned.  Set `exclude_user_authored = false` (opt-
///   in) only if an operator explicitly wants user findings to feed a rule.
#[derive(Debug)]
pub struct InferEngine {
    rules: Vec<InferRule>,
    version: u64,
    limits: RunLimits,
    derivations: DerivationIndex,
    /// When `true` (the default), edges with `Provenance::Agent { gated:
    /// false }` are excluded from `DeriveEdge` chain matching so user-authored
    /// findings do not drive inference by default.
    pub exclude_user_authored: bool,
}

impl Default for InferEngine {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            version: 0,
            limits: RunLimits::default(),
            derivations: DerivationIndex::default(),
            exclude_user_authored: true,
        }
    }
}

impl InferEngine {
    /// A fresh engine with the given limits and no rules.
    ///
    /// `exclude_user_authored` defaults to `true` — user-provenance edges are
    /// not used as `DeriveEdge` chain-join inputs.  Call
    /// [`Self::with_user_authored_inference`] to override.
    #[must_use]
    pub fn with_limits(limits: RunLimits) -> Self {
        Self {
            rules: Vec::new(),
            version: 0,
            limits,
            derivations: DerivationIndex::default(),
            exclude_user_authored: true,
        }
    }

    /// Override the user-authored edge exclusion policy.
    ///
    /// Set to `false` only when an operator explicitly wants `Provenance::Agent
    /// { gated: false }` edges (user findings) to participate in `DeriveEdge`
    /// chain matching.  The default is `true`.
    #[must_use]
    pub const fn with_user_authored_inference(mut self, allow: bool) -> Self {
        self.exclude_user_authored = !allow;
        self
    }

    /// The current rule-set version.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// The maintained derivation index (persist via [`DerivationIndex::to_json`]).
    #[must_use]
    pub const fn derivation_index(&self) -> &DerivationIndex {
        &self.derivations
    }

    /// Chain-join materialization ceiling: generous headroom over
    /// `max_derived` (candidates legitimately include already-derived
    /// triples), floored so tiny test limits do not strangle the join.
    const fn join_bound(&self) -> usize {
        let scaled = self.limits.max_derived.saturating_mul(8);
        if scaled > 4096 { scaled } else { 4096 }
    }

    /// Resume from a persisted derivation index ([`DerivationIndex::from_json`]).
    ///
    /// The index is TRUSTED state from the operator's own persistence — but a
    /// forged or stale index still cannot destroy base facts: `retract`
    /// verifies `Rule` provenance on the graph edge before deleting anything.
    #[must_use]
    pub fn with_derivation_index(mut self, derivations: DerivationIndex) -> Self {
        self.derivations = derivations;
        self
    }

    /// Return the currently installed rule set (read-only).
    ///
    /// Callers that need to build an updated set (e.g. the K-R5 induction
    /// worker) clone this slice, push new rules, then call
    /// [`hot_reload`](Self::hot_reload).
    #[must_use]
    pub fn rules(&self) -> &[InferRule] {
        &self.rules
    }

    /// Atomically replace the rule set: EVERY rule validates (shape + stratification) or
    /// NOTHING changes; success bumps the version (mirrors `tdw-tag-rules`).
    ///
    /// Stratification: a rule may reference (consume) a derived edge type produced by
    /// another rule ONLY if that producer's stratum is STRICTLY LOWER. A same-or-higher
    /// reference is [`InferError::Unstratifiable`]. This applies to `DeriveEdge.when` rels
    /// AND `PropagateTag.along` edge types, for the same termination argument.
    ///
    /// # Errors
    ///
    /// Returns the first validation/stratification failure without mutating the engine.
    pub fn hot_reload(&mut self, rules: Vec<InferRule>) -> Result<(), InferError> {
        for rule in &rules {
            validate_rule(rule)?;
        }
        validate_stratification(&rules)?;
        self.rules = rules;
        self.version += 1;
        Ok(())
    }

    /// Shape-fail-soft variant of [`hot_reload`](Self::hot_reload): rules that
    /// fail per-rule [`validate_rule`] are SKIPPED (and returned) instead of
    /// failing the whole reload, so a single malformed rule cannot drop an
    /// entire batch (e.g. an induction cycle's promotions).
    ///
    /// Set-level stratification stays ATOMIC over the surviving rules: if the
    /// shape-valid set is unstratifiable the engine is left UNCHANGED and the
    /// error is returned (dropping individual rules to break a cycle is not
    /// attempted). On success returns the skipped `(rule_id, reason)` pairs.
    ///
    /// Use the strict [`hot_reload`](Self::hot_reload) for config loads that must
    /// reject loudly; use this for best-effort installs of generated rules.
    ///
    /// # Errors
    ///
    /// [`InferError::Unstratifiable`] (or another non-`InvalidRule` validation
    /// error) if the surviving rule set fails the set-level check; the engine is
    /// not mutated.
    pub fn hot_reload_lenient(
        &mut self,
        rules: Vec<InferRule>,
    ) -> Result<Vec<(String, &'static str)>, InferError> {
        let mut kept = Vec::with_capacity(rules.len());
        let mut skipped = Vec::new();
        for rule in rules {
            match validate_rule(&rule) {
                Ok(()) => kept.push(rule),
                Err(InferError::InvalidRule { rule_id, reason }) => skipped.push((rule_id, reason)),
                Err(other) => return Err(other),
            }
        }
        validate_stratification(&kept)?;
        self.rules = kept;
        self.version += 1;
        Ok(skipped)
    }

    /// Run every rule to a fixpoint, stratum by stratum (ascending).
    ///
    /// Within each stratum a FULL re-scan iterates until no new fact is derived (NOT
    /// semi-naive). Every new fact counts against `limits.max_derived`; every
    /// stratum-iteration against `limits.max_iterations`.
    ///
    /// # Errors
    ///
    /// Returns a limit error, or a propagated graph/tag engine error.
    pub async fn run_full(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<InferReport, InferError> {
        let fire: BTreeSet<usize> = (0..self.rules.len()).collect();
        self.run(graph, tags, now, &fire).await
    }

    /// Re-run ONLY rules whose inputs intersect `changed`.
    ///
    /// A `DeriveEdge` fires when `changed.edge_types` mentions any of its `when` rels; a
    /// `PropagateTag` fires when `changed.tags` mentions its base `tag` OR
    /// `changed.edge_types` intersects its `along` edge types.
    ///
    /// This UNDER-APPROXIMATES retraction: it adds the facts a change newly justifies but
    /// never removes facts a deletion invalidated. The documented safe fallback for true
    /// retraction is a full re-run from a clean graph.
    ///
    /// # Errors
    ///
    /// Returns a limit error, or a propagated graph/tag engine error.
    pub async fn run_incremental(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
        changed: &ChangeSet,
    ) -> Result<InferReport, InferError> {
        let fire: BTreeSet<usize> = self
            .rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule_triggered(rule, changed))
            .map(|(index, _)| index)
            .collect();
        self.run(graph, tags, now, &fire).await
    }

    /// Delete every derived fact whose support TRANSITIVELY includes `fact_key`.
    ///
    /// Derived edges are removed via [`GraphEngine::delete_edges`]. Derived TAGS cannot be
    /// removed — `tdw-tags` is append-only with no unassign primitive — so they are
    /// reported in [`RetractReport::unremovable_tags`]; the caller's documented fallback is
    /// a full re-run from a clean graph.
    ///
    /// # Errors
    ///
    /// Returns a propagated graph engine error.
    pub async fn retract(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        _tags: &Arc<dyn TagEngine>,
        fact_key: &str,
    ) -> Result<RetractReport, InferError> {
        // Transitive support closure: every derived fact whose support chain bottoms out
        // at fact_key. A derived fact f is invalidated when fact_key appears in f's
        // support, OR any already-invalidated derived fact does.
        let mut invalid: BTreeSet<String> = BTreeSet::new();
        let mut frontier = vec![fact_key.to_string()];
        while let Some(current) = frontier.pop() {
            for (key, derivation) in self.derivations.iter() {
                if invalid.contains(key) {
                    continue;
                }
                if derivation.support.iter().any(|input| input == &current)
                    && invalid.insert(key.clone())
                {
                    frontier.push(key.clone());
                }
            }
        }

        let mut report = RetractReport::default();
        for key in &invalid {
            if let Some((from, derived_type, to)) = parse_edge_key(key) {
                // Surgical removal: only Rule-provenance edges may be deleted.
                // `delete_edges` keys on (from, rel, to) alone and would
                // destroy a same-shaped BASE fact (3-lens review B1) —
                // instead, atomically replace the node's (from, rel, *) set
                // with the non-derived survivors.
                let mut survivors = Vec::new();
                let mut removed = 0_usize;
                // Retract scans for Rule-provenance edges only — never exclude user-authored
                // here because the filter would leave retracted derived edges alive.
                for edge in scan_rel(graph, derived_type, false).await? {
                    if edge.from != from {
                        continue;
                    }
                    if edge.to == to && matches!(edge.provenance, Provenance::Rule { .. }) {
                        removed += 1;
                    } else {
                        survivors.push(edge);
                    }
                }
                if removed > 0 {
                    graph
                        .replace_edges(from, derived_type, survivors)
                        .await
                        .map_err(|error| InferError::Graph(error.to_string()))?;
                    report.removed_edges += removed;
                } else {
                    // Indexed as derived but absent or shadowed by a
                    // non-derived edge — preserved, never deleted.
                    report.preserved_base_edges.push(key.clone());
                }
                self.derivations.remove(key);
            } else if key.starts_with("tag:") {
                // Append-only store: cannot unassign. Report and keep the
                // index entry so a later full re-run can reconcile honestly.
                report.unremovable_tags.push(key.clone());
            }
        }
        report.unremovable_tags.sort();
        report.unremovable_tags.dedup();
        report.preserved_base_edges.sort();
        report.preserved_base_edges.dedup();
        Ok(report)
    }

    /// Drive the fixpoint over the rules selected in `fire`, stratum by stratum.
    async fn run(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
        fire: &BTreeSet<usize>,
    ) -> Result<InferReport, InferError> {
        let mut report = InferReport {
            version: self.version,
            ..InferReport::default()
        };

        // Strata in ascending order; each rule's outputs are visible to higher strata
        // because edges land in the graph before the next stratum runs.
        let mut strata: Vec<u8> = self.rules.iter().map(InferRule::stratum).collect();
        strata.sort_unstable();
        strata.dedup();

        // Per-RUN bound: facts derived by THIS run, not the engine's lifetime
        // accumulation — a long-lived engine must not trip the limit on a run
        // that adds almost nothing (3-lens review N1).
        let mut derived_this_run = 0_usize;
        for stratum in strata {
            loop {
                if report.iterations >= self.limits.max_iterations {
                    return Err(InferError::IterationLimitExceeded {
                        limit: self.limits.max_iterations,
                    });
                }
                report.iterations += 1;
                let mut new_this_iteration = 0_usize;
                for index in 0..self.rules.len() {
                    if !fire.contains(&index) || self.rules[index].stratum() != stratum {
                        continue;
                    }
                    let rule = self.rules[index].clone();
                    let derived = match &rule {
                        InferRule::DeriveEdge { .. } => self.fire_derive_edge(graph, &rule).await?,
                        InferRule::PropagateTag { .. } => {
                            self.fire_propagate_tag(graph, tags, &rule, now).await?
                        }
                    };
                    new_this_iteration += derived.edges + derived.tags;
                    derived_this_run += derived.edges + derived.tags;
                    report.derived_edges += derived.edges;
                    report.assigned_tags += derived.tags;
                    if derived_this_run > self.limits.max_derived {
                        return Err(InferError::DerivedLimitExceeded {
                            limit: self.limits.max_derived,
                        });
                    }
                }
                if new_this_iteration == 0 {
                    break;
                }
            }
        }
        Ok(report)
    }

    /// One pass of a `DeriveEdge` rule: scan + join its chain, write new edges.
    async fn fire_derive_edge(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        rule: &InferRule,
    ) -> Result<Derived, InferError> {
        let InferRule::DeriveEdge {
            rule_id,
            when,
            derived_type,
            ..
        } = rule
        else {
            return Ok(Derived::default());
        };

        let matches = self
            .match_chain(graph, when, self.exclude_user_authored)
            .await?;
        // Existing (from, to) pairs of the derived type — base facts, or facts
        // derived by earlier runs. A derived edge NEVER re-asserts an existing
        // triple: upserting over a same-identity base edge would silently
        // clobber its provenance, and recording it in the index would put a
        // base fact's shape into the retraction set (3-lens review B1).
        let mut existing: BTreeSet<(String, String)> = BTreeSet::new();
        // Existing-edge dedup scan: check all edges of the derived type regardless
        // of provenance so we never re-assert over a same-identity base fact.
        for edge in scan_rel(graph, derived_type, false).await? {
            existing.insert((edge.from.clone(), edge.to.clone()));
        }
        let mut new_edges = Vec::new();
        let mut counted = 0_usize;
        for matched in matches {
            // Skip self-loops: a chain whose endpoints coincide is never derived.
            if matched.from == matched.to {
                continue;
            }
            let key = edge_key(&matched.from, derived_type, &matched.to);
            if self.derivations.contains(&key)
                || existing.contains(&(matched.from.clone(), matched.to.clone()))
            {
                continue;
            }
            new_edges.push(GraphEdge {
                from: matched.from.clone(),
                to: matched.to.clone(),
                rel: derived_type.clone(),
                props: serde_json::Value::Null,
                provenance: Provenance::Rule {
                    rule_id: rule_id.clone(),
                    version: self.version,
                },
                valid_from: None,
                valid_to: None,
            });
            self.derivations.record(
                key,
                Derivation {
                    rule_id: rule_id.clone(),
                    version: self.version,
                    support: matched.support,
                },
            );
            counted += 1;
        }
        if !new_edges.is_empty() {
            graph
                .upsert_edges(new_edges)
                .await
                .map_err(|error| InferError::Graph(error.to_string()))?;
        }
        Ok(Derived {
            edges: counted,
            tags: 0,
        })
    }

    /// One pass of a `PropagateTag` rule: seed from tag holders, walk edges, assign.
    async fn fire_propagate_tag(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        rule: &InferRule,
        now: &str,
    ) -> Result<Derived, InferError> {
        let InferRule::PropagateTag {
            rule_id,
            tag,
            include_descendants,
            along,
            outbound,
            max_hops,
            ..
        } = rule
        else {
            return Ok(Derived::default());
        };

        // Seed tags: the base tag, plus its taxonomy descendants under subsumption.
        let mut seed_tags = vec![tag.clone()];
        if *include_descendants {
            seed_tags.extend(
                tags.descendants(tag)
                    .await
                    .map_err(|error| InferError::Tag(error.to_string()))?,
            );
        }

        let direction = if *outbound {
            Direction::Out
        } else {
            Direction::In
        };
        let filter = TraversalFilter {
            rels: Some(along.clone()),
            direction,
            max_hops: *max_hops,
            ..TraversalFilter::default()
        };

        // For each seed entity (holding any seed tag), walk along `along` up to max_hops.
        // The support of a propagated tag is the seed's tag fact key — the base fact whose
        // retraction must invalidate it.
        let mut counted = 0_usize;
        let mut reached_seen: BTreeSet<(String, String)> = BTreeSet::new();
        for seed_tag in &seed_tags {
            let seeds = tags
                .entities_with_tag(seed_tag, now)
                .await
                .map_err(|error| InferError::Tag(error.to_string()))?;
            for seed in seeds {
                let support_key = tag_key(&seed, seed_tag);
                let subgraph = graph
                    .expand(std::slice::from_ref(&seed), &filter)
                    .await
                    .map_err(|error| InferError::Graph(error.to_string()))?;
                for node in subgraph.nodes {
                    if node.id == seed {
                        continue; // never assign to the seed itself
                    }
                    if !reached_seen.insert((node.id.clone(), tag.clone())) {
                        continue;
                    }
                    let active = tags
                        .active_tags(&node.id, now)
                        .await
                        .map_err(|error| InferError::Tag(error.to_string()))?;
                    if active.contains(tag) {
                        continue; // already active — skip the append-only dup
                    }
                    let fact_key = tag_key(&node.id, tag);
                    if self.derivations.contains(&fact_key) {
                        continue;
                    }
                    tags.assign(TagAssignment {
                        entity_id: node.id.clone(),
                        tag_id: tag.clone(),
                        assigned_at: now.to_string(),
                        expires_at: None,
                        provenance: format!("derived:rule:{rule_id}@v{}", self.version),
                    })
                    .await
                    .map_err(|error| InferError::Tag(error.to_string()))?;
                    self.derivations.record(
                        fact_key,
                        Derivation {
                            rule_id: rule_id.clone(),
                            version: self.version,
                            support: vec![support_key.clone()],
                        },
                    );
                    counted += 1;
                }
            }
        }
        Ok(Derived {
            edges: 0,
            tags: counted,
        })
    }

    /// Match a head-to-tail edge chain by paging [`GraphEngine::edges`] and joining in
    /// memory. Returns one [`ChainMatch`] per distinct endpoint pair (first `from`, last
    /// `to`) with the support set of the consumed input edges.
    ///
    /// When `exclude_user_authored` is `true`, edges with
    /// `Provenance::Agent { gated: false }` are excluded from chain inputs so
    /// user-authored findings do not drive `DeriveEdge` derivations.
    async fn match_chain(
        &self,
        graph: &Arc<dyn GraphEngine>,
        when: &[EdgePattern],
        exclude_user_authored: bool,
    ) -> Result<Vec<ChainMatch>, InferError> {
        // Partial matches: (current tail node, support so far). Seeded by the first rel.
        let first = &when[0].rel;
        let mut partials: Vec<(String, String, Vec<String>)> = Vec::new();
        for edge in scan_rel(graph, first, exclude_user_authored).await? {
            partials.push((
                edge.from.clone(),
                edge.to.clone(),
                vec![edge_key(&edge.from, &edge.rel, &edge.to)],
            ));
        }
        for pattern in &when[1..] {
            // Index this hop's edges by their `from` so the join is a lookup, not a scan.
            let mut by_from: BTreeMap<String, Vec<GraphEdge>> = BTreeMap::new();
            for edge in scan_rel(graph, &pattern.rel, exclude_user_authored).await? {
                by_from.entry(edge.from.clone()).or_default().push(edge);
            }
            let mut next = Vec::new();
            for (head, tail, support) in partials {
                if let Some(edges) = by_from.get(&tail) {
                    for edge in edges {
                        let mut support = support.clone();
                        support.push(edge_key(&edge.from, &edge.rel, &edge.to));
                        next.push((head.clone(), edge.to.clone(), support));
                        // Bound the join MATERIALIZATION — a memory guard
                        // distinct from max_derived (candidates include
                        // already-derived triples a steady-state re-run
                        // legitimately re-matches, so the fact limit must
                        // not apply here).
                        if next.len() > self.join_bound() {
                            return Err(InferError::JoinBoundExceeded {
                                bound: self.join_bound(),
                            });
                        }
                    }
                }
            }
            partials = next;
        }
        Ok(partials
            .into_iter()
            .map(|(from, to, support)| ChainMatch { from, to, support })
            .collect())
    }
}

/// A derived-fact tally for one rule pass.
#[derive(Clone, Copy, Debug, Default)]
struct Derived {
    edges: usize,
    tags: usize,
}

/// A matched chain: derived-edge endpoints plus the consumed input fact keys.
#[derive(Clone, Debug)]
struct ChainMatch {
    from: String,
    to: String,
    support: Vec<String>,
}

/// Page through every edge of one relationship type.
///
/// When `exclude_user_authored` is `true`, edges with
/// `Provenance::Agent { gated: false }` are dropped — they are user-finding
/// edges that must not participate in `DeriveEdge` chain matching by default.
async fn scan_rel(
    graph: &Arc<dyn GraphEngine>,
    rel: &str,
    exclude_user_authored: bool,
) -> Result<Vec<GraphEdge>, InferError> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let page = graph
            .edges(Some(rel), offset, EDGE_PAGE)
            .await
            .map_err(|error| InferError::Graph(error.to_string()))?;
        let count = page.len();
        for edge in page {
            if exclude_user_authored
                && matches!(edge.provenance, Provenance::Agent { gated: false, .. })
            {
                continue;
            }
            all.push(edge);
        }
        if count < EDGE_PAGE {
            break;
        }
        offset += EDGE_PAGE;
    }
    Ok(all)
}

/// Whether a change set triggers a rule under `run_incremental`.
fn rule_triggered(rule: &InferRule, changed: &ChangeSet) -> bool {
    match rule {
        InferRule::DeriveEdge { when, .. } => when
            .iter()
            .any(|pattern| changed.edge_types.contains(&pattern.rel)),
        InferRule::PropagateTag { tag, along, .. } => {
            changed.tags.contains(tag)
                || along
                    .iter()
                    .any(|edge_type| changed.edge_types.contains(edge_type))
        }
    }
}

/// Cross-rule stratification: a consumed DERIVED type must come from a strictly lower
/// stratum than the consumer. Types not produced by any rule are base facts (ingested) and
/// impose no constraint.
fn validate_stratification(rules: &[InferRule]) -> Result<(), InferError> {
    // producer type -> (rule_id, stratum). A derived type produced by multiple rules takes
    // the LOWEST producer stratum as its "available at" level (any lower producer suffices).
    let mut producers: BTreeMap<&str, (&str, u8)> = BTreeMap::new();
    for rule in rules {
        if let Some(produced) = rule.produced_type() {
            let entry = producers
                .entry(produced)
                .or_insert_with(|| (rule.rule_id(), rule.stratum()));
            if rule.stratum() < entry.1 {
                *entry = (rule.rule_id(), rule.stratum());
            }
        }
    }
    for rule in rules {
        for consumed in rule.consumed_types() {
            if let Some((producer, producer_stratum)) = producers.get(consumed)
                && *producer_stratum >= rule.stratum()
            {
                return Err(InferError::Unstratifiable {
                    consumer: rule.rule_id().to_string(),
                    consumer_stratum: rule.stratum(),
                    producer: (*producer).to_string(),
                    producer_stratum: *producer_stratum,
                    derived_type: consumed.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Parse `edge:<from>-><type>-><to>` back into its three parts (None for tag keys).
fn parse_edge_key(key: &str) -> Option<(&str, &str, &str)> {
    let body = key.strip_prefix("edge:")?;
    let (from, rest) = body.split_once("->")?;
    let (derived_type, to) = rest.split_once("->")?;
    Some((from, derived_type, to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_limits_defaults_match_spec() {
        let limits = RunLimits::default();
        assert_eq!(limits.max_iterations, 32);
        assert_eq!(limits.max_derived, 10_000);
    }

    #[test]
    fn parses_edge_keys_and_skips_tag_keys() {
        assert_eq!(
            parse_edge_key(&edge_key("a", "rel", "b")),
            Some(("a", "rel", "b"))
        );
        assert!(parse_edge_key(&tag_key("e", "risk:x")).is_none());
    }

    #[test]
    fn stratification_rejects_same_stratum_derived_consumption() {
        // r0 produces "mid" at stratum 0; r1 consumes "mid" ALSO at stratum 0 -> reject.
        let rules = vec![
            InferRule::DeriveEdge {
                rule_id: "r0".to_string(),
                stratum: 0,
                when: vec![
                    EdgePattern {
                        rel: "a".to_string(),
                    },
                    EdgePattern {
                        rel: "b".to_string(),
                    },
                ],
                derived_type: "mid".to_string(),
            },
            InferRule::DeriveEdge {
                rule_id: "r1".to_string(),
                stratum: 0,
                when: vec![
                    EdgePattern {
                        rel: "mid".to_string(),
                    },
                    EdgePattern {
                        rel: "c".to_string(),
                    },
                ],
                derived_type: "top".to_string(),
            },
        ];
        assert!(matches!(
            validate_stratification(&rules),
            Err(InferError::Unstratifiable { .. })
        ));
    }

    #[test]
    fn hot_reload_lenient_skips_shape_invalid_and_keeps_the_rest() {
        // One valid rule with a colon-containing (induced) id + one malformed
        // rule: the malformed one is skipped, the valid one installs — a single
        // bad rule must not drop the whole batch.
        let mut engine = InferEngine::default();
        let good = InferRule::DeriveEdge {
            rule_id: "inducted-pattern-a:b".to_string(),
            stratum: 0,
            when: vec![
                EdgePattern {
                    rel: "x".to_string(),
                },
                EdgePattern {
                    rel: "y".to_string(),
                },
            ],
            derived_type: "z".to_string(),
        };
        let bad = InferRule::DeriveEdge {
            rule_id: "bad/id".to_string(),
            stratum: 0,
            when: vec![EdgePattern {
                rel: "x".to_string(),
            }],
            derived_type: "w".to_string(),
        };
        let skipped = engine
            .hot_reload_lenient(vec![good, bad])
            .expect("stratifiable survivors install");
        assert_eq!(skipped.len(), 1, "the malformed rule is skipped, not fatal");
        assert_eq!(skipped[0].0, "bad/id");
        assert_eq!(engine.rules().len(), 1, "the valid colon-id rule installs");
        assert_eq!(engine.rules()[0].rule_id(), "inducted-pattern-a:b");
    }

    #[test]
    fn stratification_allows_strictly_lower_derived_consumption() {
        let rules = vec![
            InferRule::DeriveEdge {
                rule_id: "r0".to_string(),
                stratum: 0,
                when: vec![
                    EdgePattern {
                        rel: "a".to_string(),
                    },
                    EdgePattern {
                        rel: "b".to_string(),
                    },
                ],
                derived_type: "mid".to_string(),
            },
            InferRule::DeriveEdge {
                rule_id: "r1".to_string(),
                stratum: 1,
                when: vec![
                    EdgePattern {
                        rel: "mid".to_string(),
                    },
                    EdgePattern {
                        rel: "c".to_string(),
                    },
                ],
                derived_type: "top".to_string(),
            },
        ];
        assert!(validate_stratification(&rules).is_ok());
    }
}
