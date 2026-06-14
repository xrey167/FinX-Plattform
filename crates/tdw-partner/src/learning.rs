//! The learning-loop read seam (the partner design §3, partner-system W4.1/W4.2).
//!
//! This module is the W4 closure: it is where a *promoted* learning artifact
//! becomes partner *behavior*. The discipline the design mandates is strict —
//! **Partner Core only READS the gated runtime; it never mutates behavior
//! directly** (the partner design §3 "the one rule"). So everything here is a
//! pure read over state the gated subsystems already own:
//!
//! - **W4.1** — [`LearningState::read_from`] snapshots
//!   [`tdw_knowledge::runtime::KnowledgeRuntime::versions`] and resolves the
//!   principal's [`Adaptivity`] through the runtime's
//!   [`AdaptivityResolver`](tdw_knowledge::runtime::AdaptivityResolver). A bumped
//!   `infer_version` (only bumped past the B9 gate, via `update_versions` after a
//!   promotion) changes what the partner does on the next turn — proven by
//!   [`LearningState::infer_generation`] feeding [`apply_to_resolution`].
//! - **W4.2** — [`retrieval_admits`] is the trust-dial → retrieval filter: a
//!   knowledge item scored below the principal's `retrieval_floor` is dropped at
//!   context-assembly time, and learned route preferences (the `self_tune`
//!   outputs the gated runtime exposes as a version bump) re-weight route
//!   resolution via [`apply_to_resolution`].
//!
//! The gate is upstream and total: a version only moves past B9
//! (`KnowledgeRuntime::update_versions` is called *after* a promotion clears the
//! eval/threshold), and a principal only reaches `Learning`+ through the same
//! resolver the write gate consults. So reading here can never bypass a gate —
//! it consumes the gate's already-promoted output.

use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_taxonomy::Adaptivity;

use crate::principal::Principal;
use crate::resolve::ResolvedRoutes;

/// A per-turn snapshot of the gated learning state Partner Core reads
/// (the partner design §3 BEHAVIOR step).
///
/// Built once at the top of a turn from the [`KnowledgeRuntime`] the core
/// already holds, then threaded into the resolve + context steps so a promoted
/// lesson/rule/param takes effect *this* turn. It is a pure value — no I/O, no
/// mutation — so the same runtime snapshot always yields the same behavior.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LearningState {
    /// The inference rule-set generation (`KnowledgeVersions::infer_version`).
    ///
    /// Bumped only past the B9 gate (after an induced rule is promoted and
    /// hot-reloaded, the daemon calls `update_versions`). A change here flips
    /// [`Self::honors_induced_rules`], so a turn run after a promotion honors the
    /// newly induced routing hints while a turn run before it does not.
    pub infer_version: Option<u64>,
    /// The tag-rule-set generation (`KnowledgeVersions::rules_version`).
    ///
    /// Bumped past the same gate; carried so the audit trail can attribute a
    /// behavior change to the exact rule generation that drove it.
    pub rules_version: Option<u64>,
    /// The principal's resolved learning authority.
    ///
    /// Resolved through the runtime's adaptivity resolver — the *same* resolver
    /// the write gate consults — so a principal below [`Adaptivity::Learning`]
    /// reads and answers but neither writes nor has learned preferences applied
    /// to its routing. `None` when the runtime exposes no resolver (learning
    /// off) or the resolver does not know this agent.
    pub adaptivity: Option<Adaptivity>,
    /// Learned route preferences, most-preferred first (W4.2).
    ///
    /// A `self_tune`/induction output the gated runtime surfaces (its presence
    /// is itself a gated, version-bumped signal). Each entry is a catalog route
    /// id (or prefix) the learning loop has validated as worth leading with for
    /// this principal. [`apply_to_resolution`] stably moves a resolved data
    /// route whose id matches a preference ahead of the rest, preserving the
    /// relative order otherwise — so a learned preference *reshapes behavior*,
    /// not just the audit stamp. Empty (the default) leaves routing untouched,
    /// so a partner with no learned preferences resolves at the baseline order.
    pub preferred_routes: Vec<String>,
}

impl LearningState {
    /// Read the current learning state from the gated `runtime` for `principal`
    /// (the partner design §3, W4.1).
    ///
    /// Snapshots the version triple (`versions()` takes a brief read lock) and
    /// resolves the principal's adaptivity through the runtime's resolver. This
    /// is the single read seam — Partner Core calls it once per turn so the most
    /// recently promoted state is the one that drives the turn.
    #[must_use]
    pub fn read_from(runtime: &KnowledgeRuntime, principal: &Principal) -> Self {
        let versions = runtime.versions();
        let adaptivity = runtime
            .adaptivity_resolver()
            .and_then(|resolve| resolve(&principal.agent_id));
        Self {
            infer_version: versions.infer_version,
            rules_version: versions.rules_version,
            adaptivity,
            // The runtime exposes no structured per-principal preference map; the
            // gated daemon populates learned preferences onto the snapshot via
            // [`Self::with_preferred_routes`] from the induction/self_tune output
            // (whose presence is itself a gated version bump). Reading the
            // baseline runtime yields no preferences, so routing is untouched
            // until the gate produces one — the honest "bounded by the gated
            // signal" default.
            preferred_routes: Vec::new(),
        }
    }

    /// Attach learned route preferences (most-preferred first) to this snapshot
    /// (W4.2).
    ///
    /// The gated daemon calls this with the induction/`self_tune` output after a
    /// promotion clears the eval gate, so the preferences carried here are always
    /// the gated, version-bumped signal — never a free-form caller hint. They are
    /// only *applied* when [`Self::honors_induced_rules`] also holds, so a
    /// below-`Learning` principal is never reshaped even if preferences are set.
    #[must_use]
    pub fn with_preferred_routes(mut self, preferred_routes: Vec<String>) -> Self {
        self.preferred_routes = preferred_routes;
        self
    }

    /// Whether learned behavior is applied for this principal at all
    /// (the partner design §3 trust-dial gate).
    ///
    /// Learned preferences and induced rules reach behavior only at
    /// [`Adaptivity::Learning`] or above — mirroring the write gate's admission,
    /// so a low-trust principal's turns are never reshaped by learning.
    #[must_use]
    pub fn learning_active(&self) -> bool {
        self.adaptivity
            .is_some_and(|adaptivity| adaptivity >= Adaptivity::Learning)
    }

    /// Whether induced rules are honored this turn (W4.1/W4.3 link).
    ///
    /// True only when learning is active for the principal AND the gated runtime
    /// reports an inference generation (an `infer_version` is set, i.e. at least
    /// one rule-set has been promoted past B9). A bumped `infer_version`
    /// therefore *enables* induced-rule routing — the concrete "a bumped version
    /// changes behavior" the W4.1 unit pins.
    #[must_use]
    pub fn honors_induced_rules(&self) -> bool {
        self.learning_active() && self.infer_version.is_some()
    }

    /// The inference generation as a behavior key (W4.1).
    ///
    /// `0` when no rule-set is promoted (or learning is inactive); otherwise the
    /// promoted `infer_version`. [`apply_to_resolution`] keys off this, so two
    /// turns at different generations resolve differently — the unit-testable
    /// proof that a version bump changes behavior without any direct mutation.
    #[must_use]
    pub fn infer_generation(&self) -> u64 {
        if self.learning_active() {
            self.infer_version.unwrap_or(0)
        } else {
            0
        }
    }
}

/// Whether a retrieved knowledge item clears the principal's trust floor
/// (the partner design §3 trust-dial → retrieval, W4.2).
///
/// The context step calls this for every candidate: an item whose retrieval
/// `score` is below `principal.trust.retrieval_floor` is dropped before it can
/// ground the answer, so a low-trust principal sees only high-confidence
/// knowledge. A `0.0` floor (the default) admits everything. The comparison is
/// inclusive (`score >= floor`) so an item exactly at the floor is admitted.
#[must_use]
pub fn retrieval_admits(principal: &Principal, score: f64) -> bool {
    score >= principal.trust.retrieval_floor
}

/// Apply the gated learning state to a resolved route set (W4.1/W4.2/W4.3).
///
/// This is the one place a promoted artifact reshapes a turn's behavior, and it
/// is a **pure** transform over already-gated state:
///
/// - When [`LearningState::honors_induced_rules`] holds, the inference
///   generation is recorded onto the resolution (so a downstream surface can
///   attribute the routing to the promoted rule-set, and so a version bump is
///   *observable* in behavior — the W4.1 unit).
/// - Learned route preferences (a `self_tune` output the gated runtime surfaces
///   as a version bump) stably re-order the resolved data routes so a preferred
///   route leads. With learning inactive the resolution is returned untouched.
///
/// Returns the (possibly re-ordered) routes plus the applied inference
/// generation, so the caller can both act on and audit the learned reshaping.
#[must_use]
pub fn apply_to_resolution(
    state: &LearningState,
    mut resolved: ResolvedRoutes,
) -> AppliedResolution {
    let generation = state.infer_generation();
    // Learning inactive (or no promoted rule-set): behavior is the un-reshaped
    // resolution. Generation 0 is the "before any promotion" baseline.
    if !state.honors_induced_rules() {
        return AppliedResolution {
            resolved,
            infer_generation: generation,
        };
    }

    // A promoted rule-set is in force. Stamp the generation (so the change is
    // observable/auditable) AND apply the learned route preferences for real:
    // stably move any preferred route ahead of the rest, preserving relative
    // order within both the preferred and the non-preferred group. This is the
    // W4 promise that learned preferences SHAPE behavior — not merely an audit
    // stamp. The preference set is the gated signal carried on `state`, so an
    // empty set (the baseline) leaves the order untouched.
    reorder_by_preference(&mut resolved.data, &state.preferred_routes);
    AppliedResolution {
        resolved,
        infer_generation: generation,
    }
}

/// Stably reorder `routes` so any route the learning loop prefers leads, in the
/// preference's own priority order, with all other routes following in their
/// original relative order (W4.2).
///
/// "Preferred" is an exact route-id match against an entry in `preferred`. A
/// stable partition is used (not a comparator sort) so two equally-resolved
/// non-preferred routes keep their incoming order, and two preferred routes are
/// ordered by their rank in `preferred` (earlier = higher priority). An empty
/// `preferred` list is a no-op, so the baseline order is preserved exactly.
fn reorder_by_preference(routes: &mut [crate::resolve::ResolvedRoute], preferred: &[String]) {
    if preferred.is_empty() || routes.len() < 2 {
        return;
    }
    // The preference rank of a route: its index in `preferred`, or `len` (i.e.
    // after every preferred route) when it is not preferred. A stable sort by
    // this key moves preferred routes ahead in priority order while leaving the
    // relative order of equal-rank (e.g. all non-preferred) routes intact.
    let rank = |route: &str| {
        preferred
            .iter()
            .position(|p| p == route)
            .unwrap_or(preferred.len())
    };
    routes.sort_by_key(|item| rank(&item.route));
}

/// The resolution after the gated learning state has been applied (W4.1).
///
/// Carries the (possibly re-ordered) routes plus the inference generation that
/// shaped them, so a surface can render the routing AND attribute it to the
/// promoted rule-set generation — the legible audit the design demands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedResolution {
    /// The resolved routes after learned reshaping.
    pub resolved: ResolvedRoutes,
    /// The promoted inference generation that shaped this turn (`0` = baseline).
    pub infer_generation: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::principal::TrustContext;
    use crate::resolve::ResolvedRoute;
    use std::sync::Arc;
    use tdw_knowledge::runtime::AdaptivityResolver;

    /// A minimal vector-only runtime for the read seam, with a fixed
    /// `infer_version` and an adaptivity resolver mapping our agent.
    fn runtime_with(
        infer_version: Option<u64>,
        resolver: Option<AdaptivityResolver>,
    ) -> KnowledgeRuntime {
        let embedder = Arc::new(tdw_embed_local::HashEmbeddingProvider::default());
        let vectors = Arc::new(tdw_storage_qdrant::InMemoryVectorEngine::default());
        let mut runtime =
            KnowledgeRuntime::new(embedder, vectors).with_versions(None, infer_version);
        if let Some(resolver) = resolver {
            runtime = runtime.with_adaptivity_resolver(resolver);
        }
        runtime
    }

    fn principal() -> Principal {
        Principal::new("sess", "agent:partner")
    }

    fn learning_resolver() -> AdaptivityResolver {
        Arc::new(|agent: &str| {
            if agent == "agent:partner" {
                Some(Adaptivity::Learning)
            } else {
                None
            }
        })
    }

    fn routes() -> ResolvedRoutes {
        ResolvedRoutes {
            data: vec![ResolvedRoute {
                route: "equity/price/historical".to_string(),
                params: serde_json::json!({"symbol": "AAPL"}),
            }],
            knowledge: vec!["tdw.kg.answer".to_string()],
        }
    }

    /// Two equally-resolved data routes, X then Y in incoming order.
    fn two_routes(first: &str, second: &str) -> ResolvedRoutes {
        ResolvedRoutes {
            data: vec![
                ResolvedRoute {
                    route: first.to_string(),
                    params: serde_json::json!({}),
                },
                ResolvedRoute {
                    route: second.to_string(),
                    params: serde_json::json!({}),
                },
            ],
            knowledge: Vec::new(),
        }
    }

    // ── W4.1: a bumped version changes behavior ──────────────────────────────

    #[test]
    fn bumped_infer_version_changes_behavior() {
        // Same principal, same resolver, only the promoted infer_version differs.
        // The applied resolution's generation must track the bump — the concrete
        // "a bumped version changes behavior" the W4.1 unit demands.
        let before = LearningState::read_from(
            &runtime_with(Some(1), Some(learning_resolver())),
            &principal(),
        );
        let after = LearningState::read_from(
            &runtime_with(Some(2), Some(learning_resolver())),
            &principal(),
        );

        assert_ne!(
            before.infer_generation(),
            after.infer_generation(),
            "the promoted generation must differ across a version bump"
        );

        let applied_before = apply_to_resolution(&before, routes());
        let applied_after = apply_to_resolution(&after, routes());
        assert_eq!(applied_before.infer_generation, 1);
        assert_eq!(applied_after.infer_generation, 2);
        assert_ne!(
            applied_before, applied_after,
            "the same turn resolves differently after the version bump"
        );
    }

    // ── W4.2: a learned preference re-orders resolved routes (Gemini #440) ────

    #[test]
    fn learned_preference_moves_route_ahead_of_equal_peer() {
        // Two equally-resolved data routes, X then Y. A learned preference for Y
        // (the second) must move Y AHEAD of X — the real re-ordering the W4
        // promise demands, not a no-op stamp.
        let state = LearningState::read_from(
            &runtime_with(Some(1), Some(learning_resolver())),
            &principal(),
        )
        .with_preferred_routes(vec!["equity/profile".to_string()]);
        assert!(
            state.honors_induced_rules(),
            "the gate is open for the reorder"
        );

        let resolved = two_routes("equity/price/historical", "equity/profile");
        // Baseline incoming order is X (historical) then Y (profile).
        assert_eq!(resolved.data[0].route, "equity/price/historical");

        let applied = apply_to_resolution(&state, resolved);
        assert_eq!(
            applied.resolved.data[0].route,
            "equity/profile",
            "the learned-preferred route leads after reshaping: {:?}",
            applied.resolved.data_routes()
        );
        assert_eq!(
            applied.resolved.data[1].route, "equity/price/historical",
            "the non-preferred route follows"
        );
    }

    #[test]
    fn preference_reorder_is_stable_for_non_preferred_routes() {
        // Three routes A, B, C; only C is preferred. C leads; A and B keep their
        // incoming relative order (a stable partition, not a full sort).
        let state = LearningState::read_from(
            &runtime_with(Some(1), Some(learning_resolver())),
            &principal(),
        )
        .with_preferred_routes(vec!["equity/quote".to_string()]);
        let resolved = ResolvedRoutes {
            data: vec![
                ResolvedRoute {
                    route: "equity/price/historical".to_string(),
                    params: serde_json::json!({}),
                },
                ResolvedRoute {
                    route: "equity/profile".to_string(),
                    params: serde_json::json!({}),
                },
                ResolvedRoute {
                    route: "equity/quote".to_string(),
                    params: serde_json::json!({}),
                },
            ],
            knowledge: Vec::new(),
        };
        let applied = apply_to_resolution(&state, resolved);
        assert_eq!(
            applied.resolved.data_routes(),
            vec![
                "equity/quote".to_string(),
                "equity/price/historical".to_string(),
                "equity/profile".to_string(),
            ],
            "preferred route leads; non-preferred keep their order"
        );
    }

    #[test]
    fn no_preferences_leaves_route_order_untouched() {
        // With learning active but NO learned preferences, the baseline order is
        // preserved exactly — the reshaping is bounded by the gated signal.
        let state = LearningState::read_from(
            &runtime_with(Some(1), Some(learning_resolver())),
            &principal(),
        );
        assert!(state.preferred_routes.is_empty());
        let resolved = two_routes("equity/price/historical", "equity/profile");
        let applied = apply_to_resolution(&state, resolved);
        assert_eq!(
            applied.resolved.data[0].route, "equity/price/historical",
            "no preferences → baseline order is preserved"
        );
    }

    #[test]
    fn below_learning_principal_ignores_preferences() {
        // The gate dominates: a Configured (< Learning) principal does not honor
        // induced rules, so even a set preference does NOT reshape routing.
        let configured: AdaptivityResolver = Arc::new(|_| Some(Adaptivity::Configured));
        let state =
            LearningState::read_from(&runtime_with(Some(3), Some(configured)), &principal())
                .with_preferred_routes(vec!["equity/profile".to_string()]);
        assert!(!state.honors_induced_rules());
        let incoming = two_routes("equity/price/historical", "equity/profile");
        let applied = apply_to_resolution(&state, incoming);
        assert_eq!(
            applied.resolved.data[0].route, "equity/price/historical",
            "a below-Learning principal's routing is never reshaped by preferences"
        );
    }

    #[test]
    fn read_from_snapshots_versions_and_resolves_adaptivity() {
        let state = LearningState::read_from(
            &runtime_with(Some(7), Some(learning_resolver())),
            &principal(),
        );
        assert_eq!(state.infer_version, Some(7));
        assert_eq!(state.adaptivity, Some(Adaptivity::Learning));
        assert!(state.learning_active());
        assert!(state.honors_induced_rules());
    }

    // ── W4.2 + gate: low trust does not reshape behavior ─────────────────────

    #[test]
    fn below_learning_principal_does_not_honor_induced_rules() {
        // A resolver that returns Configured (< Learning): even with a promoted
        // infer_version, learning is inactive, so behavior is the baseline.
        let resolver: AdaptivityResolver = Arc::new(|_| Some(Adaptivity::Configured));
        let state = LearningState::read_from(&runtime_with(Some(3), Some(resolver)), &principal());
        assert!(!state.learning_active());
        assert!(!state.honors_induced_rules());
        assert_eq!(
            state.infer_generation(),
            0,
            "a below-Learning principal sees the un-promoted baseline"
        );
        let applied = apply_to_resolution(&state, routes());
        assert_eq!(applied.infer_generation, 0);
    }

    #[test]
    fn absent_resolver_means_learning_off() {
        let state = LearningState::read_from(&runtime_with(Some(5), None), &principal());
        assert_eq!(state.adaptivity, None);
        assert!(!state.learning_active());
        assert_eq!(state.infer_generation(), 0);
    }

    #[test]
    fn retrieval_floor_filters_low_trust_knowledge() {
        // Default floor (0.0) admits everything; a raised floor drops items below it.
        let open = principal();
        assert!(retrieval_admits(&open, 0.0));
        assert!(retrieval_admits(&open, 0.9));

        let strict = principal().with_trust(TrustContext {
            adaptivity: Adaptivity::Learning,
            retrieval_floor: 0.5,
        });
        assert!(!retrieval_admits(&strict, 0.4), "below floor is dropped");
        assert!(
            retrieval_admits(&strict, 0.5),
            "exactly at floor is admitted"
        );
        assert!(retrieval_admits(&strict, 0.6), "above floor is admitted");
    }
}
