//! The MCP knowledge FEEDBACK tool (knowledge-system B10).
//!
//! `tdw.kg.feedback` lets agents record which retrieval hits were helpful, feeding
//! the usage-aware consolidation planner in `tdw-agent-store`. It is **append-only
//! stats** — it does NOT mutate the graph, tags, or proposals directly.
//!
//! ## Gating
//!
//! The tool appears in `tools/list` only when:
//! - A [`KnowledgeRuntime`] is attached, AND
//! - The runtime has a [`RetrievalFeedbackStore`] handle attached (via
//!   [`KnowledgeRuntime::with_feedback_store`]).
//!
//! Without the store the tool is absent from the catalog; a call to the name
//! returns a tool error (never a protocol error), matching the B8/B9 posture.
//!
//! ## Identity
//!
//! `agent_id` is a caller-supplied argument here (not host-bound like the B9
//! write tools), because feedback recording is a safe append-only operation — it
//! does not land graph mutations. The grammar is the same as B9 (validated via
//! [`tdw_knowledge::proposals::validate_agent_id`]).
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
/// runtime has a feedback store attached (gated at the `lib.rs` seam).
/// Annotated as NOT read-only (it appends a record) and NOT idempotent (each
/// call creates a new event), matching the B9 write-tool annotation shape.
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool_with_annotations(
        TOOL_NAME,
        "Record Retrieval Feedback",
        "Record which knowledge-graph hits were helpful. APPEND-ONLY usage stats — does NOT \
         mutate graph nodes, tags, or proposals. Feeds the usage-aware consolidation planner \
         so actively-referenced memories are retained longer. `agent_id` is validated \
         (no `:`, `;`, or control characters; max 128 bytes). `hit_ids` is bounded to \
         64 entries; excess ids are silently truncated. Requires the knowledge runtime \
         with a feedback store attached.",
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "The calling agent's id (grammar: no `:`/`;`/control chars, max 128 bytes)."
                },
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
            "required": ["agent_id", "query_fingerprint"],
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
/// - A missing feedback store attachment.
/// - Missing or invalid `agent_id` / `query_fingerprint`.
/// - A store validation failure (empty `embedder_model` from the runtime, etc.).
pub fn execute(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<RetrievalFeedbackStore>>,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    // Validate agent_id grammar (same rules as B9 proposals).
    let agent_id = require_str(arguments, "agent_id")?;
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
    let versions: KnowledgeVersions = runtime.versions().clone();

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
