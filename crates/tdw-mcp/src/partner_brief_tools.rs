#![allow(clippy::doc_markdown)] // narrative docstrings describe the MCP surface
//! `tdw.partner.brief` — the proactive morning-brief tool (partner-system W3.6,
//! partner §2).
//!
//! A **thin adapter** over the pure [`tdw_partner::build_brief`] assembler: it
//! takes the unified signal sources (fired alerts + the knowledge signals the
//! caller already gathered from `tdw.kg.{digest,thesis_health,questions,diff}`),
//! ranks them into one [`tdw_partner::Nudge`] stream, applies any dismissal
//! re-ranking (the W3.5 feedback effect), and returns the ranked brief. No
//! ranking, scheduling, or autonomy logic lives here — that is all in the
//! `tdw-partner` shared core. This is the second of the two verbs the onboarding
//! surface leads with (alongside `tdw.partner.ask`); the brief's daily schedule
//! is a `tdw-cron` trigger wired in the daemon (partner-system W3.3).

use serde_json::{Map, Value, json};
use tdw_partner::{BriefInputs, Dismissal, build_brief, rerank_with_dismissals};

use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool};

/// The tool name owned by this module.
pub const TOOL_NAME: &str = "tdw.partner.brief";

/// Whether `name` is the partner brief tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

/// Descriptor for `tdw.partner.brief`. Appended to `tools/list` only when a
/// Partner Core is attached (same gate as `tdw.partner.ask`).
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool(
        TOOL_NAME,
        "FinX Partner Brief",
        "The proactive morning brief: fan the unified signal sources — fired price alerts and \
         the knowledge signals (thesis health, open questions, contradictions, staleness) — into \
         ONE stream of nudges, ranked by severity then recency. Pass the gathered signals as \
         `inputs` (the shape the alert evaluator and the tdw.kg.{digest,thesis_health,questions,\
         diff} verbs produce); pass `dismissals` to down-rank nudge kinds the user keeps \
         dismissing (the learning feedback effect). Returns the ranked nudges plus a count. The \
         daily schedule is driven by a tdw-cron trigger in the daemon; this verb assembles the \
         brief on demand.",
        json!({
            "type": "object",
            "properties": {
                "inputs": {
                    "type": "object",
                    "description": "The unified signal sources (BriefInputs): { alerts: [...], signals: [...] }.",
                    "properties": {
                        "alerts": { "type": "array", "items": { "type": "object" } },
                        "signals": { "type": "array", "items": { "type": "object" } }
                    },
                    "additionalProperties": false
                },
                "dismissals": {
                    "type": "array",
                    "description": "Per-kind dismissal counts that down-rank noisy nudge kinds: [{kind, count}].",
                    "items": { "type": "object" }
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session identity the brief is scoped to (default: \"mcp-session\")."
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional agent identity (default: \"agent:partner\")."
                }
            },
            "additionalProperties": false
        }),
    )
}

/// Execute `tdw.partner.brief`: assemble and rank the brief from the gathered
/// inputs. Every failure is a tool error, never a protocol error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] when `inputs` / `dismissals` are present
/// but malformed (cannot deserialize into the partner types).
pub fn execute(arguments: &Map<String, Value>) -> Result<ToolExecution, ToolFailure> {
    let session_id = arguments
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("mcp-session");
    let agent_id = arguments
        .get("agent_id")
        .and_then(Value::as_str)
        .unwrap_or("agent:partner");
    let principal = tdw_partner::Principal::new(session_id, agent_id);

    // Inputs default to empty (an honest empty brief) when not supplied.
    let inputs: BriefInputs = match arguments.get("inputs") {
        None | Some(Value::Null) => BriefInputs::default(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| ToolFailure::Execution(format!("malformed inputs: {error}")))?,
    };
    let dismissals: Vec<Dismissal> = match arguments.get("dismissals") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| ToolFailure::Execution(format!("malformed dismissals: {error}")))?,
    };

    let brief = build_brief(&inputs, &principal);
    let ranked = rerank_with_dismissals(brief, &dismissals);

    let nudges = serde_json::to_value(&ranked)
        .map_err(|error| ToolFailure::Execution(format!("serialize nudges: {error}")))?;

    Ok(structured(json!({
        "count": ranked.len(),
        "nudges": nudges,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn owns_only_the_brief_name() {
        assert!(owns("tdw.partner.brief"));
        assert!(!owns("tdw.partner.ask"));
    }

    fn run(arguments: &Map<String, Value>) -> ToolExecution {
        match execute(arguments) {
            Ok(execution) => execution,
            Err(ToolFailure::Execution(message)) => panic!("brief execution error: {message}"),
            Err(ToolFailure::Protocol(_)) => panic!("brief returned a protocol error"),
        }
    }

    #[test]
    fn empty_inputs_yield_an_empty_brief() {
        let execution = run(&Map::new());
        assert_eq!(execution.structured["count"], json!(0));
        assert_eq!(execution.structured["nudges"], json!([]));
    }

    #[test]
    fn ranks_a_golden_brief_from_fixed_inputs() {
        // W3.2 golden: a fired alert + two knowledge signals rank
        // Critical-alert → High-thesis → Medium-question.
        let arguments = args(&json!({
            "inputs": {
                "alerts": [
                    { "id": "a-1", "symbol": "AAPL", "headline": "AAPL crossed $200", "fired_at": "2026-06-14" }
                ],
                "signals": [
                    { "id": "q-3", "kind": "OpenQuestion", "severity": "Medium",
                      "headline": "open question", "kg_nodes": ["question:q-3"], "as_of": "2026-06-14" },
                    { "id": "t-7", "kind": "ThesisHealth", "severity": "High",
                      "headline": "thesis weakening", "kg_nodes": ["finding:t-7"], "as_of": "2026-06-13" }
                ]
            }
        }));
        let execution = run(&arguments);
        assert_eq!(execution.structured["count"], json!(3));
        let nudges = execution.structured["nudges"].as_array().expect("array");
        assert_eq!(nudges[0]["headline"], json!("AAPL crossed $200"));
        assert_eq!(nudges[1]["headline"], json!("thesis weakening"));
        assert_eq!(nudges[2]["headline"], json!("open question"));
    }

    #[test]
    fn dismissals_rerank_the_brief() {
        let arguments = args(&json!({
            "inputs": {
                "signals": [
                    { "id": "noisy", "kind": "Staleness", "severity": "Medium",
                      "headline": "noisy", "kg_nodes": [], "as_of": "2026-06-14" },
                    { "id": "useful", "kind": "OpenQuestion", "severity": "Medium",
                      "headline": "useful", "kg_nodes": [], "as_of": "2026-06-13" }
                ]
            },
            "dismissals": [ { "kind": "Staleness", "count": 4 } ]
        }));
        let execution = run(&arguments);
        let nudges = execution.structured["nudges"].as_array().expect("array");
        assert_eq!(
            nudges[0]["headline"],
            json!("useful"),
            "a repeatedly-dismissed kind sinks below an un-dismissed peer"
        );
    }

    #[test]
    fn malformed_inputs_is_a_tool_error() {
        let arguments = args(&json!({ "inputs": { "alerts": "not-an-array" } }));
        assert!(matches!(
            execute(&arguments),
            Err(ToolFailure::Execution(_))
        ));
    }
}
