#![allow(clippy::doc_markdown)] // narrative docstrings describe the MCP surface
//! `tdw.partner.ask` — the single Partner Core front-door tool (partner-system
//! W2.6, FinX Partner §1.5).
//!
//! This is a **thin adapter**: it maps an MCP `tools/call` request to one
//! [`PartnerCore::turn`] and serializes the streamed [`PartnerEvent`]s into the
//! tool response (the streamed answer text + the closing citation). No routing,
//! retrieval, or autonomy logic lives here — that is all in `tdw-partner`, the
//! shared core. The 49 `tdw.*` tools stay registered for power users; this is
//! the one verb the onboarding surface leads with ("ask FinX anything").

use serde_json::{Map, Value, json};
use tdw_partner::{PartnerCore, PartnerEvent, PartnerTurn, Principal};

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool};

/// The tool name owned by this module.
pub const TOOL_NAME: &str = "tdw.partner.ask";

/// Whether `name` is the partner ask tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

/// Maximum characters in an utterance (mirrors the knowledge query cap).
const MAX_UTTERANCE_CHARS: usize = 8192;

/// Descriptor for `tdw.partner.ask`. Appended to `tools/list` only when a
/// [`PartnerCore`] is attached (gated in [`crate::McpServer`]).
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool(
        TOOL_NAME,
        "Ask FinX (Partner)",
        "The single Partner front-door: ask FinX anything in natural language. \
         Internally resolves the right data routes and knowledge verbs (bounded by \
         the endpoint catalog — never free-form), fetches and grounds the answer, and \
         returns the answer text plus a citation of the routes and knowledge-graph \
         nodes it drew from. This one verb dispatches to the right tool for you; the \
         lower-level tdw.* tools remain available for power users. \
         \n\n\
         Caps: utterance ≤ 8192 chars.",
        json!({
            "type": "object",
            "properties": {
                "utterance": {
                    "type": "string",
                    "description": "Your question, in natural language (non-empty, ≤ 8192 chars)."
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session identity for episodic continuity (default: \"mcp-session\")."
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional agent identity threaded into the write-back gate (default: \"agent:partner\")."
                }
            },
            "required": ["utterance"],
            "additionalProperties": false
        }),
    )
}

/// Execute `tdw.partner.ask`: run one Partner Core turn and serialize the
/// streamed events.
///
/// Every failure is a tool error, never a protocol error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for malformed input or a data-plane fetch
/// failure surfaced by the turn.
pub fn execute(
    partner: &PartnerCore,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let utterance = arguments
        .get("utterance")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ToolFailure::Execution("missing required argument: utterance".to_string())
        })?;
    if utterance.trim().is_empty() {
        return Err(ToolFailure::Execution(
            "utterance must be non-empty".to_string(),
        ));
    }
    if utterance.chars().count() > MAX_UTTERANCE_CHARS {
        return Err(ToolFailure::Execution(format!(
            "utterance must be at most {MAX_UTTERANCE_CHARS} characters"
        )));
    }
    let session_id = arguments
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("mcp-session");
    let agent_id = arguments
        .get("agent_id")
        .and_then(Value::as_str)
        .unwrap_or("agent:partner");

    let principal = Principal::new(session_id, agent_id);
    let turn = PartnerTurn::new(principal, utterance);

    // Collect the streamed events so the MCP response carries the ordered
    // reasoning steps, the answer text, and the closing citation — the same
    // event vocabulary every surface renders.
    let mut reasoning: Vec<String> = Vec::new();
    let mut answer = String::new();
    let mut citation: Option<Value> = None;
    let outcome = block_on(partner.turn(&turn, &mut |event| match event {
        PartnerEvent::Reasoning(message) => reasoning.push(message),
        PartnerEvent::Answer(fragment) => answer.push_str(&fragment),
        PartnerEvent::Citation(provenance) => {
            citation = Some(json!({
                "routes": provenance.routes,
                "kg_nodes": provenance.kg_nodes,
                "as_of": provenance.as_of,
            }));
        }
    }))
    .map_err(|error| ToolFailure::Execution(error.to_string()))?;

    Ok(structured(json!({
        "utterance": utterance,
        "answer": answer,
        "reasoning": reasoning,
        "citation": citation,
        "resolved_routes": {
            "data": outcome.resolved.data,
            "knowledge": outcome.resolved.knowledge,
        },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tdw_eval_runner::StubLanguageModel;
    use tdw_partner::{DataPlane, DataPlaneError};

    struct NoopPlane;

    #[async_trait::async_trait]
    impl DataPlane for NoopPlane {
        async fn fetch(&self, route: &str, _params: Value) -> Result<Value, DataPlaneError> {
            Err(DataPlaneError::Fetch {
                route: route.to_string(),
                message: "no data plane in test".to_string(),
            })
        }
    }

    fn core() -> PartnerCore {
        PartnerCore::new(Arc::new(StubLanguageModel), Arc::new(NoopPlane))
    }

    #[test]
    fn owns_only_the_partner_ask_name() {
        assert!(owns("tdw.partner.ask"));
        assert!(!owns("tdw.kg.answer"));
    }

    #[test]
    fn ask_streams_an_answer_offline() {
        let core = core();
        let mut args = Map::new();
        args.insert("utterance".to_string(), json!("What is a P/E ratio?"));
        let Ok(execution) = execute(&core, &args) else {
            panic!("ask executes offline");
        };
        // The structured payload carries the streamed answer + citation block.
        let value = &execution.structured;
        let answer = value["answer"].as_str().unwrap_or_default();
        assert!(
            answer.contains("P/E ratio"),
            "streamed answer echoes the question: {answer:?}"
        );
        // The reasoning trail opens with the partner's review step.
        assert!(
            value["reasoning"]
                .as_array()
                .is_some_and(|steps| !steps.is_empty()),
            "carries the reasoning trail: {value}"
        );
    }

    #[test]
    fn ask_rejects_empty_utterance() {
        let core = core();
        let mut args = Map::new();
        args.insert("utterance".to_string(), json!("   "));
        assert!(
            matches!(execute(&core, &args), Err(ToolFailure::Execution(_))),
            "empty utterance is rejected as a tool error"
        );
    }
}
