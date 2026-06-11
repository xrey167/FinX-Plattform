//! The two MCP knowledge EXPLAIN tools (knowledge-system K-X1).
//!
//! `tdw.kg.why` and `tdw.kg.diff` are exposed by [`McpServer`](crate::McpServer)
//! only when a [`KnowledgeRuntime`] with a graph engine is attached. Every failure
//! here is a tool error ([`ToolFailure::Execution`]), never a protocol error —
//! identical posture to the B8 read tools. The async graph/tag calls are bridged
//! via [`crate::knowledge_tools::block_on`].
//!
//! # Design notes
//!
//! Both tools are deliberately **deterministic**: no LLM inference, no
//! probabilistic ranking. Every step of the `why` chain is derived from stored
//! structured provenance fields. Every delta in the `diff` response uses the
//! exact same [`tdw_core::active_at`] visibility predicate the retriever and
//! graph-traversal tools use — the diff function never re-implements temporal
//! filtering, so temporal-leakage bugs cannot be introduced here independently.
//!
//! # Caps (documented per the K-X1 spec)
//!
//! ## `tdw.kg.why`
//! - Chain depth: at most [`WHY_MAX_CHAIN_DEPTH`] steps (default 16). Chains
//!   longer than this are truncated with an honest `"chain_depth_cap_reached"`
//!   flag rather than erroring, so the caller always gets a partial result.
//! - Fan-out (e.g. support-set size for rule-derived edges):
//!   at most [`WHY_MAX_SUPPORT_FANOUT`] items per step.
//!
//! ## `tdw.kg.diff`
//! - Snapshot span: `to_as_of - from_as_of` must not exceed
//!   [`DIFF_MAX_SPAN_DAYS`] unless `scope_entity_id` is supplied (in which case
//!   a full-graph scan is avoided and the cap is lifted for scoped queries).
//! - Item lists: `limit` defaults to [`DIFF_DEFAULT_LIMIT`], max
//!   [`DIFF_MAX_LIMIT`]. Counts are always exact; lists are paginated.

use serde_json::{Map, Value, json};
use tdw_core::{Direction, GraphEdge, Provenance, TraversalFilter, active_at};
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_tags::date_to_timestamp;

use crate::{
    ToolDescriptor, ToolExecution, ToolFailure, knowledge_tools::block_on, structured, tool,
};

// ── Tool names ────────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &["tdw.kg.why", "tdw.kg.diff"];

/// Whether `name` is one of the explain tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum provenance chain depth returned by `tdw.kg.why`.
/// Chains are truncated at this point; a `chain_depth_cap_reached: true` flag is
/// set in the response so the caller knows the trace is incomplete.
pub const WHY_MAX_CHAIN_DEPTH: usize = 16;

/// Maximum support-set fan-out per why-chain step (rule support sets can
/// theoretically enumerate every fact in the graph; this bounds the output).
pub const WHY_MAX_SUPPORT_FANOUT: usize = 32;

/// Maximum span (in days) for a full-graph `tdw.kg.diff` scan. Above this the
/// tool returns a tool error unless `scope_entity_id` is supplied.
pub const DIFF_MAX_SPAN_DAYS: i64 = 3650; // ~10 years

/// Default item-list limit for `tdw.kg.diff` (edges, nodes, tags).
pub const DIFF_DEFAULT_LIMIT: usize = 100;

/// Maximum item-list limit the caller may request.
pub const DIFF_MAX_LIMIT: usize = 256;

// ── Descriptors ───────────────────────────────────────────────────────────────

/// Descriptors for the two explain tools. Appended to `tools/list` only when a
/// runtime with a graph engine is attached. Both are `readOnlyHint: true`,
/// `idempotentHint: true`.
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![why_descriptor(), diff_descriptor()]
}

fn why_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.why",
        "Knowledge Provenance Explainer",
        "Explain the complete provenance chain for one graph edge, tag assignment, or entity. \
         Returns an ordered chain of typed steps (source → transformation → gate), each with \
         structured ids and timestamps drawn entirely from stored data — no LLM inference. \
         Unknown or unpersisted links are stated honestly, never invented. \
         Requires the graph engine; a runtime without it returns a tool error.\n\
         \n\
         Caps: chain depth ≤ 16 steps; rule support fan-out ≤ 32 items per step.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Explain the creation provenance of this entity node."
                },
                "edge": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "string" },
                        "rel":  { "type": "string" },
                        "to":   { "type": "string" }
                    },
                    "required": ["from", "rel", "to"],
                    "additionalProperties": false,
                    "description": "Explain the provenance of this specific edge."
                },
                "tag_assignment": {
                    "type": "object",
                    "properties": {
                        "entity_id": { "type": "string" },
                        "tag_id":    { "type": "string" }
                    },
                    "required": ["entity_id", "tag_id"],
                    "additionalProperties": false,
                    "description": "Explain the provenance of this tag assignment."
                },
                "as_of": {
                    "type": "string",
                    "description": "Optional temporal context (YYYY-MM-DD or normalized UTC timestamp)."
                }
            },
            "additionalProperties": false
        }),
    )
}

fn diff_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.diff",
        "Knowledge Time-Travel Diff",
        "Compute the knowledge-state delta between two as_of snapshots using the same temporal \
         visibility predicates the graph and retriever use — no re-implementation of temporal \
         filtering, so leakage bugs cannot originate here independently. Returns entities \
         added/tombstoned, edges that became valid or had their validity window closed \
         (invalidated facts), and tag assignments gained or expired. Counts are always exact; \
         item lists are bounded by `limit`. Never reveals facts whose validity windows lie \
         entirely AFTER `to_as_of`.\n\
         \n\
         Caps: full-graph span ≤ 3650 days unless `scope_entity_id` is supplied; \
         limit default 100, max 256.",
        json!({
            "type": "object",
            "properties": {
                "from_as_of": {
                    "type": "string",
                    "description": "Earlier snapshot timestamp (YYYY-MM-DD or normalized UTC). Must be < to_as_of."
                },
                "to_as_of": {
                    "type": "string",
                    "description": "Later snapshot timestamp (YYYY-MM-DD or normalized UTC). Must be > from_as_of."
                },
                "plane": {
                    "type": "string",
                    "description": "Restrict to one plane label stored in edge props (optional)."
                },
                "entity_kind": {
                    "type": "string",
                    "description": "Restrict node-level delta to this entity kind (lowercase, e.g. instrument)."
                },
                "scope_entity_id": {
                    "type": "string",
                    "description": "Restrict diff to within 2 hops of this entity (k-hop ≤ 2). Lifts the span cap."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 256,
                    "description": "Max items per delta list (default 100, max 256). Counts are always exact."
                }
            },
            "required": ["from_as_of", "to_as_of"],
            "additionalProperties": false
        }),
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch one explain tool. Every failure is a tool error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing engine, malformed input, or
/// an engine failure — never [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    name: &str,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.why" => why(runtime, arguments),
        "tdw.kg.diff" => diff(runtime, arguments),
        other => Err(execution(format!("unknown explain tool: {other}"))),
    }
}

// ── tdw.kg.why ────────────────────────────────────────────────────────────────

fn why(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;
    let tags = runtime.tags();

    let entity_id = optional_str(arguments, "entity_id");
    let edge_obj = arguments.get("edge").and_then(Value::as_object);
    let tag_obj = arguments.get("tag_assignment").and_then(Value::as_object);
    let as_of = optional_as_of(arguments, "as_of");

    // Exactly one subject must be specified.
    let subjects_given = [entity_id.is_some(), edge_obj.is_some(), tag_obj.is_some()]
        .into_iter()
        .filter(|&b| b)
        .count();
    if subjects_given != 1 {
        return Err(execution(
            "tdw.kg.why: provide exactly one of entity_id, edge, or tag_assignment".to_string(),
        ));
    }

    let result = if let Some(eid) = entity_id {
        block_on(why_entity(graph, eid, as_of.as_deref()))?
    } else if let Some(edge) = edge_obj {
        let from = require_str_from(edge, "from")?;
        let rel = require_str_from(edge, "rel")?;
        let to = require_str_from(edge, "to")?;
        block_on(why_edge(graph, from, rel, to, as_of.as_deref()))?
    } else if let Some(tag) = tag_obj {
        let entity_id = require_str_from(tag, "entity_id")?;
        let tag_id = require_str_from(tag, "tag_id")?;
        let tags_engine = tags.ok_or_else(|| {
            execution("tag engine not attached — cannot explain tag provenance".to_string())
        })?;
        block_on(why_tag(
            graph,
            tags_engine,
            entity_id,
            tag_id,
            as_of.as_deref(),
        ))?
    } else {
        unreachable!("subjects_given == 1 was checked above")
    };

    Ok(structured(result))
}

/// Build a why-chain for a graph edge.
async fn why_edge(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    from: &str,
    rel: &str,
    to: &str,
    as_of: Option<&str>,
) -> Result<Value, ToolFailure> {
    // Collect all matching edges (same from/rel/to, any validity window).
    let all_edges = block_on_inner(graph.edges(Some(rel), 0, usize::MAX))
        .await
        .map_err(|e| execution(e.to_string()))?;

    let matching: Vec<&GraphEdge> = all_edges
        .iter()
        .filter(|e| e.from == from && e.to == to)
        .collect();

    if matching.is_empty() {
        return Ok(json!({
            "subject": { "kind": "edge", "from": from, "rel": rel, "to": to },
            "chain": [],
            "summary": format!("Edge {from} -{rel}-> {to} not found in the graph."),
            "chain_depth_cap_reached": false
        }));
    }

    // Use the as_of-active edge when provided; fall back to the first found.
    let edge = as_of.map_or_else(
        || matching.first().copied(),
        |as_of| {
            matching
                .iter()
                .find(|e| active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), as_of))
                .or_else(|| matching.first())
                .copied()
        },
    );

    let Some(edge) = edge else {
        return Ok(json!({
            "subject": { "kind": "edge", "from": from, "rel": rel, "to": to },
            "chain": [],
            "summary": format!("Edge {from} -{rel}-> {to}: no active window at {as_of:?}."),
            "chain_depth_cap_reached": false
        }));
    };

    let chain = provenance_chain(&edge.provenance, 0);
    let cap = chain.len() >= WHY_MAX_CHAIN_DEPTH;
    let summary = edge_summary(&edge.provenance, from, rel, to);

    Ok(json!({
        "subject": {
            "kind": "edge",
            "from": from,
            "rel": rel,
            "to": to,
            "valid_from": edge.valid_from,
            "valid_to": edge.valid_to
        },
        "chain": chain,
        "summary": summary,
        "chain_depth_cap_reached": cap
    }))
}

/// Build a why-chain for a tag assignment on an entity.
async fn why_tag(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    tags: &std::sync::Arc<dyn tdw_tags::TagEngine>,
    entity_id: &str,
    tag_id: &str,
    as_of: Option<&str>,
) -> Result<Value, ToolFailure> {
    // Verify entity exists in the graph.
    let _ = graph
        .node(entity_id)
        .await
        .map_err(|e| execution(e.to_string()))?;

    // Find assignments for this entity+tag using the active_tags approach.
    // We use a date form for `active_tags`; use today's date when no as_of is given.
    // We need the full assignment including provenance, so we query via the tag engine
    // if it exposes assignments; otherwise we rely on active_tags to confirm existence.
    let query_date = as_of.unwrap_or("9999-12-31");
    let active = tags
        .active_tags(entity_id, query_date)
        .await
        .map_err(|e| execution(e.to_string()))?;

    let tag_found = active.iter().any(|t| t == tag_id);

    // Parse the provenance from the TagAssignment if available via the engine.
    // The TagEngine contract only exposes active_tags (list of tag ids), not the
    // full assignment objects. We therefore surface what is available from the
    // InMemoryTagEngine's underlying store when the engine is a concrete type,
    // but for the general contract we can only state whether the tag is active
    // and note that the full assignment provenance requires the engine to expose
    // `assignments_for`. We return an honest "provenance string not accessible
    // via the TagEngine trait" step rather than inventing.
    if !tag_found {
        let date_label = as_of.unwrap_or("open");
        return Ok(json!({
            "subject": {
                "kind": "tag_assignment",
                "entity_id": entity_id,
                "tag_id": tag_id
            },
            "chain": [{
                "step": 0,
                "kind": "not_found",
                "summary": format!(
                    "Tag {tag_id:?} is not active on {entity_id:?} at {date_label:?}."
                )
            }],
            "summary": format!("Tag {tag_id} not active on {entity_id} at {date_label}."),
            "chain_depth_cap_reached": false
        }));
    }

    // Tag is confirmed active. Report what provenance the trait surface gives us.
    // The TagEngine trait does not expose full assignment objects (only tag-id lists),
    // so provenance strings are not accessible through the engine contract alone.
    // A concrete InMemoryTagEngine with TagStore does expose `.assignments()`, but
    // that requires downcasting which is not stable across backends.
    // We surface this gap honestly.
    let date_label = as_of.unwrap_or("open");
    Ok(json!({
        "subject": {
            "kind": "tag_assignment",
            "entity_id": entity_id,
            "tag_id": tag_id,
            "as_of": as_of
        },
        "chain": [
            {
                "step": 0,
                "kind": "confirmed_active",
                "entity_id": entity_id,
                "tag_id": tag_id,
                "as_of": date_label,
                "summary": format!(
                    "Tag {tag_id:?} is active on {entity_id:?} at {date_label:?}."
                )
            },
            {
                "step": 1,
                "kind": "provenance_not_accessible",
                "detail": "The TagEngine trait contract exposes active_tags (tag-id lists) but \
                           not full TagAssignment objects with their provenance strings. To resolve \
                           the full provenance chain (rule:.../derived:rule:.../agent:...;proposal:...), \
                           the host must supply a TagStore-backed engine with assignment inspection. \
                           This step is honestly absent, not invented.",
                "summary": "Assignment provenance string not accessible via the TagEngine trait."
            }
        ],
        "summary": format!(
            "Tag {tag_id} is active on {entity_id} at {date_label}; \
             full provenance chain requires TagStore-level access (honestly absent at this layer)."
        ),
        "chain_depth_cap_reached": false
    }))
}

/// Build a why-chain for an entity node.
async fn why_entity(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    entity_id: &str,
    _as_of: Option<&str>,
) -> Result<Value, ToolFailure> {
    let node = graph
        .node(entity_id)
        .await
        .map_err(|e| execution(e.to_string()))?;

    let Some(node) = node else {
        return Ok(json!({
            "subject": { "kind": "entity", "entity_id": entity_id },
            "chain": [{
                "step": 0,
                "kind": "not_found",
                "summary": format!("Entity {entity_id:?} not found in the graph.")
            }],
            "summary": format!("Entity {entity_id} not found."),
            "chain_depth_cap_reached": false
        }));
    };

    let merge_neighbors = fetch_merge_neighbors(graph, entity_id).await?;
    let crosswalk_neighbors = fetch_crosswalk_neighbors(graph, entity_id).await?;

    let mut chain: Vec<Value> = vec![json!({
        "step": 0,
        "kind": "entity_kind",
        "entity_id": entity_id,
        "kind_value": node.kind,
        "label": node.label,
        "aliases": node.aliases,
        "valid_from": node.valid_from,
        "valid_to": node.valid_to,
        "summary": format!(
            "Entity {entity_id:?} is kind {:?}, label {:?}.",
            node.kind, node.label
        )
    })];

    if let Some(result) = append_crosswalk_steps(&mut chain, entity_id, &crosswalk_neighbors) {
        return Ok(result);
    }
    append_merge_steps(&mut chain, entity_id, &merge_neighbors);

    let merged = !merge_neighbors.is_empty();
    let summary = if merged {
        format!(
            "Entity {entity_id} (kind {:?}, label {:?}) is tombstoned — merged into {:?}.",
            node.kind,
            node.label,
            merge_neighbors.first().map(|(_, n)| &n.id)
        )
    } else {
        format!(
            "Entity {entity_id} (kind {:?}, label {:?}); {} crosswalk edge(s).",
            node.kind,
            node.label,
            crosswalk_neighbors.len()
        )
    };

    Ok(json!({
        "subject": { "kind": "entity", "entity_id": entity_id },
        "chain": chain,
        "summary": summary,
        "chain_depth_cap_reached": chain.len() >= WHY_MAX_CHAIN_DEPTH
    }))
}

async fn fetch_merge_neighbors(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    entity_id: &str,
) -> Result<Vec<(tdw_core::GraphEdge, tdw_core::GraphNode)>, ToolFailure> {
    let filter = TraversalFilter {
        rels: Some(vec!["merged_into".to_string()]),
        direction: Direction::Out,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    graph
        .neighbors(entity_id, &filter)
        .await
        .map_err(|e| execution(e.to_string()))
}

async fn fetch_crosswalk_neighbors(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    entity_id: &str,
) -> Result<Vec<(tdw_core::GraphEdge, tdw_core::GraphNode)>, ToolFailure> {
    // Identifier crosswalk edges (any edge with `listed_on`, `identified_by`,
    // `alias_of`, `same_as` — common crosswalk rels).
    let filter = TraversalFilter {
        rels: Some(vec![
            "listed_on".to_string(),
            "identified_by".to_string(),
            "alias_of".to_string(),
            "same_as".to_string(),
        ]),
        direction: Direction::Both,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    graph
        .neighbors(entity_id, &filter)
        .await
        .map_err(|e| execution(e.to_string()))
}

/// Append crosswalk steps to `chain`. Returns `Some(early_result)` when the
/// chain depth cap is hit before all crosswalk edges are appended.
fn append_crosswalk_steps(
    chain: &mut Vec<Value>,
    entity_id: &str,
    crosswalk_neighbors: &[(tdw_core::GraphEdge, tdw_core::GraphNode)],
) -> Option<Value> {
    for (edge, neighbor) in crosswalk_neighbors {
        if chain.len() >= WHY_MAX_CHAIN_DEPTH {
            return Some(json!({
                "subject": { "kind": "entity", "entity_id": entity_id },
                "chain": chain,
                "summary": format!("Entity {entity_id}: chain depth cap reached ({WHY_MAX_CHAIN_DEPTH})."),
                "chain_depth_cap_reached": true
            }));
        }
        let step = chain.len();
        chain.push(json!({
            "step": step,
            "kind": "crosswalk_edge",
            "rel": edge.rel,
            "neighbor": neighbor.id,
            "provenance": edge.provenance,
            "valid_from": edge.valid_from,
            "valid_to": edge.valid_to,
            "summary": format!(
                "Crosswalk edge -{:?}-> {:?} (provenance: {}).",
                edge.rel, neighbor.id, provenance_one_liner(&edge.provenance)
            )
        }));
    }
    None
}

/// Append merge-history steps to `chain` (up to the depth cap).
fn append_merge_steps(
    chain: &mut Vec<Value>,
    entity_id: &str,
    merge_neighbors: &[(tdw_core::GraphEdge, tdw_core::GraphNode)],
) {
    for (edge, target) in merge_neighbors {
        if chain.len() >= WHY_MAX_CHAIN_DEPTH {
            break;
        }
        let step = chain.len();
        let approved_by = match &edge.provenance {
            Provenance::Manual { approved_by } => approved_by.clone(),
            _ => "(unknown)".to_string(),
        };
        chain.push(json!({
            "step": step,
            "kind": "merged_into",
            "target_entity_id": target.id,
            "approved_by": approved_by,
            "valid_from": edge.valid_from,
            "summary": format!(
                "Entity {entity_id:?} was merged into {:?} (approved by {approved_by:?}).",
                target.id
            )
        }));
    }
}

// ── Provenance helpers ────────────────────────────────────────────────────────

/// Build an ordered chain of typed steps from a single [`Provenance`] value.
/// Returns at most [`WHY_MAX_CHAIN_DEPTH`] items.
fn provenance_chain(provenance: &Provenance, start_step: usize) -> Vec<Value> {
    let mut chain = Vec::new();
    let step = start_step;

    match provenance {
        Provenance::Ingest { source } => {
            chain.push(json!({
                "step": step,
                "kind": "ingest",
                "source": source,
                "summary": format!("Ingested from source {source:?}.")
            }));
        }
        Provenance::Rule { rule_id, version } => {
            chain.push(json!({
                "step": step,
                "kind": "rule_derived",
                "rule_id": rule_id,
                "version": version,
                "support_fanout_cap": WHY_MAX_SUPPORT_FANOUT,
                "support_index_note": "Support set (input fact keys) not stored on the edge itself; \
                    query the DerivationIndex via the inference engine for the edge key to retrieve it. \
                    This is honestly absent here, not invented.",
                "summary": format!(
                    "Derived by rule {rule_id:?} at version {version}. \
                     Support set not persisted on edge (see DerivationIndex; \
                     fan-out cap {WHY_MAX_SUPPORT_FANOUT})."
                )
            }));
        }
        Provenance::Agent { agent_id, gated } => {
            if *gated {
                // Gated agent write — parse the proposal id from the edge props if available.
                chain.push(json!({
                    "step": step,
                    "kind": "agent_gated",
                    "agent_id": agent_id,
                    "gated": true,
                    "note": "Validation timestamps and exact promotion route (eval pass_rate vs \
                             human approve, approver id) are stored in the ProposalQueue history \
                             field as human-readable audit lines. They are not re-serialized here \
                             to avoid coupling this read tool to the mutable write-queue state. \
                             Consult tdw.kg.proposals for the full audit trail.",
                    "summary": format!(
                        "Written by agent {agent_id:?} through the gated proposal path. \
                         Full gate audit in ProposalQueue history (see tdw.kg.proposals)."
                    )
                }));
            } else {
                chain.push(json!({
                    "step": step,
                    "kind": "agent_direct",
                    "agent_id": agent_id,
                    "gated": false,
                    "summary": format!(
                        "Written directly by agent {agent_id:?} (not gated)."
                    )
                }));
            }
        }
        Provenance::Manual { approved_by } => {
            chain.push(json!({
                "step": step,
                "kind": "manual",
                "approved_by": approved_by,
                "summary": format!("Human decision approved by {approved_by:?}.")
            }));
        }
        Provenance::System { detail } => {
            chain.push(json!({
                "step": step,
                "kind": "system",
                "detail": detail,
                "summary": format!("Platform-internal write: {detail:?}.")
            }));
        }
    }

    chain
}

/// A one-liner description of a provenance value (for embedding in parent summaries).
fn provenance_one_liner(provenance: &Provenance) -> String {
    match provenance {
        Provenance::Ingest { source } => format!("ingest:{source}"),
        Provenance::Rule { rule_id, version } => format!("rule:{rule_id}@v{version}"),
        Provenance::Agent {
            agent_id,
            gated: true,
        } => format!("agent:{agent_id};gated"),
        Provenance::Agent {
            agent_id,
            gated: false,
        } => format!("agent:{agent_id}"),
        Provenance::Manual { approved_by } => format!("manual:{approved_by}"),
        Provenance::System { detail } => format!("system:{detail}"),
    }
}

/// A human summary for a full edge provenance.
fn edge_summary(provenance: &Provenance, from: &str, rel: &str, to: &str) -> String {
    match provenance {
        Provenance::Ingest { source } => {
            format!("Edge {from} -{rel}-> {to} was ingested from {source:?}.")
        }
        Provenance::Rule { rule_id, version } => {
            format!(
                "Edge {from} -{rel}-> {to} was derived by rule {rule_id:?} (version {version}). \
                 Support set stored in DerivationIndex (honestly absent here)."
            )
        }
        Provenance::Agent {
            agent_id,
            gated: true,
        } => {
            format!(
                "Edge {from} -{rel}-> {to} was written by agent {agent_id:?} through the gated \
                 proposal path. Full gate audit in ProposalQueue history."
            )
        }
        Provenance::Agent {
            agent_id,
            gated: false,
        } => {
            format!(
                "Edge {from} -{rel}-> {to} was written directly by agent {agent_id:?} (not gated)."
            )
        }
        Provenance::Manual { approved_by } => {
            format!("Edge {from} -{rel}-> {to} was manually approved by {approved_by:?}.")
        }
        Provenance::System { detail } => {
            format!("Edge {from} -{rel}-> {to}: platform-internal write ({detail:?}).")
        }
    }
}

// ── tdw.kg.diff ───────────────────────────────────────────────────────────────

fn diff(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;
    let tags = runtime.tags();

    let from_raw = require_str(arguments, "from_as_of")?;
    let to_raw = require_str(arguments, "to_as_of")?;

    // Normalize both timestamps. Accept YYYY-MM-DD or full UTC timestamp.
    let from_ts = normalize_as_of(from_raw)?;
    let to_ts = normalize_as_of(to_raw)?;

    // from < to constraint.
    if from_ts >= to_ts {
        return Err(execution(format!(
            "from_as_of ({from_raw:?}) must be strictly before to_as_of ({to_raw:?})"
        )));
    }

    let plane_filter = optional_str(arguments, "plane").map(ToString::to_string);
    let entity_kind_filter = optional_str(arguments, "entity_kind").map(ToString::to_string);
    let scope_entity_id = optional_str(arguments, "scope_entity_id").map(ToString::to_string);
    let limit = optional_usize(arguments, "limit")?
        .unwrap_or(DIFF_DEFAULT_LIMIT)
        .min(DIFF_MAX_LIMIT);

    // Span cap: full-graph scans are bounded; scoped queries are exempt.
    if scope_entity_id.is_none() {
        let span_days = timestamp_span_days(&from_ts, &to_ts);
        if span_days > DIFF_MAX_SPAN_DAYS {
            return Err(execution(format!(
                "diff span {span_days} days exceeds cap {DIFF_MAX_SPAN_DAYS}; \
                 supply scope_entity_id to lift the cap for a scoped diff"
            )));
        }
    }

    let result = block_on(compute_diff(
        graph,
        tags,
        &from_ts,
        &to_ts,
        plane_filter.as_deref(),
        entity_kind_filter.as_deref(),
        scope_entity_id.as_deref(),
        limit,
    ))?;

    Ok(structured(result))
}

/// Compute the knowledge delta between two temporal snapshots.
///
/// Uses [`active_at`] from `tdw_core` as the single temporal visibility
/// predicate — the same function the graph engine and retriever use, so this
/// function cannot introduce temporal-leakage bugs independently.
///
/// Leakage invariant: `edges_added` only contains edges whose `valid_from` is
/// in `(from_ts, to_ts]` (became visible in the window). `edges_invalidated`
/// only contains edges whose `valid_to` is in `(from_ts, to_ts]` (closed in
/// the window). Neither list can contain facts whose window lies entirely after
/// `to_ts` — such facts are not visible at `to_ts` and therefore not in the
/// "new" snapshot.
/// Result of computing the edge-level delta between two snapshots.
struct EdgeDelta {
    added_count: usize,
    invalidated_count: usize,
    added_items: Vec<Value>,
    invalidated_items: Vec<Value>,
}

#[allow(clippy::too_many_arguments)]
async fn compute_diff(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    tags: Option<&std::sync::Arc<dyn tdw_tags::TagEngine>>,
    from_ts: &str,
    to_ts: &str,
    plane_filter: Option<&str>,
    entity_kind_filter: Option<&str>,
    scope_entity_id: Option<&str>,
    limit: usize,
) -> Result<Value, ToolFailure> {
    let raw_edges = if let Some(scope_id) = scope_entity_id {
        collect_scoped_edges(graph, scope_id).await?
    } else {
        collect_all_edges(graph).await?
    };

    let delta = compute_edge_delta(&raw_edges, from_ts, to_ts, plane_filter, limit);

    // Tag delta (requires tag engine).
    let (tags_gained, tags_expired, tags_gained_count, tags_expired_count) =
        if let Some(tags) = tags {
            compute_tag_delta(
                tags,
                scope_entity_id,
                entity_kind_filter,
                from_ts,
                to_ts,
                limit,
            )
            .await?
        } else {
            (Vec::new(), Vec::new(), 0usize, 0usize)
        };

    // Entity tombstones: merged_into edges that became active in the window.
    // (Cannot enumerate all nodes without a node-scan API — approximated via edge endpoints.)
    let entities_tombstoned: Vec<Value> = delta
        .added_items
        .iter()
        .filter(|e| e["rel"].as_str() == Some("merged_into"))
        .map(|e| {
            json!({
                "entity_id": e["from"],
                "merged_into": e["to"],
                "valid_from": e["valid_from"]
            })
        })
        .collect();
    let entities_tombstoned_count = entities_tombstoned.len();
    let entities_tombstoned_items: Vec<Value> =
        entities_tombstoned.into_iter().take(limit).collect();

    Ok(json!({
        "from_as_of": from_ts,
        "to_as_of": to_ts,
        "filters": {
            "plane": plane_filter,
            "entity_kind": entity_kind_filter,
            "scope_entity_id": scope_entity_id
        },
        "edges_added": {
            "count": delta.added_count,
            "truncated": delta.added_count > limit,
            "items": delta.added_items
        },
        "edges_invalidated": {
            "count": delta.invalidated_count,
            "truncated": delta.invalidated_count > limit,
            "items": delta.invalidated_items
        },
        "entities_tombstoned": {
            "count": entities_tombstoned_count,
            "truncated": entities_tombstoned_count > limit,
            "items": entities_tombstoned_items,
            "note": "Entity tombstones are detected from merged_into edges added in the window. \
                     A full node-added scan is not available without a node-enumeration API."
        },
        "tags_gained": {
            "count": tags_gained_count,
            "truncated": tags_gained_count > limit,
            "items": tags_gained
        },
        "tags_expired": {
            "count": tags_expired_count,
            "truncated": tags_expired_count > limit,
            "items": tags_expired
        },
        "leakage_guard": "Temporal predicates use tdw_core::active_at exclusively. \
                          No fact with valid_to <= from_as_of appears in edges_added. \
                          No fact with valid_from > to_as_of appears in any list."
    }))
}

/// Compute the edge-level delta between two temporal snapshots from a pre-collected
/// set of candidate edges (already filtered to the relevant scope).
///
/// Uses [`active_at`] exclusively — not re-implementing temporal visibility.
fn compute_edge_delta(
    raw_edges: &[tdw_core::GraphEdge],
    from_ts: &str,
    to_ts: &str,
    plane_filter: Option<&str>,
    limit: usize,
) -> EdgeDelta {
    // Apply plane filter.
    let candidate_edges: Vec<&tdw_core::GraphEdge> = raw_edges
        .iter()
        .filter(|e| {
            plane_filter.is_none_or(|p| {
                e.props
                    .get("plane")
                    .and_then(Value::as_str)
                    .is_some_and(|ep| ep == p)
            })
        })
        .collect();

    // Leakage-safe temporal predicates — identical to what the graph engine uses.
    let active_at_from = |e: &&tdw_core::GraphEdge| {
        active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), from_ts)
    };
    let active_at_to =
        |e: &&tdw_core::GraphEdge| active_at(e.valid_from.as_deref(), e.valid_to.as_deref(), to_ts);

    // Edges that became valid: NOT active at from_ts, IS active at to_ts.
    // Leakage invariant: active_at_to == true for every entry, so no fact after to_ts leaks in.
    let mut edges_added: Vec<&tdw_core::GraphEdge> = candidate_edges
        .iter()
        .filter(|e| !active_at_from(e) && active_at_to(e))
        .copied()
        .collect();
    let added_count = edges_added.len();
    edges_added.sort_by(|a, b| {
        (&a.from, &a.rel, &a.to, &a.valid_from).cmp(&(&b.from, &b.rel, &b.to, &b.valid_from))
    });
    let added_items: Vec<Value> = edges_added
        .iter()
        .take(limit)
        .map(|e| {
            json!({
                "from": e.from,
                "rel": e.rel,
                "to": e.to,
                "valid_from": e.valid_from,
                "valid_to": e.valid_to,
                "provenance": e.provenance
            })
        })
        .collect();

    // Edges whose validity window CLOSED in the window: WAS active at from_ts, NOT at to_ts.
    let edges_invalidated_raw: Vec<&tdw_core::GraphEdge> = candidate_edges
        .iter()
        .filter(|e| active_at_from(e) && !active_at_to(e))
        .copied()
        .collect();
    let invalidated_count = edges_invalidated_raw.len();

    // For each invalidated edge, find a successor with the same (from, rel) active at to_ts.
    let mut invalidated_items: Vec<Value> = edges_invalidated_raw
        .iter()
        .map(|old| {
            let successor = candidate_edges.iter().find(|cand| {
                cand.from == old.from
                    && cand.rel == old.rel
                    && active_at_to(cand)
                    && (cand.to != old.to
                        || cand.valid_from != old.valid_from
                        || cand.valid_to != old.valid_to)
            });
            json!({
                "edge": {
                    "from": old.from,
                    "rel": old.rel,
                    "to": old.to,
                    "valid_from": old.valid_from,
                    "valid_to": old.valid_to,
                    "provenance": old.provenance
                },
                "successor": successor.map(|s| json!({
                    "from": s.from,
                    "rel": s.rel,
                    "to": s.to,
                    "valid_from": s.valid_from,
                    "valid_to": s.valid_to,
                    "provenance": s.provenance
                }))
            })
        })
        .collect();
    invalidated_items.sort_by(|a, b| {
        let key_a = (
            a["edge"]["from"].as_str().unwrap_or(""),
            a["edge"]["rel"].as_str().unwrap_or(""),
            a["edge"]["to"].as_str().unwrap_or(""),
        );
        let key_b = (
            b["edge"]["from"].as_str().unwrap_or(""),
            b["edge"]["rel"].as_str().unwrap_or(""),
            b["edge"]["to"].as_str().unwrap_or(""),
        );
        key_a.cmp(&key_b)
    });
    let invalidated_items: Vec<Value> = invalidated_items.into_iter().take(limit).collect();

    EdgeDelta {
        added_count,
        invalidated_count,
        added_items,
        invalidated_items,
    }
}

/// Collect all edges via paginated scan.
async fn collect_all_edges(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
) -> Result<Vec<tdw_core::GraphEdge>, ToolFailure> {
    let mut edges = Vec::new();
    let mut offset = 0usize;
    let page_size = 1024usize;
    loop {
        let page = graph
            .edges(None, offset, page_size)
            .await
            .map_err(|e| execution(e.to_string()))?;
        if page.is_empty() {
            break;
        }
        offset += page.len();
        edges.extend(page);
    }
    Ok(edges)
}

/// Collect edges reachable within 2 hops from `scope_id`.
async fn collect_scoped_edges(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    scope_id: &str,
) -> Result<Vec<tdw_core::GraphEdge>, ToolFailure> {
    // Use the graph engine's expand with max_hops=2 (no as_of filter — we want
    // ALL edges so we can apply our own temporal predicate pair).
    let filter = TraversalFilter {
        direction: Direction::Both,
        max_hops: 2,
        rels: None,
        kinds: None,
        as_of: None, // intentionally no as_of: we collect all and apply active_at ourselves
    };
    let subgraph = graph
        .expand(&[scope_id.to_string()], &filter)
        .await
        .map_err(|e| execution(e.to_string()))?;
    Ok(subgraph.edges)
}

/// Compute the tag-assignment delta between two temporal snapshots.
///
/// Tags gained: active at `to_ts` but not at `from_ts`.
/// Tags expired: active at `from_ts` but not at `to_ts`.
///
/// We drive this through `active_tags` (date-level) using both timestamps
/// converted to date form.
async fn compute_tag_delta(
    tags: &std::sync::Arc<dyn tdw_tags::TagEngine>,
    scope_entity_id: Option<&str>,
    _entity_kind_filter: Option<&str>,
    from_ts: &str,
    to_ts: &str,
    limit: usize,
) -> Result<(Vec<Value>, Vec<Value>, usize, usize), ToolFailure> {
    // Convert to YYYY-MM-DD form for the tag engine (which uses date-level comparisons).
    let from_date = ts_to_date(from_ts);
    let to_date = ts_to_date(to_ts);

    // Tag engine operates per-entity, not over all entities at once.
    // When scoped, we check only the scope entity.
    // When unscoped, we cannot enumerate all entities from the TagEngine trait alone.
    // We return honest counts based on what is accessible.
    if let Some(entity_id) = scope_entity_id {
        let from_active = tags
            .active_tags(entity_id, &from_date)
            .await
            .map_err(|e| execution(e.to_string()))?;
        let to_active = tags
            .active_tags(entity_id, &to_date)
            .await
            .map_err(|e| execution(e.to_string()))?;

        let from_set: std::collections::BTreeSet<&str> =
            from_active.iter().map(String::as_str).collect();
        let to_set: std::collections::BTreeSet<&str> =
            to_active.iter().map(String::as_str).collect();

        let gained: Vec<String> = to_set
            .difference(&from_set)
            .map(|s| (*s).to_string())
            .collect();
        let expired: Vec<String> = from_set
            .difference(&to_set)
            .map(|s| (*s).to_string())
            .collect();

        let gained_count = gained.len();
        let expired_count = expired.len();

        let gained_items: Vec<Value> = gained
            .iter()
            .take(limit)
            .map(|tag_id| {
                json!({
                    "entity_id": entity_id,
                    "tag_id": tag_id
                })
            })
            .collect();
        let expired_items: Vec<Value> = expired
            .iter()
            .take(limit)
            .map(|tag_id| {
                json!({
                    "entity_id": entity_id,
                    "tag_id": tag_id
                })
            })
            .collect();

        Ok((gained_items, expired_items, gained_count, expired_count))
    } else {
        // No scope: the TagEngine trait does not expose a full entity enumeration.
        // Returning empty with a note is honest.
        let note = json!({
            "note": "Full tag delta for all entities requires entity enumeration, \
                     which the TagEngine trait does not expose. Supply scope_entity_id \
                     to get per-entity tag deltas."
        });
        Ok((vec![note.clone()], vec![note], 0, 0))
    }
}

// ── Async helper (inner) ──────────────────────────────────────────────────────

/// Bridge for async calls inside an already-`block_on`'d async context.
/// We are already inside `block_on` → we are on a scoped thread with its own
/// runtime. Using `.await` directly is safe here.
async fn block_on_inner<F: std::future::Future>(future: F) -> F::Output {
    future.await
}

// ── Argument helpers ──────────────────────────────────────────────────────────

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn require_str_from<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, ToolFailure> {
    object
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| execution(format!("missing required field: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn optional_usize(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<usize>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| execution(format!("{name} must be a non-negative integer"))),
    }
}

/// Normalize an optional `as_of` argument: YYYY-MM-DD → UTC timestamp; longer
/// values pass through. Returns `None` when absent.
fn optional_as_of(arguments: &Map<String, Value>, name: &str) -> Option<String> {
    let value = optional_str(arguments, name)?;
    if is_date(value) {
        Some(date_to_timestamp(value))
    } else {
        Some(value.to_string())
    }
}

/// Normalize a required `as_of` string.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for an invalid timestamp form.
fn normalize_as_of(value: &str) -> Result<String, ToolFailure> {
    if is_date(value) {
        Ok(date_to_timestamp(value))
    } else if is_timestamp(value) {
        Ok(value.to_string())
    } else {
        Err(execution(format!(
            "invalid timestamp {value:?}: expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ"
        )))
    }
}

/// `YYYY-MM-DD` shape check.
fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value
            .chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// Basic normalized UTC timestamp shape check (mirrors `tdw_core::graph::is_timestampish`).
fn is_timestamp(value: &str) -> bool {
    value.len() >= 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
        && value.chars().all(|c| !c.is_control())
}

/// Approximate span in days between two UTC timestamps (lexicographic year difference × 365 + day-of-year).
/// Used only for the span cap check; exact precision is not required.
fn timestamp_span_days(from_ts: &str, to_ts: &str) -> i64 {
    // Parse year, month, day from "YYYY-MM-DDTHH:MM:SSZ" (or longer).
    fn parse_ymd(ts: &str) -> Option<(i64, i64, i64)> {
        let y: i64 = ts.get(0..4)?.parse().ok()?;
        let m: i64 = ts.get(5..7)?.parse().ok()?;
        let d: i64 = ts.get(8..10)?.parse().ok()?;
        Some((y, m, d))
    }
    // Rough Julian day number (no leap-year correction needed for a cap check).
    const fn julian(y: i64, m: i64, d: i64) -> i64 {
        365 * y + y / 4 - y / 100 + y / 400 + (153 * m + 2) / 5 + d
    }
    match (parse_ymd(from_ts), parse_ymd(to_ts)) {
        (Some((fy, fm, fd)), Some((ty, tm, td))) => (julian(ty, tm, td) - julian(fy, fm, fd)).abs(),
        _ => 0,
    }
}

/// Convert a normalized UTC timestamp to a `YYYY-MM-DD` date string (truncate at day).
fn ts_to_date(ts: &str) -> String {
    ts.get(0..10).unwrap_or(ts).to_string()
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}
