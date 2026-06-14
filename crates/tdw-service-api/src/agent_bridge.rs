//! `AgentBridgeHandler` implementation routing the daemon's `OpenBB` Workspace
//! copilot turn through **Partner Core** (feature = "agent-route").
//!
//! This is the *caller-side* of the agent-protocol transport seam:
//! `tdw-app-server` defines the [`AgentBridgeHandler`] trait but does not depend
//! on this crate; here we implement it on a thin [`AgentBridgeState`] that holds
//! a [`PartnerCore`] (the ONE shared front door, `finx-partner` §1) and delegates
//! the whole copilot turn to [`PartnerCore::answer_workspace`].
//!
//! This is the partner-system **W2.7** swap — the highest-leverage move in W2.
//! Previously this bridge called the bare `tdw_openbb_agent::answer()` sequencer
//! directly; routing it through `PartnerCore` instead lights up the Workspace
//! surface as a *thin adapter over the shared core*, validating the
//! "shared core, thin adapter" thesis. The two-request widget-data leg is
//! preserved exactly (Partner Core reuses the same pure `tdw_openbb_agent`
//! sequencer for the Workspace path), so the existing `agent_route_e2e.rs`
//! golden SSE transcript is unchanged. No `OpenBB` JSON shape and no routing /
//! retrieval / autonomy logic lives here — only the core wiring.
//!
//! # Offline-first
//!
//! The default Partner Core drives the deterministic offline
//! [`StubLanguageModel`](tdw_eval_runner::StubLanguageModel) over a no-op data
//! plane, so the bridge runs end-to-end with no network or credentials. Inject a
//! live streaming client via [`AgentBridgeState::with_language_model`] (behind
//! the daemon's existing credential/env gates) to answer against a real model.

#![cfg(feature = "agent-route")]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tdw_app_server::AgentBridgeHandler;
use tdw_eval_runner::StubLanguageModel;
use tdw_llm::StreamingLanguageModel;
use tdw_openbb_agent::{Answer, QueryRequest};
use tdw_partner::{DataPlane, DataPlaneError, PartnerCore};

/// A no-op data plane for the offline Workspace bridge default.
///
/// The Workspace two-request widget-data leg (the only data path the copilot
/// surface uses) fetches through the *frontend* via `get_widget_data`, not
/// through the dispatcher, so the bridge's Partner Core needs no server-side
/// fetch to preserve the existing contract. A daemon that wants the partner to
/// fetch catalog routes server-side injects a real [`DataPlane`] over
/// `dispatcher::rest_fetch_data` here.
struct NoopDataPlane;

#[async_trait]
impl DataPlane for NoopDataPlane {
    async fn fetch(&self, route: &str, _params: Value) -> Result<Value, DataPlaneError> {
        Err(DataPlaneError::Fetch {
            route: route.to_string(),
            message: "server-side data plane not wired for the Workspace bridge \
                      (widget data flows through the frontend get_widget_data leg)"
                .to_string(),
        })
    }
}

/// Adapter that implements [`AgentBridgeHandler`] by routing the turn through a
/// [`PartnerCore`] (the shared front door).
///
/// Cheap to clone (`Arc`-shared core); construct one via
/// [`AgentBridgeState::new`] (the offline stub) or
/// [`AgentBridgeState::with_language_model`] (a live client), then wrap it in an
/// `Arc` for `tdw_app_server::serve_agent_http`.
#[derive(Clone)]
pub struct AgentBridgeState {
    partner: PartnerCore,
}

impl Default for AgentBridgeState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBridgeState {
    /// Build a bridge state whose Partner Core drives the offline deterministic
    /// [`StubLanguageModel`] (no network, no credentials).
    #[must_use]
    pub fn new() -> Self {
        Self::with_language_model(Arc::new(StubLanguageModel))
    }

    /// Build a bridge state whose Partner Core drives `model` (inject a live
    /// streaming client here to answer against a real model). The data plane
    /// defaults to the no-op (widget data flows through the frontend leg).
    #[must_use]
    pub fn with_language_model(model: Arc<dyn StreamingLanguageModel>) -> Self {
        Self {
            partner: PartnerCore::new(model, Arc::new(NoopDataPlane)),
        }
    }

    /// Build a bridge state from an already-composed [`PartnerCore`] — the seam
    /// a daemon uses to share one core (with a real data plane + trust context)
    /// across every surface.
    #[must_use]
    pub const fn from_partner(partner: PartnerCore) -> Self {
        Self { partner }
    }

    /// Build an `Arc<dyn AgentBridgeHandler>` ready to hand to
    /// `tdw_app_server::serve_agent_http`.
    #[must_use]
    pub fn into_handler(self) -> Arc<dyn AgentBridgeHandler> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl AgentBridgeHandler for AgentBridgeState {
    async fn answer(&self, request: QueryRequest) -> Answer {
        // The shared core's Workspace seam reuses the pure two-leg sequencer, so
        // the wire contract is identical to the pre-W2.7 bare `answer()` call —
        // the turn now simply flows through Partner Core.
        self.partner.answer_workspace(&request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tdw_openbb_agent::SseEvent;

    #[tokio::test]
    async fn default_state_answers_a_simple_question_with_message_chunks() {
        let state = AgentBridgeState::new();
        let request: QueryRequest = serde_json::from_value(json!({
            "messages": [{"role": "human", "content": "What is a P/E ratio?"}]
        }))
        .expect("parses");
        let answer = state.answer(request).await;
        assert!(!answer.closed_for_widget_data);
        assert!(
            answer
                .events
                .iter()
                .any(|event| matches!(event, SseEvent::MessageChunk { .. })),
            "the shared core streams an answer"
        );
    }

    #[tokio::test]
    async fn primary_widget_without_tool_result_requests_widget_data() {
        let state = AgentBridgeState::new();
        let request: QueryRequest = serde_json::from_value(json!({
            "messages": [{"role": "human", "content": "How did AAPL do?"}],
            "widgets": {"primary": [{"uuid": "w-1", "params": {"symbol": "AAPL"}}]}
        }))
        .expect("parses");
        let answer = state.answer(request).await;
        assert!(answer.closed_for_widget_data);
        assert!(
            answer
                .events
                .iter()
                .any(|event| matches!(event, SseEvent::GetWidgetData { .. })),
            "the first leg emits get_widget_data through the shared core"
        );
    }
}
