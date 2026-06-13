//! MCP tool for session → findings distillation (knowledge-system K-X9).
//!
//! # Trust class: AGENT proposal (B9-gated)
//!
//! Unlike the K-X6 finding tools (which land USER findings immediately), a
//! *distilled* finding is a CANDIDATE mined from a session transcript and is
//! therefore routed through the **B9 PROPOSAL GATE** exactly like K-M1
//! extraction.  `tdw.kg.distill` never writes the graph directly: every
//! candidate is submitted via [`ProposalQueue::submit`] as a `Validated`
//! [`ProposalKind::Annotation`] on an existing graph entity, invisible to reads
//! until the eval-gated promotion + K-L4 sweep lands it.  A distilled finding is
//! `pending-eval` — a candidate, never established knowledge until promoted.
//!
//! # Keyless / offline
//!
//! The default path is the deterministic extractive distiller in
//! [`tdw_knowledge::distillation`] — no model required.  A keyless install runs
//! it unchanged.  (A production LLM refinement seam is documented on the
//! library worker; it is off by default and cost-bounded when wired.)
//!
//! # Leakage safety (injected-now)
//!
//! The caller supplies `as_of` = the session time.  It is the ONLY timestamp
//! stamped onto the proposal history; no wall-clock is read for the gate write.
//! A replayed / back-dated session therefore never leaks a future `now` into the
//! audit trail.
//!
//! # Gate availability
//!
//! The tool requires the runtime to have the proposal queue + adaptivity
//! resolver + a host-bound agent identity attached (the same gate the
//! `tdw.kg.annotate` write surface needs).  When any is absent the tool returns
//! a loud `ToolFailure::Execution` rather than silently doing nothing.

use serde_json::{Map, Value, json};
use tdw_knowledge::distillation::{SessionDistiller, SessionTranscript, TranscriptTurn};
use tdw_knowledge::runtime::KnowledgeRuntime;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool names ──────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &["tdw.kg.distill"];

/// Whether `name` is the distill tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Caps (input validation) ──────────────────────────────────────────────────

/// Maximum number of transcript turns accepted in one distillation request.
/// Bounds request size and the deterministic scan; well above a typical session.
pub const MAX_TURNS: usize = 4_096;

/// Maximum characters of a single turn's text. Untrusted input cap; the library
/// claim builder caps the distilled claim separately.
pub const MAX_TURN_TEXT_CHARS: usize = 16_384;

/// Maximum characters for the session id / episode id fields.
pub const MAX_ID_CHARS: usize = 256;

// ── Descriptor ────────────────────────────────────────────────────────────────

/// Descriptor for `tdw.kg.distill` (K-X9).
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![distill_descriptor()]
}

fn distill_descriptor() -> ToolDescriptor {
    // Not read-only (enqueues proposals) and idempotent on the same transcript
    // (deterministic extraction; B9 duplicate-annotation rejection makes a
    // re-run a no-op against an already-pending finding).
    tool_with_annotations(
        "tdw.kg.distill",
        "Distill Session into Candidate Findings",
        "Distill a session transcript window into candidate research FINDINGS (a claim plus the \
         supporting episode evidence ids) and route each as a PROPOSAL through the B9 gate — \
         never a direct write. Findings are pending-eval candidates, invisible to reads until \
         promoted; they are NOT presented as established knowledge. Deterministic and keyless: \
         the default extractive path needs no model. Provenance: each proposal carries \
         agent_id=\"distill:<session_id>\" and a note recording the originating session and \
         supporting episode ids. Leakage-safe: the supplied as_of (= session time) is the only \
         timestamp stamped onto the proposal — no wall-clock is read. An empty session, or one \
         mentioning no known graph entity, returns an honest empty result. Requires the \
         knowledge runtime's proposal queue + adaptivity resolver + bound agent identity.",
        distill_input_schema(),
        false,
        true,
    )
}

fn distill_input_schema() -> Value {
    json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Stable session id. Sanitized into the provenance \
                                    agent_id=\"distill:<session_id>\" and the note envelope."
                },
                "as_of": {
                    "type": "string",
                    "description": "Leakage-safe session time (e.g. YYYY-MM-DD). The ONLY \
                                    timestamp stamped onto the proposal history."
                },
                "turns": {
                    "type": "array",
                    "description": "The transcript window, in order. Empty → honest empty result.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "episode_id": {
                                "type": "string",
                                "description": "Stable evidence id for this turn (the citation a \
                                                finding cites)."
                            },
                            "role": {
                                "type": "string",
                                "description": "Speaker role (informational; not used for anchoring)."
                            },
                            "text": {
                                "type": "string",
                                "description": "The turn text mined for known-entity mentions."
                            }
                        },
                        "required": ["episode_id", "text"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["session_id", "as_of", "turns"],
            "additionalProperties": false
        })
}

// ── Execute ─────────────────────────────────────────────────────────────────

/// Dispatch a distill tool call.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing gate (proposal queue /
/// adaptivity resolver / bound agent id), missing graph/tag engine, malformed
/// input, or a hard queue/engine failure — never [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.distill" => distill(runtime, arguments),
        other => Err(execution(format!("unknown distill tool: {other}"))),
    }
}

fn distill(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let transcript = parse_transcript(arguments)?;

    // Resolve the B9 gate handles. The bound agent id is host-bound; the
    // adaptivity resolver decides whether the gate admits the distilled
    // proposals (Learning+). A missing handle is a loud, honest error.
    let agent_id = runtime
        .bound_agent_id()
        .ok_or_else(|| execution("no agent identity bound to the distill surface".to_string()))?;
    let resolver = runtime
        .adaptivity_resolver()
        .ok_or_else(|| execution("knowledge adaptivity resolver not attached".to_string()))?;
    let adaptivity =
        resolver(agent_id).ok_or_else(|| execution(format!("unknown agent {agent_id:?}")))?;
    let proposals = runtime
        .proposals()
        .ok_or_else(|| execution("knowledge proposal queue not attached".to_string()))?;
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;
    let tags = runtime
        .tags()
        .ok_or_else(|| execution("knowledge tag engine not attached".to_string()))?;

    let mut distiller = SessionDistiller::new(
        std::sync::Arc::clone(proposals),
        std::sync::Arc::clone(graph),
        std::sync::Arc::clone(tags),
        adaptivity,
    );
    if let Some(cell) = runtime.distillation_freshness_cell() {
        distiller = distiller.with_freshness_cell(std::sync::Arc::clone(cell));
    }

    let result = block_on(async { distiller.distill(&transcript).await })
        .map_err(|error| execution(error.to_string()))?;

    let findings: Vec<Value> = result
        .findings
        .iter()
        .map(|f| {
            json!({
                "anchor_entity_id": f.anchor_entity_id,
                "claim": f.claim,
                "evidence_episode_ids": f.evidence_episode_ids,
                "proposal_id": f.proposal_id,
                "status": if f.proposal_id.is_some() { "proposed" } else { "skipped" },
            })
        })
        .collect();

    Ok(structured(json!({
        "session_id": result.session_id,
        "empty": result.empty,
        "enqueued": result.enqueued(),
        "skipped": result.skipped,
        // Pending-eval honesty: these are CANDIDATES routed through the gate,
        // invisible to reads until promoted — not established knowledge.
        "pending_eval": true,
        "findings": findings,
    })))
}

// ── Argument parsing ──────────────────────────────────────────────────────────

fn parse_transcript(arguments: &Map<String, Value>) -> Result<SessionTranscript, ToolFailure> {
    let session_id = require_str(arguments, "session_id")?;
    validate_id(session_id, "session_id")?;
    let as_of = require_str(arguments, "as_of")?;
    if as_of.trim().is_empty() {
        return Err(execution("as_of must not be empty".to_string()));
    }

    let turns_val = arguments
        .get("turns")
        .ok_or_else(|| execution("missing required argument: turns".to_string()))?;
    let turns_arr = turns_val
        .as_array()
        .ok_or_else(|| execution("turns must be an array".to_string()))?;
    if turns_arr.len() > MAX_TURNS {
        return Err(execution(format!(
            "turns has {} entries (max {MAX_TURNS})",
            turns_arr.len()
        )));
    }

    let mut turns = Vec::with_capacity(turns_arr.len());
    for (i, turn_val) in turns_arr.iter().enumerate() {
        let turn_obj = turn_val
            .as_object()
            .ok_or_else(|| execution(format!("turns[{i}] must be an object")))?;
        let episode_id = require_str(turn_obj, "episode_id")?;
        validate_id(episode_id, "episode_id")?;
        let text = require_str(turn_obj, "text")?;
        if text.chars().count() > MAX_TURN_TEXT_CHARS {
            return Err(execution(format!(
                "turns[{i}].text exceeds {MAX_TURN_TEXT_CHARS} characters"
            )));
        }
        let role = turn_obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        turns.push(TranscriptTurn {
            episode_id: episode_id.to_string(),
            role,
            text: text.to_string(),
        });
    }

    Ok(SessionTranscript {
        session_id: session_id.to_string(),
        as_of: as_of.to_string(),
        turns,
    })
}

fn validate_id(value: &str, field: &str) -> Result<(), ToolFailure> {
    if value.trim().is_empty() {
        return Err(execution(format!("{field} must not be empty")));
    }
    if value.chars().count() > MAX_ID_CHARS {
        return Err(execution(format!("{field} exceeds {MAX_ID_CHARS} characters")));
    }
    Ok(())
}

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(turns: &Value) -> Map<String, Value> {
        json!({
            "session_id": "sess-1",
            "as_of": "2026-06-13",
            "turns": turns,
        })
        .as_object()
        .expect("object literal")
        .clone()
    }

    #[test]
    fn owns_only_distill() {
        assert!(owns("tdw.kg.distill"));
        assert!(!owns("tdw.kg.finding"));
    }

    #[test]
    fn descriptor_is_not_read_only() {
        let d = &descriptors()[0];
        assert_eq!(d.name, "tdw.kg.distill");
        assert_eq!(d.annotations["readOnlyHint"], json!(false));
    }

    #[test]
    fn parse_transcript_happy_path() {
        let a = args(&json!([
            {"episode_id": "ep-0", "role": "user", "text": "AAPL?"},
            {"episode_id": "ep-1", "text": "AAPL beat"}
        ]));
        let Ok(t) = parse_transcript(&a) else {
            panic!("expected parse to succeed");
        };
        assert_eq!(t.session_id, "sess-1");
        assert_eq!(t.as_of, "2026-06-13");
        assert_eq!(t.turns.len(), 2);
        assert_eq!(t.turns[1].role, "", "missing role defaults empty");
    }

    #[test]
    fn parse_transcript_empty_turns_ok() {
        let Ok(t) = parse_transcript(&args(&json!([]))) else {
            panic!("expected parse to succeed");
        };
        assert!(t.turns.is_empty());
    }

    #[test]
    fn parse_transcript_rejects_missing_episode_id() {
        let a = args(&json!([{"text": "no id"}]));
        assert!(parse_transcript(&a).is_err());
    }

    #[test]
    fn parse_transcript_rejects_non_array_turns() {
        let a = json!({"session_id": "s", "as_of": "d", "turns": "nope"})
            .as_object()
            .expect("object literal")
            .clone();
        assert!(parse_transcript(&a).is_err());
    }
}
