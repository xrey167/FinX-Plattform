//! The five MCP knowledge READ tools (knowledge-system B8).
//!
//! `tdw.kg.search`, `tdw.kg.entity`, `tdw.kg.traverse`, `tdw.kg.path`, and
//! `tdw.tags.query` are exposed by [`McpServer`](crate::McpServer) only when a
//! [`KnowledgeRuntime`] is attached. Every failure here — a missing engine, a
//! malformed argument, an out-of-range hop budget — is a tool error
//! ([`ToolFailure::Execution`]), never a protocol error: the plan requires the
//! knowledge surface to fail honestly inside the tool result.
//!
//! The retriever and the graph/tag engines are async while `execute_tool` is
//! sync, so each tool bridges through a tiny current-thread `tokio` runtime
//! exactly as the daemon ops surface does (`ops.rs`).

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tdw_core::{Direction, GraphEngine, TraversalFilter};
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_retrieve::{GraphExpansion, KnowledgeQuery, QueryFilter};
use tdw_tags::date_to_timestamp;
use tdw_taxonomy::EntityKind;

use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool};

/// The names this module owns. `tdw.tags.query` lives here too (a read tool over
/// the tag engine), so the dispatch layer can route the whole set by prefix-free
/// membership.
pub const TOOL_NAMES: &[&str] = &[
    "tdw.kg.search",
    "tdw.kg.entity",
    "tdw.kg.traverse",
    "tdw.kg.path",
    "tdw.tags.query",
];

/// Whether `name` is one of the knowledge read tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

/// Descriptors for the five read tools, appended to `tools/list` only when a
/// runtime is attached. All are `readOnlyHint: true`, `idempotentHint: true`.
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        search_descriptor(),
        entity_descriptor(),
        traverse_descriptor(),
        path_descriptor(),
        tags_query_descriptor(),
    ]
}

fn search_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.search",
        "Knowledge Hybrid Search",
        "Hybrid retrieval (vector + lexical + temporal tag channels) with optional graph \
         expansion. Read-only. Returns explained hits (which channels matched, matched tags, \
         graph path, seed hit) and the knowledge versions (embedder_model, rules_version, \
         infer_version).",
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Free-text query (non-empty)." },
                "top_k": { "type": "integer", "minimum": 1, "maximum": 256, "description": "Fused result size, default 8." },
                "tags_any": { "type": "array", "items": { "type": "string" }, "description": "Match any of these tags (taxonomy descendants included)." },
                "entity_kinds": { "type": "array", "items": { "type": "string" }, "description": "Restrict to these entity kinds (lowercase tokens, e.g. instrument)." },
                "plane": { "type": "string", "description": "Restrict to one plane (agent / platform / shared)." },
                "as_of": { "type": "string", "description": "Temporal point of view, YYYY-MM-DD. Excludes undated documents." },
                "expand": {
                    "type": "object",
                    "properties": {
                        "k_hop": { "type": "integer", "minimum": 1, "maximum": 3 },
                        "edge_types": { "type": "array", "items": { "type": "string" } },
                        "per_hit_limit": { "type": "integer", "minimum": 1, "maximum": 64 },
                        "decay": { "type": "number", "exclusiveMinimum": 0, "maximum": 1 }
                    },
                    "required": ["k_hop", "per_hit_limit", "decay"],
                    "additionalProperties": false,
                    "description": "Graph expansion of the top fused hits."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    )
}

fn entity_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.entity",
        "Knowledge Graph Entity",
        "Fetch one graph node and its one-hop neighborhood (both directions). Read-only. \
         Requires the knowledge graph; a runtime without it returns a tool error.",
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Graph node id, e.g. instrument:AAPL." }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
    )
}

fn traverse_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.traverse",
        "Knowledge Graph Traverse",
        "Breadth-first expansion from seed nodes into a subgraph with structured provenance \
         VISIBLE on every edge (derived:* / Rule provenance is shown). Read-only. Hop budget \
         is capped at 3.",
        json!({
            "type": "object",
            "properties": {
                "seeds": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 16, "description": "Seed node ids (1..=16)." },
                "rels": { "type": "array", "items": { "type": "string" }, "description": "Restrict to these relationship types." },
                "kinds": { "type": "array", "items": { "type": "string" }, "description": "Restrict reached nodes to these entity kinds." },
                "as_of": { "type": "string", "description": "Temporal point of view (YYYY-MM-DD or normalized timestamp)." },
                "direction": { "type": "string", "enum": ["out", "in", "both"], "description": "Traversal direction, default both." },
                "max_hops": { "type": "integer", "minimum": 1, "maximum": 3, "description": "Hop budget 1..=3, default 1." }
            },
            "required": ["seeds"],
            "additionalProperties": false
        }),
    )
}

fn path_descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.path",
        "Knowledge Graph Path",
        "Shortest path between two graph nodes as an ordered edge list, or null when \
         unreachable. Read-only. Hop budget 1..=8.",
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Source node id." },
                "to": { "type": "string", "description": "Target node id." },
                "rels": { "type": "array", "items": { "type": "string" }, "description": "Restrict to these relationship types." },
                "max_hops": { "type": "integer", "minimum": 1, "maximum": 8, "description": "Hop budget 1..=8, default 4." }
            },
            "required": ["from", "to"],
            "additionalProperties": false
        }),
    )
}

fn tags_query_descriptor() -> ToolDescriptor {
    tool(
        "tdw.tags.query",
        "Knowledge Tag Query",
        "Query the temporal tag engine. Read-only. Exactly one mode: entity_id+as_of returns \
         active tags; tag_id alone returns descendants; tag_id+as_of returns entities holding \
         the tag. Requires the tag engine.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": { "type": "string", "description": "With as_of: active tags on this entity." },
                "tag_id": { "type": "string", "description": "Alone: descendants; with as_of: entities holding the tag." },
                "as_of": { "type": "string", "description": "Temporal point of view, YYYY-MM-DD." }
            },
            "additionalProperties": false
        }),
    )
}

/// Dispatch one knowledge read tool. Every failure is a tool error.
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
        "tdw.kg.search" => search(runtime, arguments),
        "tdw.kg.entity" => entity(runtime, arguments),
        "tdw.kg.traverse" => traverse(runtime, arguments),
        "tdw.kg.path" => path(runtime, arguments),
        "tdw.tags.query" => tags_query(runtime, arguments),
        other => Err(execution(format!("unknown knowledge tool: {other}"))),
    }
}

/// Query-string ceiling: tool arguments are attacker-controlled, and API
/// embedders bill per token — an unbounded query is a cost/DoS lever.
const MAX_QUERY_CHARS: usize = 8192;

fn search(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let query = require_str(arguments, "query")?;
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(execution(format!(
            "query must be at most {MAX_QUERY_CHARS} characters"
        )));
    }
    let top_k = optional_usize(arguments, "top_k")?.unwrap_or(8);
    let filter = search_filter(arguments)?;
    let expand = optional_expansion(arguments)?;
    let knowledge_query = KnowledgeQuery::try_new(query, top_k, filter, expand)
        .map_err(|error| execution(error.to_string()))?;
    let hits = block_on(runtime.retriever().search(&knowledge_query))
        .map_err(|error| execution(error.to_string()))?;
    let hits = serde_json::to_value(&hits).map_err(|error| serde_failure(&error))?;
    let versions =
        serde_json::to_value(runtime.versions()).map_err(|error| serde_failure(&error))?;
    Ok(structured(json!({ "hits": hits, "versions": versions })))
}

/// Build the cross-channel query filter from the search arguments.
fn search_filter(arguments: &Map<String, Value>) -> Result<QueryFilter, ToolFailure> {
    Ok(QueryFilter {
        tags_any: optional_string_array(arguments, "tags_any")?.unwrap_or_default(),
        entity_kinds: optional_entity_kinds(arguments, "entity_kinds")?,
        plane: optional_str(arguments, "plane").map(ToString::to_string),
        as_of: optional_str(arguments, "as_of").map(ToString::to_string),
    })
}

fn entity(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let id = require_str(arguments, "id")?;
    let graph = require_graph(runtime)?;
    let filter = TraversalFilter {
        direction: Direction::Both,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    let (node, neighbors) = block_on(async {
        let node = graph.node(id).await?;
        let neighbors = graph.neighbors(id, &filter).await?;
        Ok::<_, tdw_core::Error>((node, neighbors))
    })
    .map_err(|error| execution(error.to_string()))?;
    let node = serde_json::to_value(&node).map_err(|error| serde_failure(&error))?;
    let neighbors: Vec<Value> = neighbors
        .into_iter()
        .map(|(edge, node)| json!({ "edge": edge, "node": node }))
        .collect();
    Ok(structured(json!({ "node": node, "neighbors": neighbors })))
}

fn traverse(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let seeds = require_string_array(arguments, "seeds")?;
    if seeds.is_empty() {
        return Err(execution("seeds must not be empty".to_string()));
    }
    if seeds.len() > 16 {
        return Err(execution(format!(
            "seeds must be at most 16, got {}",
            seeds.len()
        )));
    }
    let max_hops = optional_u8(arguments, "max_hops")?.unwrap_or(1);
    // B8 caps traversal at 3 hops (the engine ceiling is 8); reject anything above.
    if !(1..=3).contains(&max_hops) {
        return Err(execution(format!("max_hops must be 1..=3, got {max_hops}")));
    }
    let filter = TraversalFilter {
        rels: optional_string_array(arguments, "rels")?,
        kinds: optional_entity_kinds(arguments, "kinds")?,
        as_of: optional_as_of_timestamp(arguments, "as_of"),
        direction: optional_direction(arguments)?.unwrap_or(Direction::Both),
        max_hops,
    };
    let graph = require_graph(runtime)?;
    // The engine's expand is UNBOUNDED per hop (a hub node returns its whole
    // neighborhood) — on this attacker-controlled surface the tool walks the
    // graph itself under hard budgets. Exceeding any budget is a tool error
    // telling the caller to narrow the query, never silent truncation.
    let subgraph = block_on(budgeted_expand(graph.as_ref(), &seeds, &filter))?;
    // Provenance is serialized as-is so derived:* / Rule provenance stays VISIBLE.
    let subgraph = serde_json::to_value(&subgraph).map_err(|error| serde_failure(&error))?;
    Ok(structured(subgraph))
}

/// Per-anchor neighbor ceiling for one traverse hop.
const TRAVERSE_NEIGHBORS_PER_NODE: usize = 64;
/// Total node budget for one traverse response.
const TRAVERSE_MAX_NODES: usize = 512;
/// Total edge budget for one traverse response.
const TRAVERSE_MAX_EDGES: usize = 1024;

/// Hop-bounded BFS with hard node/edge budgets (deterministic order, edges
/// deduplicated by identity).
async fn budgeted_expand(
    graph: &dyn tdw_core::GraphEngine,
    seeds: &[String],
    filter: &TraversalFilter,
) -> Result<tdw_core::Subgraph, ToolFailure> {
    let storage = |error: tdw_core::Error| execution(error.to_string());
    let mut nodes: std::collections::BTreeMap<String, tdw_core::GraphNode> =
        std::collections::BTreeMap::new();
    let mut edge_identities: std::collections::BTreeSet<(String, String, String, Option<String>)> =
        std::collections::BTreeSet::new();
    let mut edges: Vec<tdw_core::GraphEdge> = Vec::new();
    let mut visited: std::collections::BTreeSet<String> = seeds.iter().cloned().collect();
    for seed in seeds {
        if let Some(node) = graph.node(seed).await.map_err(storage)? {
            nodes.insert(node.id.clone(), node);
        }
    }
    let hop_filter = TraversalFilter {
        max_hops: 1,
        ..filter.clone()
    };
    let mut frontier: Vec<String> = seeds.to_vec();
    for _ in 0..filter.max_hops {
        let mut next = Vec::new();
        for anchor in &frontier {
            let reached = graph
                .neighbors(anchor, &hop_filter)
                .await
                .map_err(storage)?;
            if reached.len() > TRAVERSE_NEIGHBORS_PER_NODE {
                return Err(execution(format!(
                    "traverse budget exceeded: {anchor:?} has more than \
                     {TRAVERSE_NEIGHBORS_PER_NODE} neighbors under this filter — narrow \
                     rels/kinds or reduce seeds"
                )));
            }
            for (edge, node) in reached {
                let identity = (
                    edge.from.clone(),
                    edge.rel.clone(),
                    edge.to.clone(),
                    edge.valid_from.clone(),
                );
                if edge_identities.insert(identity) {
                    edges.push(edge);
                    if edges.len() > TRAVERSE_MAX_EDGES {
                        return Err(execution(format!(
                            "traverse budget exceeded: more than {TRAVERSE_MAX_EDGES} edges — \
                             narrow rels/kinds or reduce seeds/max_hops"
                        )));
                    }
                }
                if visited.insert(node.id.clone()) {
                    next.push(node.id.clone());
                }
                nodes.entry(node.id.clone()).or_insert(node);
                if nodes.len() > TRAVERSE_MAX_NODES {
                    return Err(execution(format!(
                        "traverse budget exceeded: more than {TRAVERSE_MAX_NODES} nodes — \
                         narrow rels/kinds or reduce seeds/max_hops"
                    )));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    edges.sort_by(|left, right| {
        (&left.from, &left.rel, &left.to, &left.valid_from).cmp(&(
            &right.from,
            &right.rel,
            &right.to,
            &right.valid_from,
        ))
    });
    Ok(tdw_core::Subgraph {
        nodes: nodes.into_values().collect(),
        edges,
    })
}

fn path(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let from = require_str(arguments, "from")?;
    let to = require_str(arguments, "to")?;
    let max_hops = optional_u8(arguments, "max_hops")?.unwrap_or(4);
    if !(1..=8).contains(&max_hops) {
        return Err(execution(format!("max_hops must be 1..=8, got {max_hops}")));
    }
    let filter = TraversalFilter {
        rels: optional_string_array(arguments, "rels")?,
        max_hops,
        ..TraversalFilter::default()
    };
    let graph = require_graph(runtime)?;
    let edges = block_on(graph.shortest_path(from, to, &filter))
        .map_err(|error| execution(error.to_string()))?;
    let edges = serde_json::to_value(&edges).map_err(|error| serde_failure(&error))?;
    Ok(structured(json!({ "path": edges })))
}

fn tags_query(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let tags = runtime
        .tags()
        .ok_or_else(|| execution("knowledge tag engine not attached".to_string()))?;
    let entity_id = optional_str(arguments, "entity_id");
    let tag_id = optional_str(arguments, "tag_id");
    let as_of = optional_str(arguments, "as_of");
    let usage = "tdw.tags.query: provide exactly one of entity_id+as_of (active tags), \
                 tag_id (descendants), or tag_id+as_of (entities with tag)";
    let result = match (entity_id, tag_id, as_of) {
        (Some(entity_id), None, Some(as_of)) => {
            let active = block_on(tags.active_tags(entity_id, as_of))
                .map_err(|error| execution(error.to_string()))?;
            json!({ "active_tags": active })
        }
        (None, Some(tag_id), None) => {
            let descendants =
                block_on(tags.descendants(tag_id)).map_err(|error| execution(error.to_string()))?;
            json!({ "descendants": descendants })
        }
        (None, Some(tag_id), Some(as_of)) => {
            let entities = block_on(tags.entities_with_tag(tag_id, as_of))
                .map_err(|error| execution(error.to_string()))?;
            json!({ "entities_with_tag": entities })
        }
        _ => return Err(execution(usage.to_string())),
    };
    Ok(structured(result))
}

/// The graph engine, or a tool error naming the missing attachment.
fn require_graph(runtime: &KnowledgeRuntime) -> Result<&Arc<dyn GraphEngine>, ToolFailure> {
    runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))
}

/// Build the graph-expansion settings from the optional `expand` object.
fn optional_expansion(
    arguments: &Map<String, Value>,
) -> Result<Option<GraphExpansion>, ToolFailure> {
    let Some(value) = arguments.get("expand") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| execution("expand must be an object".to_string()))?;
    let k_hop = optional_u8(object, "k_hop")?
        .ok_or_else(|| execution("expand.k_hop is required".to_string()))?;
    let per_hit_limit = optional_usize(object, "per_hit_limit")?
        .ok_or_else(|| execution("expand.per_hit_limit is required".to_string()))?;
    let decay = object
        .get("decay")
        .ok_or_else(|| execution("expand.decay is required".to_string()))?
        .as_f64()
        .ok_or_else(|| execution("expand.decay must be a number".to_string()))?;
    let edge_types = optional_string_array(object, "edge_types")?.unwrap_or_default();
    Ok(Some(GraphExpansion {
        k_hop,
        edge_types,
        per_hit_limit,
        decay,
    }))
}

/// Parse `entity_kinds`/`kinds` into taxonomy kinds; an unknown kind is a tool error.
fn optional_entity_kinds(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<Vec<EntityKind>>, ToolFailure> {
    let Some(values) = optional_string_array(arguments, name)? else {
        return Ok(None);
    };
    let mut kinds = Vec::with_capacity(values.len());
    for value in values {
        let kind: EntityKind = serde_json::from_value(Value::String(value.clone()))
            .map_err(|_| execution(format!("unknown entity kind: {value}")))?;
        kinds.push(kind);
    }
    Ok(Some(kinds))
}

/// Normalize an optional `as_of`: a `YYYY-MM-DD` date maps to the graph's
/// timestamp form; a longer value is passed through (the engine validates it).
fn optional_as_of_timestamp(arguments: &Map<String, Value>, name: &str) -> Option<String> {
    let value = optional_str(arguments, name)?;
    if is_date(value) {
        Some(date_to_timestamp(value))
    } else {
        Some(value.to_string())
    }
}

fn optional_direction(arguments: &Map<String, Value>) -> Result<Option<Direction>, ToolFailure> {
    match optional_str(arguments, "direction") {
        None => Ok(None),
        Some("out") => Ok(Some(Direction::Out)),
        Some("in") => Ok(Some(Direction::In)),
        Some("both") => Ok(Some(Direction::Both)),
        Some(other) => Err(execution(format!(
            "direction must be one of out / in / both, got {other:?}"
        ))),
    }
}

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn require_string_array(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Vec<String>, ToolFailure> {
    optional_string_array(arguments, name)?
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_string_array(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<Vec<String>>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| execution(format!("{name} must be an array of strings")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(execution(format!("{name} must be an array of strings"))),
    }
}

fn optional_usize(
    arguments: &Map<String, Value>,
    name: &str,
) -> Result<Option<usize>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| execution(format!("{name} must be a non-negative integer"))),
    }
}

fn optional_u8(arguments: &Map<String, Value>, name: &str) -> Result<Option<u8>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| u8::try_from(number).ok())
            .map(Some)
            .ok_or_else(|| execution(format!("{name} must be an integer in 0..=255"))),
    }
}

/// `YYYY-MM-DD` shape check (mirrors the tag/retrieve date grammar).
fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
}

/// Drive an async knowledge call from sync `execute_tool`.
///
/// When an ambient tokio runtime exists (the embedded `McpOnly` surface runs
/// the stdio loop inside `block_in_place` on a multi-thread runtime),
/// building a fresh runtime here would PANIC ("Cannot start a runtime from
/// within a runtime") — a remote crash per tool call (B8 security review).
/// Reuse the ambient handle in that case; otherwise build the tiny per-call
/// runtime the ops surface uses.
pub fn block_on<F>(future: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    use tokio::runtime::{Builder, Handle, RuntimeFlavor};
    let fresh = || {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build")
    };
    match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        // Ambient CURRENT-THREAD runtime (e.g. a plain #[tokio::test]):
        // block_in_place is unsupported there, so run the self-contained
        // engine future on a scoped thread with its own tiny runtime.
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| fresh().block_on(future))
                .join()
                .expect("bridge thread must not panic")
        }),
        Err(_) => fresh().block_on(future),
    }
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

fn serde_failure(error: &serde_json::Error) -> ToolFailure {
    ToolFailure::Execution(error.to_string())
}
