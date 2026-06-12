//! The MCP knowledge FEEDBACK tool (knowledge-system B10 + K-L5).
//!
//! `tdw.kg.feedback` lets agents record which retrieval hits were helpful, feeding
//! the usage-aware consolidation planner in `tdw-agent-store`. It is **append-only
//! stats** — it does NOT mutate the graph, tags, or proposals directly.
//!
//! ## Gating
//!
//! The tool appears in `tools/list` only when ALL THREE conditions hold:
//! - A [`KnowledgeRuntime`] with a **bound agent id** is attached, AND
//! - A [`RetrievalFeedbackStore`] handle is attached to [`McpServer`] via
//!   [`McpServer::with_feedback_store`] / [`McpServer::set_feedback_store`].
//!
//! The store is NOT attached to the runtime; it is a separate field on the server.
//! Without the store OR without a bound agent id the tool is absent from the
//! catalog; a call to the name returns a tool error (never a protocol error),
//! matching the B8/B9 posture.
//!
//! ## Identity and trust model (K-L5 CLOSED)
//!
//! `agent_id` is **host-bound** (K-L5). It is set on the [`KnowledgeRuntime`] at
//! construction time from `[knowledge.agent] id` config and is NEVER accepted as a
//! tool argument. Remote callers cannot forge a different identity via the MCP
//! surface — any `agent_id` field in the call arguments is an unknown property and
//! the schema has `additionalProperties: false`, so it is rejected.
//!
//! The previous B10 caller-supplied model (bounded poisoning channel) is now
//! CLOSED: because the bearer token authenticates the one principal bound at
//! listener construction, a caller that reaches this surface IS that principal.
//! Multi-principal HTTP (distinct identity per authenticated connection) is
//! explicitly deferred as a follow-up story.
//!
//! The `agent_id` links to a memory name via the convention
//! `event.agent_id == memory.meta.base.name` OR `event.hit_ids` contains the
//! memory name. When neither matches, the event silently contributes no credit
//! to that memory — this is a deliberate no-op, not an error.
//!
//! ## Sync→async bridge
//!
//! The feedback store is behind a `tokio::sync::Mutex`. `execute_tool` is sync,
//! so the call bridges through [`crate::knowledge_tools::block_on`] exactly as
//! the B8 read tools do.

use serde_json::{Map, Value, json};
use std::sync::Arc;
use tdw_agent_store::{MAX_HIT_IDS, RetrievalEvent, RetrievalFeedbackStore};
use tdw_knowledge::runtime::{KnowledgeRuntime, KnowledgeVersions};
use tokio::sync::Mutex;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

/// The name this module owns.
pub const TOOL_NAME: &str = "tdw.kg.feedback";

/// Whether `name` is the knowledge feedback tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

/// Descriptor for `tdw.kg.feedback`. Appended to `tools/list` only when the
/// runtime has a bound agent id AND a feedback store attached (gated at the
/// `lib.rs` seam). Annotated as NOT read-only (it appends a record) and NOT
/// idempotent (each call creates a new event), matching the B9 write-tool
/// annotation shape.
///
/// K-L5: `agent_id` is removed from the schema — identity is host-bound from
/// `[knowledge.agent]` config, never accepted as an argument. The schema has
/// `additionalProperties: false` so a caller that passes `agent_id` gets a
/// protocol-level unknown-property rejection, not a silent accept.
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool_with_annotations(
        TOOL_NAME,
        "Record Retrieval Feedback",
        "Record which knowledge-graph hits were helpful. APPEND-ONLY usage stats — does NOT \
         mutate graph nodes, tags, or proposals. Feeds the usage-aware consolidation planner \
         so actively-referenced memories are retained longer. Agent identity is HOST-BOUND \
         (K-L5): it comes from the session principal configured at daemon start, never from \
         a tool argument. `hit_ids` is bounded to 64 entries; excess ids are silently \
         truncated. Requires the knowledge runtime with a bound agent id and a feedback \
         store attached.",
        json!({
            "type": "object",
            "properties": {
                "query_fingerprint": {
                    "type": "string",
                    "description": "A short fingerprint for the query (non-empty, max 256 bytes; e.g. a hash or normalised query string)."
                },
                "hit_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "The document / entity ids the agent considered helpful (max 64; excess truncated)."
                },
                "used": {
                    "type": "boolean",
                    "description": "Whether the agent found the hits helpful / used them. Defaults to false."
                }
            },
            "required": ["query_fingerprint"],
            "additionalProperties": false
        }),
        false, // not read-only
        false, // not idempotent
    )
}

/// Execute `tdw.kg.feedback`. Every failure is a tool error, never a protocol
/// error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for:
/// - No agent identity bound to the runtime (K-L5: must come from config).
/// - Missing or invalid `query_fingerprint`.
/// - A store validation failure (empty `embedder_model` from the runtime, etc.).
pub fn execute(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<RetrievalFeedbackStore>>,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    // K-L5: agent identity is HOST-BOUND — read from the runtime, never from
    // arguments. The schema has additionalProperties:false so a caller that
    // passes agent_id gets a protocol-level rejection before reaching here.
    let agent_id = runtime
        .bound_agent_id()
        .ok_or_else(|| execution("no agent identity bound to this feedback surface".to_string()))?;
    // Grammar is validated at config load (validate_principal_id) but re-check
    // here so the runtime contract holds even when identity is set programmatically.
    tdw_knowledge::proposals::validate_agent_id(agent_id)
        .map_err(|error| execution(error.to_string()))?;

    let query_fingerprint = require_str(arguments, "query_fingerprint")?;
    if query_fingerprint.trim().is_empty() {
        return Err(execution("query_fingerprint must not be empty".to_string()));
    }

    let hit_ids: Vec<String> = arguments
        .get("hit_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .take(MAX_HIT_IDS)
                .collect()
        })
        .unwrap_or_default();

    let used = arguments
        .get("used")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Stamp versions from the live runtime (never from arguments — caller
    // input is attacker-controlled; versions must come from the server).
    let versions: KnowledgeVersions = runtime.versions();

    let event = RetrievalEvent {
        agent_id: agent_id.to_string(),
        query_fingerprint: query_fingerprint.to_string(),
        hit_ids,
        versions,
        used,
        recorded_at: now.to_string(),
    };

    block_on(async {
        let mut guard = store.lock().await;
        guard.append(event)
    })
    .map_err(execution)?;

    Ok(structured(json!({
        "recorded": true,
        "agent_id": agent_id,
        "query_fingerprint": query_fingerprint,
    })))
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
