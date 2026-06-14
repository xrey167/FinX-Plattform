//! The shared persona / memory / trust seam (`finx-partner` §6).
//!
//! [`Principal`] is the one identity per session that every surface (MCP,
//! Workspace, CLI) establishes once and threads through [`crate::PartnerCore`].
//! [`Provenance`] is the surface-agnostic citation a turn emits: the route ids
//! and knowledge-graph node ids that backed the answer, plus the `as_of` instant.
//!
//! These types are deliberately transport-free: an adapter maps its own auth
//! context to a [`Principal`] and renders a [`Provenance`] to its wire format,
//! but no routing, retrieval, or autonomy logic lives here.

use serde::{Deserialize, Serialize};
use tdw_taxonomy::Adaptivity;

/// The trust dial + [`Adaptivity`] for a [`Principal`].
///
/// One dial drives both retrieval filtering (low-trust knowledge is
/// down-weighted) and the autonomy threshold (the write-back gate reads the
/// `adaptivity` here). It is a per-principal value so the same shared core
/// behaves differently for a low-trust prospect versus a fully-onboarded agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustContext {
    /// The agent's learning authority — the input to the write-back gate
    /// (`finx-partner` §3). The [`crate::ProposalQueue`] refuses writes below
    /// [`Adaptivity::Learning`], so a `None`/`Configured` principal can read and
    /// answer but never mutate the graph.
    pub adaptivity: Adaptivity,
    /// The retrieval trust floor in `[0.0, 1.0]`: knowledge scored below this is
    /// filtered at context-assembly time. `0.0` admits everything (the default).
    pub retrieval_floor: f64,
}

impl Default for TrustContext {
    fn default() -> Self {
        Self {
            adaptivity: Adaptivity::Learning,
            retrieval_floor: 0.0,
        }
    }
}

/// The one identity per partner session (`finx-partner` §6).
///
/// Built once per session and consumed by every surface. The transport that
/// *establishes* it differs (MCP auth / Workspace token / CLI env) but the
/// identity, knowledge-graph scope, and trust dial are shared — that is the
/// anti-duplication invariant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Principal {
    /// The session identity (a `tdw-session` `SessionRecord` id).
    pub session_id: String,
    /// The end-user identity, when bound (drives the finding surface).
    #[serde(default)]
    pub user_id: Option<String>,
    /// The agent identity threaded into the write-back gate's admission.
    pub agent_id: String,
    /// The per-principal knowledge-graph scope (`with_graph_name`).
    pub kg_namespace: String,
    /// The trust dial + [`Adaptivity`] for this principal.
    pub trust: TrustContext,
}

impl Principal {
    /// A minimal principal for `session_id`/`agent_id` with a default
    /// (`Learning`, no retrieval floor) trust context and the default namespace.
    #[must_use]
    pub fn new(session_id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            user_id: None,
            agent_id: agent_id.into(),
            kg_namespace: "default".to_string(),
            trust: TrustContext::default(),
        }
    }

    /// Bind the end-user identity (enables the finding/episodic surface).
    #[must_use]
    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Set the per-principal knowledge-graph namespace.
    #[must_use]
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.kg_namespace = namespace.into();
        self
    }

    /// Set the trust context (trust dial + adaptivity).
    #[must_use]
    pub const fn with_trust(mut self, trust: TrustContext) -> Self {
        self.trust = trust;
        self
    }
}

/// The surface-agnostic citation a turn emits (`finx-partner` §1.2).
///
/// Carries the data route ids and knowledge-graph node ids that backed the
/// answer plus the point-in-time `as_of`. An adapter renders this to its wire
/// citation (`SseEvent::Citations` for Workspace, a JSON block for MCP, a line
/// for the CLI).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// The catalog routes (and `tdw.kg.*` verbs) the turn resolved and executed.
    #[serde(default)]
    pub routes: Vec<String>,
    /// The knowledge-graph node ids the context step grounded the answer in.
    #[serde(default)]
    pub kg_nodes: Vec<String>,
    /// The point-in-time the answer is valid as of, when known.
    #[serde(default)]
    pub as_of: Option<String>,
}

impl Provenance {
    /// Whether this provenance carries any attribution at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.routes.is_empty() && self.kg_nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_builder_threads_identity_and_trust() {
        let principal = Principal::new("sess-1", "agent:partner")
            .with_user_id("user-9")
            .with_namespace("desk-a");
        assert_eq!(principal.session_id, "sess-1");
        assert_eq!(principal.agent_id, "agent:partner");
        assert_eq!(principal.user_id.as_deref(), Some("user-9"));
        assert_eq!(principal.kg_namespace, "desk-a");
        // Default trust admits writes (Learning) and filters nothing.
        assert_eq!(principal.trust.adaptivity, Adaptivity::Learning);
        assert!((principal.trust.retrieval_floor - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn provenance_empty_until_attributed() {
        let mut provenance = Provenance::default();
        assert!(provenance.is_empty());
        provenance
            .routes
            .push("equity/price/historical".to_string());
        assert!(!provenance.is_empty());
    }

    #[test]
    fn principal_round_trips_through_serde() {
        let principal = Principal::new("s", "a").with_namespace("ns");
        let json = serde_json::to_string(&principal).expect("serialize");
        let back: Principal = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(principal, back);
    }
}
