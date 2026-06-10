//! `AgentBridgeHandler` implementation wiring the daemon's language model to the
//! `tdw-app-server` `OpenBB` copilot route family (feature = "agent-route").
//!
//! This is the *caller-side* of the agent-protocol transport seam:
//! `tdw-app-server` defines the [`AgentBridgeHandler`] trait but does not depend
//! on this crate; here we implement it on a thin [`AgentBridgeState`] that holds
//! a [`StreamingLanguageModel`] and delegates the whole copilot turn to the pure
//! [`tdw_openbb_agent::answer`] sequencer. No `OpenBB` JSON shape and no agent
//! loop logic lives here — only the model wiring.
//!
//! # Offline-first
//!
//! The default model is the deterministic offline
//! [`StubLanguageModel`](tdw_eval_runner::StubLanguageModel), so the bridge runs
//! end-to-end with no network or credentials. Inject a live streaming client via
//! [`AgentBridgeState::with_language_model`] (behind the daemon's existing
//! credential/env gates) to answer against a real model.

#![cfg(feature = "agent-route")]

use std::sync::Arc;

use tdw_app_server::AgentBridgeHandler;
use tdw_eval_runner::StubLanguageModel;
use tdw_llm::StreamingLanguageModel;
use tdw_openbb_agent::{Answer, QueryRequest, answer};

/// Adapter that implements [`AgentBridgeHandler`] over a configured streaming
/// language model.
///
/// Cheap to clone (`Arc`-shared model); construct one via
/// [`AgentBridgeState::new`] (the offline stub) or
/// [`AgentBridgeState::with_language_model`] (a live client), then wrap it in an
/// `Arc` for `tdw_app_server::serve_agent_http`.
#[derive(Clone)]
pub struct AgentBridgeState {
    model: Arc<dyn StreamingLanguageModel>,
}

impl Default for AgentBridgeState {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBridgeState {
    /// Build a bridge state driven by the offline deterministic
    /// [`StubLanguageModel`] (no network, no credentials).
    #[must_use]
    pub fn new() -> Self {
        Self {
            model: Arc::new(StubLanguageModel),
        }
    }

    /// Replace the streaming model (default: the offline [`StubLanguageModel`]).
    /// Inject a live client here to answer against a real model.
    #[must_use]
    pub fn with_language_model(model: Arc<dyn StreamingLanguageModel>) -> Self {
        Self { model }
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
        // The pure sequencer drives `complete_streaming` synchronously; the
        // offline stub is non-blocking. A live client is injected behind the
        // daemon's credential gates and is the operator's choice.
        answer(&request, self.model.as_ref())
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
            "the stub streams an answer"
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
            "the first leg emits get_widget_data"
        );
    }
}
