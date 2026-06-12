//! The MCP knowledge WRITE tools (knowledge-system B9).
//!
//! `tdw.tags.define`, `tdw.tags.assign`, and `tdw.kg.annotate` ENQUEUE gated
//! [`Proposal`](tdw_knowledge::proposals::Proposal)s — they NEVER write into the
//! graph/tag engines directly. `tdw.kg.proposals` lists proposals and runs the
//! operator actions (approve / reject / materialize). The whole surface is
//! exposed only when a [`KnowledgeRuntime`] is attached WITH a proposal queue
//! AND an adaptivity resolver AND a bound agent identity; otherwise the write
//! tools are absent from `tools/list`.
//!
//! Agent identity is HOST-BOUND: it is set on the [`KnowledgeRuntime`] at
//! construction time and never accepted as a tool argument. Remote callers
//! cannot assert a different identity via the MCP surface.
//!
//! Every failure here is a tool error ([`ToolFailure::Execution`]), never a
//! protocol error, exactly like the B8 read tools. The queue's `submit` /
//! `materialize_ready` are async while `execute_tool` is sync, so each tool
//! bridges through [`knowledge_tools::block_on`] — the same sync→async bridge
//! the read tools use.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tdw_core::GraphEngine;
use tdw_infer::{ChangeSet, InferEngine};
use tdw_knowledge::proposals::{ProposalKind, ProposalQueue};
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_tags::TagEngine;
use tdw_taxonomy::{Adaptivity, ValidationStatus};
use tokio::sync::Mutex;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

/// Inference context for the `materialize` action: engine lock + graph + tags.
type InferCtx = (
    Arc<Mutex<InferEngine>>,
    Arc<dyn GraphEngine>,
    Arc<dyn TagEngine>,
);

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &[
    "tdw.tags.define",
    "tdw.tags.assign",
    "tdw.kg.annotate",
    "tdw.kg.proposals",
];

/// Whether `name` is one of the knowledge write tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

/// Descriptors for the write tools, appended to `tools/list` only when the
/// runtime has the proposal queue + adaptivity resolver + bound agent id
/// attached (gated at the `lib.rs` seam). The three submit tools and
/// `tdw.kg.proposals` are all mutating (`readOnlyHint: false`,
/// `idempotentHint: false`).
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        define_descriptor(),
        assign_descriptor(),
        annotate_descriptor(),
        proposals_descriptor(),
    ]
}

fn write_tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    // Writes are NOT read-only and NOT idempotent (each submit enqueues a new
    // proposal id), so they carry the mutating annotation shape.
    tool_with_annotations(name, title, description, input_schema, false, false)
}

fn define_descriptor() -> ToolDescriptor {
    write_tool(
        "tdw.tags.define",
        "Propose Tag Definition",
        "ENQUEUE a proposal to define a new taxonomy tag (the agent's adaptivity must be at \
         least Learning to be admitted). Does NOT write the taxonomy directly — it returns the \
         proposal id + status; the tag only lands after the proposal reaches Ready (via eval \
         promotion or operator approval) and is materialized. Agent identity is host-bound.",
        json!({
            "type": "object",
            "properties": {
                "tag_id": { "type": "string", "description": "The new tag id, e.g. asset:equity:tech." },
                "parent": { "type": "string", "description": "Optional parent tag id (must already be defined)." }
            },
            "required": ["tag_id"],
            "additionalProperties": false
        }),
    )
}

fn assign_descriptor() -> ToolDescriptor {
    write_tool(
        "tdw.tags.assign",
        "Propose Tag Assignment",
        "ENQUEUE a proposal to assign a DEFINED tag to an existing entity (agent adaptivity must \
         be at least Learning). Does NOT write directly — returns the proposal id + status; the \
         assignment only lands once the proposal is Ready and materialized. Agent identity is \
         host-bound.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": { "type": "string", "description": "The entity to tag (must exist)." },
                "tag_id": { "type": "string", "description": "The tag to assign (must already be defined)." }
            },
            "required": ["entity_id", "tag_id"],
            "additionalProperties": false
        }),
    )
}

fn annotate_descriptor() -> ToolDescriptor {
    write_tool(
        "tdw.kg.annotate",
        "Propose Entity Annotation",
        "ENQUEUE a proposal to attach a free-text note to an existing entity (agent adaptivity \
         must be at least Learning). Does NOT write directly — returns the proposal id + status; \
         the annotation node + edge only land once the proposal is Ready and materialized. Agent \
         identity is host-bound.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": { "type": "string", "description": "The entity to annotate (must exist)." },
                "note": { "type": "string", "description": "The note (non-empty, bounded length, no control characters)." }
            },
            "required": ["entity_id", "note"],
            "additionalProperties": false
        }),
    )
}

fn proposals_descriptor() -> ToolDescriptor {
    write_tool(
        "tdw.kg.proposals",
        "Manage Gated Proposals",
        "List or act on gated write proposals. action=list returns a bounded page of proposals \
         (default 100, max 256) with `total` (all matching) and `proposals` (the page). \
         `agent_id` is a QUERY FILTER for list (not an identity claim — caller identity is \
         always host-bound from config); `limit` caps the page. action=approve / \
         action=reject / action=materialize are OPERATOR actions: they run ONLY on a runtime \
         granted operator authority, so an agent cannot approve or materialize its own \
         proposals — the whole point of the gate (B9 security review). approve/reject require \
         proposal_id (approve also takes approved_by; reject a reason); materialize writes \
         every Ready proposal into the engines, RE-VALIDATING each at write time, and returns \
         the report. Reads (tdw.kg.* / tdw.tags.query) exclude pending facts because only \
         materialized facts exist in the engines.",
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "approve", "reject", "materialize"], "description": "The proposal action." },
                "agent_id": { "type": "string", "description": "list: FILTER proposals to this agent id (query filter only — identity is host-bound, not set by this field)." },
                "limit": { "type": "integer", "description": "list: page size (default 100, max 256)." },
                "proposal_id": { "type": "string", "description": "approve/reject: the target proposal id." },
                "approved_by": { "type": "string", "description": "approve: the operator id (audit trail)." },
                "reason": { "type": "string", "description": "reject: the rejection reason (audit trail)." }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    )
}

/// Dispatch one knowledge write tool. Every failure is a tool error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing attachment, malformed input,
/// a refused admission, a validator failure, or an engine failure — never
/// [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    infer_ctx: Option<InferCtx>,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.tags.define" => submit_define(runtime, arguments),
        "tdw.tags.assign" => submit_assign(runtime, arguments),
        "tdw.kg.annotate" => submit_annotate(runtime, arguments),
        "tdw.kg.proposals" => proposals(runtime, infer_ctx, arguments),
        other => Err(execution(format!("unknown knowledge write tool: {other}"))),
    }
}

/// Today's UTC date (`YYYY-MM-DD`) — the `now` stamped on every gate transition.
/// Taken from the clock, never an argument: tool args are attacker-controlled.
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Gate an OPERATOR action: approve/reject/materialize run only when the
/// runtime was granted operator authority. An agent-facing runtime leaves it
/// off, so an agent cannot approve or materialize its own proposals — the
/// whole point of the gate (B9 security review).
fn require_operator(runtime: &KnowledgeRuntime, action: &str) -> Result<(), ToolFailure> {
    if runtime.operator_authority() {
        Ok(())
    } else {
        Err(execution(format!(
            "tdw.kg.proposals action {action:?} is an operator action and this runtime has no \
             operator authority — agents may submit and list, never approve or materialize"
        )))
    }
}

/// Resolve the calling agent's [`Adaptivity`] via the runtime's resolver. A
/// missing resolver OR an unknown agent is a tool error (writes unavailable).
fn resolve_adaptivity(
    runtime: &KnowledgeRuntime,
    agent_id: &str,
) -> Result<Adaptivity, ToolFailure> {
    let resolver = runtime
        .adaptivity_resolver()
        .ok_or_else(|| execution("knowledge adaptivity resolver not attached".to_string()))?;
    resolver(agent_id).ok_or_else(|| execution(format!("unknown agent {agent_id:?}")))
}

/// Borrowed handles to the validation engines + the gated queue.
type WriteContext<'a> = (
    &'a std::sync::Arc<dyn tdw_core::GraphEngine>,
    &'a std::sync::Arc<dyn tdw_tags::TagEngine>,
    &'a std::sync::Arc<tokio::sync::Mutex<ProposalQueue>>,
);

/// The validation engines + the queue. All three must be attached for writes.
fn write_context(runtime: &KnowledgeRuntime) -> Result<WriteContext<'_>, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;
    let tags = runtime
        .tags()
        .ok_or_else(|| execution("knowledge tag engine not attached".to_string()))?;
    let proposals = runtime
        .proposals()
        .ok_or_else(|| execution("knowledge proposal queue not attached".to_string()))?;
    Ok((graph, tags, proposals))
}

/// Obtain the host-bound agent identity, validating its grammar.
fn bound_agent_id(runtime: &KnowledgeRuntime) -> Result<&str, ToolFailure> {
    let agent_id = runtime
        .bound_agent_id()
        .ok_or_else(|| execution("no agent identity bound to this write surface".to_string()))?;
    tdw_knowledge::proposals::validate_agent_id(agent_id)
        .map_err(|error| execution(error.to_string()))?;
    Ok(agent_id)
}

/// Submit one proposal through the gate, returning `{ proposal_id, status }`.
fn submit(
    runtime: &KnowledgeRuntime,
    agent_id: &str,
    kind: ProposalKind,
) -> Result<ToolExecution, ToolFailure> {
    let adaptivity = resolve_adaptivity(runtime, agent_id)?;
    let (graph, tags, proposals) = write_context(runtime)?;
    let now = today();
    let proposal = block_on(async {
        let mut queue = proposals.lock().await;
        queue
            .submit(agent_id, adaptivity, kind, graph, tags, &now)
            .await
    })
    .map_err(|error| execution(error.to_string()))?;
    Ok(structured(json!({
        "proposal_id": proposal.id,
        "status": proposal.status,
    })))
}

fn submit_define(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let agent_id = bound_agent_id(runtime)?;
    let tag_id = require_str(arguments, "tag_id")?;
    let parent = optional_str(arguments, "parent").map(ToString::to_string);
    submit(
        runtime,
        agent_id,
        ProposalKind::TagDefine {
            tag_id: tag_id.to_string(),
            parent,
        },
    )
}

fn submit_assign(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let agent_id = bound_agent_id(runtime)?;
    let entity_id = require_str(arguments, "entity_id")?;
    let tag_id = require_str(arguments, "tag_id")?;
    submit(
        runtime,
        agent_id,
        ProposalKind::TagAssign {
            entity_id: entity_id.to_string(),
            tag_id: tag_id.to_string(),
        },
    )
}

fn submit_annotate(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let agent_id = bound_agent_id(runtime)?;
    let entity_id = require_str(arguments, "entity_id")?;
    let note = require_str(arguments, "note")?;
    submit(
        runtime,
        agent_id,
        ProposalKind::Annotation {
            entity_id: entity_id.to_string(),
            note: note.to_string(),
        },
    )
}

#[allow(clippy::significant_drop_tightening)] // ProposalPage<'a> borrows from the guard; guard must outlive the page borrow
fn proposals(
    runtime: &KnowledgeRuntime,
    infer_ctx: Option<InferCtx>,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let (graph, tags, queue) = write_context(runtime)?;
    let action = require_str(arguments, "action")?;
    let now = today();
    match action {
        "list" => {
            let agent_id = optional_str(arguments, "agent_id");
            let limit = arguments
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .map(|n| usize::try_from(n).unwrap_or(usize::MAX));
            let (proposals_val, total) = block_on(async {
                let (owned, total) = {
                    let guard = queue.lock().await;
                    let page = guard.list(agent_id, limit);
                    let owned: Vec<_> = page.proposals.into_iter().cloned().collect();
                    (owned, page.total)
                };
                serde_json::to_value(owned).map(|v| (v, total))
            })
            .map_err(|error| serde_failure(&error))?;
            Ok(structured(
                json!({ "proposals": proposals_val, "total": total }),
            ))
        }
        "approve" => {
            require_operator(runtime, "approve")?;
            let proposal_id = require_str(arguments, "proposal_id")?;
            let approved_by = optional_str(arguments, "approved_by").unwrap_or("operator");
            block_on(async {
                let mut queue = queue.lock().await;
                queue.approve(proposal_id, approved_by, &now)
            })
            .map_err(|error| execution(error.to_string()))?;
            Ok(structured(json!({ "approved": proposal_id })))
        }
        "reject" => {
            require_operator(runtime, "reject")?;
            let proposal_id = require_str(arguments, "proposal_id")?;
            let reason = optional_str(arguments, "reason").unwrap_or("rejected by operator");
            block_on(async {
                let mut queue = queue.lock().await;
                queue.reject(proposal_id, reason, &now)
            })
            .map_err(|error| execution(error.to_string()))?;
            Ok(structured(json!({ "rejected": proposal_id })))
        }
        "materialize" => {
            require_operator(runtime, "materialize")?;
            // Snapshot the kinds of all Ready+pending proposals BEFORE
            // materializing so we can build the ChangeSet for inference.
            // The queue lock is held across the snapshot + materialize to
            // keep them atomic (no other writer can sneak in).
            let (mat_report, ready_kinds) = block_on(async {
                let mut queue = queue.lock().await;
                // Snapshot kinds of Ready proposals before they are consumed.
                let ready_kinds: Vec<ProposalKind> = {
                    let page = queue.list(None, Some(usize::MAX));
                    page.proposals
                        .iter()
                        .filter(|p| p.status == ValidationStatus::Ready && p.is_pending())
                        .map(|p| p.kind.clone())
                        .collect()
                };
                let report = queue.materialize_ready(graph, tags, &now).await?;
                Ok::<_, tdw_knowledge::KnowledgeError>((report, ready_kinds))
            })
            .map_err(|error| execution(error.to_string()))?;

            // K-L1: fire incremental inference after materialization so derived
            // edges land for any edge or tag proposal that just materialized.
            // Best-effort: errors are logged, never surfaced — the proposals
            // are already durable at this point.
            if !mat_report.materialized.is_empty()
                && let Some(ctx) = infer_ctx
            {
                fire_infer_after_materialize(ctx, &ready_kinds, &now);
            }

            let report =
                serde_json::to_value(&mat_report).map_err(|error| serde_failure(&error))?;
            Ok(structured(json!({ "report": report })))
        }
        other => Err(execution(format!(
            "tdw.kg.proposals: unknown action {other:?} (expected list / approve / reject / \
             materialize)"
        ))),
    }
}

/// Fire `run_incremental` after proposals materialize (K-L1).
///
/// Builds a [`ChangeSet`] from the edge-type and tag-id kinds of the proposals
/// that were ready before materialization, then runs the engine. Errors are
/// logged to stderr and never surfaced — the proposals are already durable.
fn fire_infer_after_materialize(ctx: InferCtx, ready_kinds: &[ProposalKind], now: &str) {
    let (infer, graph, tags) = ctx;
    let mut changed = ChangeSet::default();
    for kind in ready_kinds {
        match kind {
            ProposalKind::Edge { rel, .. } => {
                changed.edge_types.insert(rel.clone());
            }
            ProposalKind::TagAssign { tag_id, .. } => {
                changed.tags.insert(tag_id.clone());
            }
            _ => {}
        }
    }
    if changed.edge_types.is_empty() && changed.tags.is_empty() {
        return;
    }
    let result = block_on(async {
        let mut guard = infer.lock().await;
        guard.run_incremental(&graph, &tags, now, &changed).await
    });
    match result {
        Ok(rep) if rep.derived_edges > 0 || rep.assigned_tags > 0 => {
            eprintln!(
                "tdw-mcp tdw.kg.proposals materialize: inference derived \
                 {} edge(s), {} tag(s) in {} iteration(s) (rule-set v{})",
                rep.derived_edges, rep.assigned_tags, rep.iterations, rep.version
            );
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!(
                "tdw-mcp tdw.kg.proposals materialize: inference failed \
                 (proposals already durable): {error}"
            );
        }
    }
}

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

fn serde_failure(error: &serde_json::Error) -> ToolFailure {
    ToolFailure::Execution(error.to_string())
}
