#![allow(clippy::doc_markdown)] // narrative docstrings describe the MCP surface
//! `tdw.partner.audit` + `tdw.partner.undo` — the audit & undo surface
//! (partner-system W5.5, partner §4).
//!
//! Two **thin adapters** over the pure `tdw-partner` audit projection:
//!
//! - `tdw.partner.audit` renders the "what I did + why" feed. It takes the
//!   already-gathered source records (the gated proposal history — incl. the
//!   `tdw.kg.proposals` / `tdw.kg.dismiss` slice folded in — the self-tune log,
//!   and the lesson audits), projects them through [`tdw_partner::audit_feed`],
//!   applies the auto-accept-within-gates default + the escalation predicate, and
//!   returns the ranked records. No new store, no autonomy logic here — that is
//!   all in the `tdw-partner` shared core.
//! - `tdw.partner.undo` describes the one-gesture reversal for an audited action.
//!   The pure descriptor (which cold-plane move reverses it) is computed here; the
//!   actual graph write reuses `tdw_partner::undo` in the daemon (which holds the
//!   graph engine), exactly as the design's reversibility rests on the existing
//!   `recall_cold_edge` / inverse-`Forget` machinery.
//!
//! Both verbs are gated on a Partner Core being attached, like `tdw.partner.ask`
//! and `tdw.partner.brief` — the surface is off-by-default.

use serde_json::{Map, Value, json};
use tdw_partner::{AuditInputs, EscalationConfig, audit_feed};

use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool};

/// The audit-feed tool name.
pub const AUDIT_TOOL_NAME: &str = "tdw.partner.audit";

/// The undo tool name.
pub const UNDO_TOOL_NAME: &str = "tdw.partner.undo";

/// Whether `name` is one of the partner audit-surface tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == AUDIT_TOOL_NAME || name == UNDO_TOOL_NAME
}

/// Descriptor for `tdw.partner.audit`. Appended to `tools/list` only when a
/// Partner Core is attached (same gate as `tdw.partner.ask` / `.brief`).
#[must_use]
pub fn audit_descriptor() -> ToolDescriptor {
    tool(
        AUDIT_TOOL_NAME,
        "FinX Partner Audit",
        "The audit-only autonomy feed: render ONE 'what I did + why' stream over the records that \
         already exist — the gated proposal history (incl. the tdw.kg.proposals / tdw.kg.dismiss \
         slice), the self-tune log, and the lesson audits — NOT a new store. Pass the gathered \
         records as `inputs` { proposals: [...], tunes: [...], lessons: [...] }. Every action \
         renders a `why` provenance chain. The auto-accept-within-gates default applies; only \
         low-confidence / high-impact / irreversible actions escalate to AwaitingHuman, which lead \
         the feed (one surface for 'what I did' and 'what I'm waiting on you for'). Pass an \
         optional `config` { confidence_floor, high_impact_entities: [...] } to tune the \
         escalation predicate. Returns the ranked action records plus a count.",
        json!({
            "type": "object",
            "properties": {
                "inputs": {
                    "type": "object",
                    "description": "The already-gathered source records (AuditInputs): { proposals: [...], tunes: [...], lessons: [...] }.",
                    "properties": {
                        "proposals": { "type": "array", "items": { "type": "object" } },
                        "tunes": { "type": "array", "items": { "type": "object" } },
                        "lessons": { "type": "array", "items": { "type": "object" } }
                    },
                    "additionalProperties": false
                },
                "config": {
                    "type": "object",
                    "description": "The escalation predicate config: { confidence_floor, high_impact_entities: [...] }.",
                    "properties": {
                        "confidence_floor": { "type": "number" },
                        "high_impact_entities": { "type": "array", "items": { "type": "string" } }
                    },
                    "additionalProperties": false
                },
                "session_id": {
                    "type": "string",
                    "description": "Optional session identity the feed is scoped to (default: \"mcp-session\")."
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

/// Descriptor for `tdw.partner.undo`.
#[must_use]
pub fn undo_descriptor() -> ToolDescriptor {
    tool(
        UNDO_TOOL_NAME,
        "FinX Partner Undo",
        "One-gesture undo for an audited action, reusing the already-reversible cold-plane \
         machinery: a Forget is reversed by recalling the cold edge; an added edge is reversed by \
         an inverse Forget (retire to cold); a param-tune / promote is reversed by a re-tune / \
         retire through the gate. Pass the audited `action` (one record from tdw.partner.audit). \
         Returns the reversal plan: which cold-plane move undoes it and the (from, rel, to) edge \
         it touches, plus the action's `status` and a `needs_confirmation` flag. When the action \
         is AwaitingHuman (high-impact / low-confidence / irreversible), `needs_confirmation` is \
         true and the daemon must NOT apply the reversal without explicit human confirmation — the \
         human-in-the-loop posture survives into the reversal surface. The graph write itself is \
         performed by the daemon, which holds the engine; the historical record is never destroyed \
         (recall/retire preserve the cold audit trail).",
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "object",
                    "description": "The audited ActionRecord to reverse (one row from tdw.partner.audit)."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    )
}

/// Execute `tdw.partner.audit`: project the gathered records into the ranked
/// feed. Every failure is a tool error, never a protocol error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] when `inputs` / `config` are present but
/// malformed (cannot deserialize into the partner types).
pub fn execute_audit(arguments: &Map<String, Value>) -> Result<ToolExecution, ToolFailure> {
    let session_id = arguments
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("mcp-session");
    let agent_id = arguments
        .get("agent_id")
        .and_then(Value::as_str)
        .unwrap_or("agent:partner");
    let principal = tdw_partner::Principal::new(session_id, agent_id);

    let inputs: AuditInputs = match arguments.get("inputs") {
        None | Some(Value::Null) => AuditInputs::default(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| ToolFailure::Execution(format!("malformed inputs: {error}")))?,
    };
    let config: EscalationConfig = match arguments.get("config") {
        None | Some(Value::Null) => EscalationConfig::default(),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| ToolFailure::Execution(format!("malformed config: {error}")))?,
    };

    let feed = audit_feed(&inputs, &principal, &config);
    let actions = serde_json::to_value(&feed)
        .map_err(|error| ToolFailure::Execution(format!("serialize actions: {error}")))?;
    let awaiting = feed
        .iter()
        .filter(|record| record.status == tdw_partner::ActionStatus::AwaitingHuman)
        .count();

    Ok(structured(json!({
        "count": feed.len(),
        "awaiting_human": awaiting,
        "actions": actions,
    })))
}

/// Execute `tdw.partner.undo`: describe the reversal for an audited action.
///
/// This returns the *plan* (which cold-plane move reverses the action + the edge
/// it touches). The daemon performs the graph write via `tdw_partner::undo`; the
/// MCP surface, with no engine handle, returns the reversal descriptor so a caller
/// (or the daemon adapter) can act on it deterministically.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] when `action` is missing or malformed.
pub fn execute_undo(arguments: &Map<String, Value>) -> Result<ToolExecution, ToolFailure> {
    let action_value = arguments
        .get("action")
        .ok_or_else(|| ToolFailure::Execution("missing required field `action`".to_string()))?;
    let action: tdw_partner::ActionRecord = serde_json::from_value(action_value.clone())
        .map_err(|error| ToolFailure::Execution(format!("malformed action: {error}")))?;

    // The reversal plan is derived purely from the action kind — the same mapping
    // `tdw_partner::undo` performs, surfaced here without an engine so a caller
    // (or the daemon adapter) can act on it deterministically. Every plan is a
    // REAL reversal; a genuinely-irreversible write is flagged `unsupported`, not
    // a vacuous success (mirrors `tdw_partner::undo`'s UndoError::Unsupported).
    let (method, edge, param, restore_value) = match &action.what {
        tdw_partner::ActionKind::Forget { from, rel, to, .. } => (
            "recall_cold_edge",
            Some(json!({ "from": from, "rel": rel, "to": to })),
            None,
            None,
        ),
        tdw_partner::ActionKind::Edge { from, rel, to } => (
            "retire_edge_to_cold",
            Some(json!({ "from": from, "rel": rel, "to": to })),
            None,
            None,
        ),
        // An annotation write retires its `annotated_by` edge to cold; a tag
        // assign/define has no reversal primitive and is explicitly unsupported.
        tdw_partner::ActionKind::KgWrite { reversal, .. } => match reversal {
            tdw_partner::KgWriteReversal::Annotation {
                entity_id,
                annotation_node,
            } => (
                "retire_edge_to_cold",
                Some(json!({ "from": entity_id, "rel": "annotated_by", "to": annotation_node })),
                None,
                None,
            ),
            tdw_partner::KgWriteReversal::Unsupported { .. } => ("unsupported", None, None, None),
        },
        // A param tune is reversed by restoring its PRIOR value (carried on the
        // action) — a real re-tune, not a no-op.
        tdw_partner::ActionKind::ParamTune {
            param,
            restore_value,
            ..
        } => (
            "restore_param",
            None,
            Some(param.clone()),
            Some(*restore_value),
        ),
        // A promoted lesson retires its `annotated_by` edge to cold (demote).
        tdw_partner::ActionKind::RuledPromote {
            subject_entity,
            annotation_node,
            ..
        } => (
            "retire_edge_to_cold",
            Some(json!({ "from": subject_entity, "rel": "annotated_by", "to": annotation_node })),
            None,
            None,
        ),
    };

    // The HITL posture must survive into the reversal surface (MEDIUM, v1.7.1):
    // an action the escalation predicate flagged `AwaitingHuman` (high-impact /
    // low-confidence / irreversible) must NOT yield a ready-to-execute reversal.
    // We surface the action's status and, when it is AwaitingHuman, mark the plan
    // `needs_confirmation` so the daemon refuses to apply it without explicit
    // human confirmation — mirroring the escalation predicate, not re-deciding it.
    let needs_confirmation = action.status == tdw_partner::ActionStatus::AwaitingHuman;

    Ok(structured(json!({
        "action_id": action.id,
        "reversible": action.reversible,
        "status": action.status,
        "needs_confirmation": needs_confirmation,
        "method": method,
        "edge": edge,
        "param": param,
        "restore_value": restore_value,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    /// Run an execution, panicking with the message on failure (ToolFailure is
    /// not Debug, so we cannot `.expect()` directly — mirrors the brief tests).
    fn ok(result: Result<ToolExecution, ToolFailure>) -> ToolExecution {
        match result {
            Ok(execution) => execution,
            Err(ToolFailure::Execution(message)) => panic!("execution error: {message}"),
            Err(ToolFailure::Protocol(_)) => panic!("unexpected protocol error"),
        }
    }

    #[test]
    fn owns_the_audit_and_undo_names() {
        assert!(owns("tdw.partner.audit"));
        assert!(owns("tdw.partner.undo"));
        assert!(!owns("tdw.partner.brief"));
    }

    #[test]
    fn empty_inputs_yield_an_empty_feed() {
        let execution = ok(execute_audit(&Map::new()));
        assert_eq!(execution.structured["count"], json!(0));
        assert_eq!(execution.structured["actions"], json!([]));
    }

    #[test]
    fn audit_renders_a_why_for_every_action() {
        // A materialized Forget proposal projects to one action carrying a why.
        let arguments = args(&json!({
            "inputs": {
                "proposals": [
                    {
                        "id": "p1",
                        "kind": { "kind": "forget", "from": "company:ACME", "rel": "hq_in", "to": "city:old", "reason": "stale fact" },
                        "agent_id": "agent:partner",
                        "status": "validated",
                        "materialized": true,
                        "history": ["2026-06-14 draft by agent:partner", "2026-06-14 validated (automated validators)"]
                    }
                ]
            }
        }));
        let execution = ok(execute_audit(&arguments));
        assert_eq!(execution.structured["count"], json!(1));
        let actions = execution.structured["actions"].as_array().expect("array");
        let why = actions[0]["why"]["routes"].as_array().expect("why routes");
        assert!(!why.is_empty(), "every action renders a why: {actions:?}");
        // A materialized routine act auto-accepts.
        assert_eq!(actions[0]["status"], json!("auto_accepted"));
    }

    #[test]
    fn high_impact_config_escalates_to_awaiting_human() {
        let arguments = args(&json!({
            "inputs": {
                "proposals": [
                    {
                        "id": "p1",
                        "kind": { "kind": "forget", "from": "thesis:core", "rel": "supports", "to": "x", "reason": "r" },
                        "agent_id": "agent:partner",
                        "status": "validated",
                        "materialized": true,
                        "history": ["2026-06-14 draft by agent:partner"]
                    }
                ]
            },
            "config": { "confidence_floor": 0.5, "high_impact_entities": ["thesis:core"] }
        }));
        let execution = ok(execute_audit(&arguments));
        assert_eq!(execution.structured["awaiting_human"], json!(1));
        let actions = execution.structured["actions"].as_array().expect("array");
        assert_eq!(actions[0]["status"], json!("awaiting_human"));
    }

    #[test]
    fn undo_describes_the_cold_plane_reversal_for_a_forget() {
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p1",
                "agent_id": "agent:partner",
                "what": { "kind": "forget", "proposal_id": "p1", "from": "company:ACME", "rel": "hq_in", "to": "city:old" },
                "why": { "routes": ["why: stale"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "auto_accepted"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["method"], json!("recall_cold_edge"));
        assert_eq!(execution.structured["edge"]["from"], json!("company:ACME"));
        assert_eq!(execution.structured["reversible"], json!(true));
    }

    #[test]
    fn undo_describes_inverse_forget_for_an_edge_write() {
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p2",
                "agent_id": "agent:partner",
                "what": { "kind": "edge", "from": "a", "rel": "supplier_of", "to": "b" },
                "why": { "routes": ["2026-06-14 draft"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "auto_accepted"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["method"], json!("retire_edge_to_cold"));
        assert_eq!(execution.structured["edge"]["to"], json!("b"));
    }

    #[test]
    fn undo_describes_restore_param_for_a_param_tune() {
        // A param tune reverses by restoring its PRIOR value — surfaced in the
        // plan, not a no-op (the Gemini #441 HIGH fix reflected on the surface).
        let arguments = args(&json!({
            "action": {
                "id": "action:tune:t1",
                "agent_id": "system:self-tuner",
                "what": { "kind": "param_tune", "record_id": "t1", "param": "rrf_k", "applied_value": 62.0, "restore_value": 60.0 },
                "why": { "routes": ["agent:system:self-tuner;gated:true"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "auto_accepted"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["method"], json!("restore_param"));
        assert_eq!(execution.structured["param"], json!("rrf_k"));
        assert_eq!(execution.structured["restore_value"], json!(60.0));
    }

    #[test]
    fn undo_flags_a_tag_write_as_unsupported_not_a_silent_ok() {
        // A tag assign/define has no reversal primitive: the plan must say
        // `unsupported`, never pretend a reversal exists.
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p8",
                "agent_id": "agent:partner",
                "what": { "kind": "kg_write", "proposal_id": "p8", "reversal": { "reversal": "unsupported", "detail": "tag has no reversal primitive" } },
                "why": { "routes": ["2026-06-14 draft"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": false,
                "status": "awaiting_human"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["method"], json!("unsupported"));
        assert_eq!(execution.structured["reversible"], json!(false));
    }

    #[test]
    fn undo_describes_annotation_edge_retire_for_a_kgwrite() {
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p7",
                "agent_id": "agent:partner",
                "what": { "kind": "kg_write", "proposal_id": "p7", "reversal": { "reversal": "annotation", "entity_id": "company:ACME", "annotation_node": "annotation:p7" } },
                "why": { "routes": ["2026-06-14 draft"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "auto_accepted"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["method"], json!("retire_edge_to_cold"));
        assert_eq!(execution.structured["edge"]["to"], json!("annotation:p7"));
    }

    #[test]
    fn undo_of_an_awaiting_human_action_requires_confirmation() {
        // A high-impact Forget the escalation predicate flagged AwaitingHuman must
        // NOT yield a ready-to-execute reversal: the plan still describes the
        // cold-plane move (so the human can see what WOULD happen) but flags
        // `needs_confirmation` so the daemon refuses to apply it un-reviewed
        // (MEDIUM, v1.7.1 — the HITL posture survives into the reversal surface).
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p9",
                "agent_id": "agent:partner",
                "what": { "kind": "forget", "proposal_id": "p9", "from": "thesis:core", "rel": "supports", "to": "x" },
                "why": { "routes": ["why: high-impact"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "awaiting_human"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        // The reversal is still described (reversible action), but gated on a human.
        assert_eq!(execution.structured["method"], json!("recall_cold_edge"));
        assert_eq!(execution.structured["status"], json!("awaiting_human"));
        assert_eq!(
            execution.structured["needs_confirmation"],
            json!(true),
            "an AwaitingHuman action's reversal requires explicit confirmation"
        );
    }

    #[test]
    fn undo_of_an_auto_accepted_action_needs_no_confirmation() {
        // The default posture: an auto-accepted action's reversal is ready to
        // execute (no human gate), so needs_confirmation is false.
        let arguments = args(&json!({
            "action": {
                "id": "action:proposal:p1",
                "agent_id": "agent:partner",
                "what": { "kind": "forget", "proposal_id": "p1", "from": "company:ACME", "rel": "hq_in", "to": "city:old" },
                "why": { "routes": ["why: stale"], "kg_nodes": [], "as_of": null },
                "confidence": 0.95,
                "reversible": true,
                "status": "auto_accepted"
            }
        }));
        let execution = ok(execute_undo(&arguments));
        assert_eq!(execution.structured["status"], json!("auto_accepted"));
        assert_eq!(execution.structured["needs_confirmation"], json!(false));
    }

    #[test]
    fn undo_missing_action_is_a_tool_error() {
        assert!(matches!(
            execute_undo(&Map::new()),
            Err(ToolFailure::Execution(_))
        ));
    }
}
