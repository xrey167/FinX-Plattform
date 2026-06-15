//! Read-only knowledge graph-visualization handler for the Workspace bridge
//! (K-M6).
//!
//! This is the *caller-side* of the `tdw_app_server::KnowledgeGraphHandler`
//! seam (mirroring [`crate::rest_handler::KnowledgeStatusAdapter`]):
//! `tdw-app-server` defines the trait but does not depend on `tdw-knowledge`;
//! this crate implements it over a shared [`KnowledgeRuntime`] and hands an
//! `Arc<dyn KnowledgeGraphHandler>` to `serve_workspace_http_with_graph`.
//!
//! It resolves an **ego-graph** — a root node plus its bounded neighborhood to a
//! capped hop depth — or, when the root names a taxonomy tag, the same bounded
//! neighborhood over the tag subsumption edges. The result is rendered as
//! Markdown the `markdown` Workspace widget displays. Strictly read-only: it
//! calls only `GraphEngine::node` and `GraphEngine::neighbors`; it never writes.
//!
//! # Safety properties (K-M6 acceptance)
//!
//! - **Bounded fan-out**: depth is clamped to [`MAX_GRAPH_DEPTH`] (well under the
//!   engine's `MAX_HOPS` ceiling) AND the traversal runs through
//!   [`bounded_ego_graph`], a handler-local BFS with a HARD total budget — it
//!   stops collecting at [`MAX_GRAPH_NODES`] / [`MAX_GRAPH_EDGES`] and never
//!   enqueues a frontier node past the cap, so a hub node can never grow the
//!   working set or the payload past the budget. (The shared, unbounded
//!   `GraphEngine::expand` is deliberately NOT used here — it would materialize a
//!   hub node's entire neighborhood before any truncation could apply.)
//! - **Leakage-safe `as_of`**: an `as_of` argument is threaded verbatim into the
//!   temporal [`TraversalFilter`]; absence means "all time" (the documented graph
//!   contract). The handler never substitutes a future timestamp, so there is no
//!   lookahead.
//! - **Injected now**: where a wall-clock instant is needed (the payload's
//!   `generated_at` stamp), it comes from an injected clock closure, never a
//!   direct `SystemTime::now()` call — deterministic under test.
//! - **Honest empty**: an absent / blank root, or a root that does not resolve to
//!   a node, yields an explicit empty graph (`node_count = 0`) with a note — never
//!   a fabricated node or edge.

#![cfg(feature = "workspace-route")]

use std::collections::{BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};
use tdw_app_server::KnowledgeGraphHandler;
use tdw_core::{Direction, GraphEdge, GraphEngine, GraphNode, Subgraph, TraversalFilter};
use tdw_knowledge::runtime::KnowledgeRuntime;

/// Hop-budget ceiling for the ego-graph. Deliberately small (well under the
/// engine's `MAX_HOPS = 8`) so a first render on a hub node stays cheap.
pub const MAX_GRAPH_DEPTH: u8 = 3;

/// Maximum nodes rendered in one response. A hub node's neighborhood is
/// truncated to this many (with an honest note) rather than returned unbounded.
pub const MAX_GRAPH_NODES: usize = 200;

/// Maximum edges rendered in one response (same truncation discipline).
pub const MAX_GRAPH_EDGES: usize = 400;

/// An injected clock yielding the current instant as an RFC 3339 string. Holds
/// the wall-clock dependency behind a closure so the payload's `generated_at`
/// stamp is deterministic under test and never a direct `SystemTime::now()`
/// call on the async serving path.
pub type GraphClock = Arc<dyn Fn() -> String + Send + Sync>;

/// Implements [`KnowledgeGraphHandler`] over a shared [`KnowledgeRuntime`].
///
/// Construct via [`KnowledgeGraphAdapter::new`] (system clock) or
/// [`KnowledgeGraphAdapter::with_clock`] (injected clock for tests), then
/// `into_handler()` for `serve_workspace_http_with_graph`.
pub struct KnowledgeGraphAdapter {
    runtime: Arc<KnowledgeRuntime>,
    clock: GraphClock,
}

impl KnowledgeGraphAdapter {
    /// Wrap a [`KnowledgeRuntime`] with the system clock for the `generated_at`
    /// stamp.
    #[must_use]
    pub fn new(runtime: Arc<KnowledgeRuntime>) -> Self {
        Self {
            runtime,
            clock: Arc::new(system_now_stamp),
        }
    }

    /// Wrap a [`KnowledgeRuntime`] with an injected clock (deterministic tests).
    #[must_use]
    pub fn with_clock(runtime: Arc<KnowledgeRuntime>, clock: GraphClock) -> Self {
        Self { runtime, clock }
    }

    /// Build an `Arc<dyn KnowledgeGraphHandler>` for
    /// `tdw_app_server::serve_workspace_http_with_graph`.
    #[must_use]
    pub fn into_handler(self) -> Arc<dyn KnowledgeGraphHandler> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl KnowledgeGraphHandler for KnowledgeGraphAdapter {
    async fn graph_widget_data(&self, params: &Value) -> Result<Value, String> {
        let request = GraphRequest::parse(params)?;
        let generated_at = (self.clock)();

        // No graph engine attached → an honest empty graph (not an error): the
        // widget renders "knowledge graph unavailable" rather than fabricating.
        let Some(graph) = self.runtime.graph() else {
            return Ok(empty_payload(
                &request,
                &generated_at,
                "no graph engine attached to this runtime",
            ));
        };

        // Honest empty for an absent/blank root: nothing to anchor an ego-graph.
        let Some(root) = request.root.as_deref().filter(|r| !r.trim().is_empty()) else {
            return Ok(empty_payload(
                &request,
                &generated_at,
                "no root node supplied",
            ));
        };

        // Honest empty when the root does not resolve to a node.
        match graph.node(root).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(empty_payload(
                    &request,
                    &generated_at,
                    "root node not found in the graph",
                ));
            }
            Err(error) => return Err(format!("graph node lookup failed: {error}")),
        }

        let filter = request.traversal_filter();
        // Bounded fan-out: rather than the shared, unbounded `GraphEngine::expand`
        // (which materializes EVERY neighbor of every frontier node before
        // returning — a hub node would explode), drive the BFS here with a hard
        // total budget. We collect at most `MAX_GRAPH_NODES` nodes /
        // `MAX_GRAPH_EDGES` edges and stop enqueuing the moment the budget is hit,
        // so the retained set — and the frontier that produces it — can never grow
        // past the cap regardless of node degree.
        let (subgraph, fetch_truncated) = bounded_ego_graph(graph.as_ref(), root, &filter)
            .await
            .map_err(|error| format!("ego-graph expansion failed: {error}"))?;

        Ok(render_payload(
            &request,
            &generated_at,
            &subgraph,
            fetch_truncated,
        ))
    }
}

/// The system-clock informational stamp (custom `@<epoch-seconds>s` form, NOT
/// RFC 3339). Isolated so it is the ONLY direct wall-clock read; the serving
/// path always goes through the injected closure.
fn system_now_stamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Minimal stamp from epoch seconds; this `generated_at` field is purely
    // informational (the graph's own temporal compares use `as_of`, not this).
    format!("@{}s", now.as_secs())
}

/// Resolve the root's ego-graph with a HARD total fan-out budget, returning the
/// bounded [`Subgraph`] and whether the budget clipped it.
///
/// Bounds the traversal at the handler rather than leaning on the shared,
/// unbounded [`GraphEngine::expand`] (which BFS-materializes every neighbor of
/// every frontier node before returning — a hub node would explode the payload
/// and the working set). Here the BFS stops the instant the retained node/edge
/// totals reach [`MAX_GRAPH_NODES`] / [`MAX_GRAPH_EDGES`], and never enqueues a
/// frontier node past the node cap, so neither the result nor the frontier can
/// grow past the budget regardless of any node's degree. `filter.max_hops` is
/// already clamped to [`MAX_GRAPH_DEPTH`] upstream.
///
/// Read-only: it calls only [`GraphEngine::neighbors`]; it never writes.
async fn bounded_ego_graph(
    graph: &dyn GraphEngine,
    root: &str,
    filter: &TraversalFilter,
) -> tdw_core::Result<(Subgraph, bool)> {
    let mut node_ids: BTreeSet<String> = BTreeSet::new();
    let mut edge_keys: BTreeSet<(String, String, String, Option<String>)> = BTreeSet::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut frontier: VecDeque<(String, u8)> = VecDeque::new();
    let mut truncated = false;

    node_ids.insert(root.to_string());
    frontier.push_back((root.to_string(), 0));

    while let Some((anchor, depth)) = frontier.pop_front() {
        if depth >= filter.max_hops {
            continue;
        }
        for (edge, node) in graph.neighbors(&anchor, filter).await? {
            // Edge budget: stop collecting once full (deterministic order from
            // the engine means the retained slice is stable, not arbitrary).
            if edges.len() < MAX_GRAPH_EDGES {
                let key = (
                    edge.from.clone(),
                    edge.to.clone(),
                    edge.rel.clone(),
                    edge.valid_from.clone(),
                );
                if edge_keys.insert(key) {
                    edges.push(edge);
                }
            } else {
                truncated = true;
            }

            // Node budget: never admit (or enqueue) a node past the cap.
            if node_ids.contains(&node.id) {
                continue;
            }
            if node_ids.len() >= MAX_GRAPH_NODES {
                truncated = true;
                continue;
            }
            node_ids.insert(node.id.clone());
            frontier.push_back((node.id, depth + 1));
        }
    }

    // Materialize the retained node set (root included when it resolves).
    let mut nodes: Vec<GraphNode> = Vec::with_capacity(node_ids.len());
    for id in &node_ids {
        if let Some(node) = graph.node(id).await? {
            nodes.push(node);
        }
    }
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| {
        (&left.from, &left.rel, &left.to, &left.valid_from).cmp(&(
            &right.from,
            &right.rel,
            &right.to,
            &right.valid_from,
        ))
    });

    Ok((Subgraph { nodes, edges }, truncated))
}

/// A parsed, validated graph-visualization request.
#[derive(Debug)]
struct GraphRequest {
    /// The root node id (entity or tag) to anchor the ego-graph on.
    root: Option<String>,
    /// Hop budget, clamped to `1..=MAX_GRAPH_DEPTH`.
    depth: u8,
    /// Leakage-safe point-in-time (RFC 3339); `None` = no temporal filter.
    as_of: Option<String>,
    /// Traversal direction (defaults to both, so an ego-graph shows in+out).
    direction: Direction,
    /// Restrict to these relationship types; `None` = all.
    rels: Option<Vec<String>>,
}

impl GraphRequest {
    /// Parse the widget query params into a validated request.
    ///
    /// Tolerant of absent fields (honest-empty handles a missing root). The only
    /// hard error is a malformed `depth` (non-numeric / zero), which is a caller
    /// mistake worth surfacing as 400.
    fn parse(params: &Value) -> Result<Self, String> {
        let root = string_param(params, "root");

        let depth = match params.get("depth") {
            None | Some(Value::Null) => GraphDefault::DEPTH,
            Some(Value::Number(number)) => {
                let raw = number
                    .as_u64()
                    .ok_or_else(|| "depth must be a positive integer".to_string())?;
                clamp_depth(raw)
            }
            Some(Value::String(text)) if text.trim().is_empty() => GraphDefault::DEPTH,
            Some(Value::String(text)) => {
                let raw: u64 = text
                    .trim()
                    .parse()
                    .map_err(|_| "depth must be a positive integer".to_string())?;
                clamp_depth(raw)
            }
            Some(_) => return Err("depth must be a positive integer".to_string()),
        };

        let as_of = string_param(params, "as_of");
        let direction = match string_param(params, "direction").as_deref() {
            Some("out") => Direction::Out,
            Some("in") => Direction::In,
            // Default and explicit "both": an ego-graph shows both directions.
            _ => Direction::Both,
        };
        let rels = string_param(params, "rels").map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        });

        Ok(Self {
            root,
            depth,
            as_of,
            direction,
            rels: rels.filter(|list| !list.is_empty()),
        })
    }

    /// The bounded [`TraversalFilter`] for the ego-graph expansion. `max_hops`
    /// is already clamped; `as_of` is threaded verbatim (leakage-safe).
    fn traversal_filter(&self) -> TraversalFilter {
        TraversalFilter {
            rels: self.rels.clone(),
            kinds: None,
            as_of: self.as_of.clone(),
            direction: self.direction,
            max_hops: self.depth,
        }
    }
}

/// Default constants split out so `parse` reads cleanly.
struct GraphDefault;
impl GraphDefault {
    const DEPTH: u8 = 1;
}

/// Clamp a requested depth into `1..=MAX_GRAPH_DEPTH` (bounded fan-out).
fn clamp_depth(raw: u64) -> u8 {
    let bounded = raw.clamp(1, u64::from(MAX_GRAPH_DEPTH));
    u8::try_from(bounded).unwrap_or(MAX_GRAPH_DEPTH)
}

/// Read an optional string query param, trimming and dropping empties.
fn string_param(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Build the honest-empty payload (no fabricated nodes/edges).
fn empty_payload(request: &GraphRequest, generated_at: &str, reason: &str) -> Value {
    let markdown = format!(
        "## Knowledge Graph\n\n_Empty graph: {reason}._\n\nRoot: `{}`\n",
        request.root.as_deref().unwrap_or("(none)")
    );
    json!({
        "results": markdown,
        "graph": {
            "root": request.root,
            "depth": request.depth,
            "as_of": request.as_of,
            "node_count": 0,
            "edge_count": 0,
            "nodes": [],
            "edges": [],
            "note": reason,
            "generated_at": generated_at,
        }
    })
}

/// Render the resolved ego-graph as Markdown plus a structured `graph` block.
///
/// `fetch_truncated` is set by [`bounded_ego_graph`] when the hard fan-out
/// budget clipped the traversal. The `.take()` caps below are a defensive
/// belt-and-suspenders — the incoming `subgraph` is already budget-bounded — so
/// the rendered lists can never exceed [`MAX_GRAPH_NODES`] / [`MAX_GRAPH_EDGES`].
fn render_payload(
    request: &GraphRequest,
    generated_at: &str,
    subgraph: &Subgraph,
    fetch_truncated: bool,
) -> Value {
    let nodes: Vec<&GraphNode> = subgraph.nodes.iter().take(MAX_GRAPH_NODES).collect();
    let edges: Vec<&GraphEdge> = subgraph.edges.iter().take(MAX_GRAPH_EDGES).collect();

    let note = if fetch_truncated {
        Some(format!(
            "neighborhood truncated at the fan-out budget \
             ({MAX_GRAPH_NODES} nodes / {MAX_GRAPH_EDGES} edges); \
             narrow the root, depth, or rels to see more"
        ))
    } else {
        None
    };

    let markdown = render_markdown(request, &nodes, &edges, note.as_deref());

    let node_values: Vec<Value> = nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id,
                "kind": node.kind,
                "label": node.label,
            })
        })
        .collect();
    let edge_values: Vec<Value> = edges
        .iter()
        .map(|edge| {
            json!({
                "from": edge.from,
                "to": edge.to,
                "rel": edge.rel,
            })
        })
        .collect();

    json!({
        "results": markdown,
        "graph": {
            "root": request.root,
            "depth": request.depth,
            "as_of": request.as_of,
            "node_count": node_values.len(),
            "edge_count": edge_values.len(),
            "nodes": node_values,
            "edges": edge_values,
            "note": note,
            "generated_at": generated_at,
        }
    })
}

/// Render a compact Markdown ego-graph view: the node table then the edge list.
fn render_markdown(
    request: &GraphRequest,
    nodes: &[&GraphNode],
    edges: &[&GraphEdge],
    note: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str("## Knowledge Graph\n\n");
    let _ = writeln!(
        out,
        "Root: `{}` · depth {} · {} nodes · {} edges",
        request.root.as_deref().unwrap_or("(none)"),
        request.depth,
        nodes.len(),
        edges.len()
    );
    if let Some(as_of) = &request.as_of {
        let _ = writeln!(out, "As of: `{as_of}`");
    }
    if let Some(note) = note {
        let _ = write!(out, "\n> {note}\n");
    }

    out.push_str("\n### Nodes\n\n| id | kind | label |\n|----|------|-------|\n");
    for node in nodes {
        let _ = writeln!(
            out,
            "| `{}` | {:?} | {} |",
            escape_cell(&node.id),
            node.kind,
            escape_cell(&node.label)
        );
    }

    out.push_str("\n### Edges\n\n| from | rel | to |\n|------|-----|----|\n");
    for edge in edges {
        let _ = writeln!(
            out,
            "| `{}` | {} | `{}` |",
            escape_cell(&edge.from),
            escape_cell(&edge.rel),
            escape_cell(&edge.to)
        );
    }
    out
}

/// Escape a Markdown table cell: pipes, carriage returns, and newlines would all
/// break the table layout. Node/edge ids are graph-controlled but still routed
/// through this so a stray separator in an id cannot corrupt the rendered table.
fn escape_cell(value: &str) -> String {
    value
        .replace('|', "\\|")
        .replace('\r', "")
        .replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(json_value: Value) -> Value {
        json_value
    }

    #[test]
    fn depth_is_clamped_to_the_ceiling() {
        assert_eq!(clamp_depth(0), 1);
        assert_eq!(clamp_depth(1), 1);
        assert_eq!(clamp_depth(99), MAX_GRAPH_DEPTH);
    }

    #[tokio::test]
    async fn bounded_ego_graph_clips_a_hub_node_to_the_budget() {
        use serde_json::json;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;
        use tdw_taxonomy::EntityKind;

        // A hub with far more neighbors than the node budget: at depth 1 the
        // unbounded `expand` would materialize all of them. The handler-local
        // BFS must hard-cap the retained set instead.
        let hub = "instrument:HUB";
        let neighbor_count = MAX_GRAPH_NODES + 50;
        let engine = InMemoryGraphEngine::default();
        let mut nodes = vec![GraphNode {
            id: hub.to_string(),
            kind: EntityKind::Instrument,
            label: "Hub".to_string(),
            aliases: Vec::new(),
            props: json!({}),
            valid_from: None,
            valid_to: None,
        }];
        let mut edges = Vec::new();
        for index in 0..neighbor_count {
            let id = format!("instrument:N{index}");
            nodes.push(GraphNode {
                id: id.clone(),
                kind: EntityKind::Instrument,
                label: format!("Node {index}"),
                aliases: Vec::new(),
                props: json!({}),
                valid_from: None,
                valid_to: None,
            });
            edges.push(GraphEdge {
                from: hub.to_string(),
                to: id,
                rel: "related_to".to_string(),
                props: json!({}),
                provenance: Provenance::Ingest {
                    source: "test:hub".to_string(),
                },
                valid_from: None,
                valid_to: None,
            });
        }
        engine.upsert_nodes(nodes).await.expect("seed nodes");
        engine.upsert_edges(edges).await.expect("seed edges");

        let filter = TraversalFilter {
            rels: None,
            kinds: None,
            as_of: None,
            direction: Direction::Both,
            max_hops: 1,
        };
        let (subgraph, truncated) = bounded_ego_graph(&engine, hub, &filter)
            .await
            .expect("bounded expand");

        assert!(truncated, "the budget must report the hub was clipped");
        assert!(
            subgraph.nodes.len() <= MAX_GRAPH_NODES,
            "retained nodes ({}) never exceed the budget {MAX_GRAPH_NODES}",
            subgraph.nodes.len()
        );
        assert!(
            subgraph.edges.len() <= MAX_GRAPH_EDGES,
            "retained edges ({}) never exceed the budget {MAX_GRAPH_EDGES}",
            subgraph.edges.len()
        );
    }

    #[test]
    fn parse_defaults_when_fields_absent() {
        let request = GraphRequest::parse(&params(json!({}))).expect("parse empty");
        assert!(request.root.is_none());
        assert_eq!(request.depth, GraphDefault::DEPTH);
        assert!(request.as_of.is_none());
        assert!(matches!(request.direction, Direction::Both));
        assert!(request.rels.is_none());
    }

    #[test]
    fn parse_threads_as_of_verbatim_no_lookahead() {
        let request = GraphRequest::parse(&params(json!({
            "root": "instrument:AAPL",
            "as_of": "2024-01-01T00:00:00Z",
            "depth": 2
        })))
        .expect("parse");
        let filter = request.traversal_filter();
        assert_eq!(filter.as_of.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(filter.max_hops, 2);
    }

    #[test]
    fn parse_clamps_oversized_depth() {
        let request =
            GraphRequest::parse(&params(json!({ "root": "x", "depth": 50 }))).expect("parse");
        assert_eq!(request.depth, MAX_GRAPH_DEPTH);
    }

    #[test]
    fn parse_rejects_non_numeric_depth() {
        let error = GraphRequest::parse(&params(json!({ "depth": "soon" })))
            .expect_err("non-numeric depth is rejected");
        assert!(error.contains("positive integer"), "got: {error}");
    }

    #[test]
    fn empty_payload_is_honest_not_fabricated() {
        let request = GraphRequest::parse(&params(json!({}))).expect("parse");
        let payload = empty_payload(&request, "@0s", "no root node supplied");
        assert_eq!(payload["graph"]["node_count"], 0);
        assert_eq!(payload["graph"]["edge_count"], 0);
        assert!(
            payload["graph"]["nodes"]
                .as_array()
                .expect("nodes array")
                .is_empty(),
            "no fabricated nodes"
        );
        assert!(
            payload["results"]
                .as_str()
                .expect("results markdown")
                .contains("Empty graph"),
            "markdown states the graph is empty"
        );
    }

    #[test]
    fn render_emits_an_honest_note_when_the_budget_clipped_the_fetch() {
        use tdw_taxonomy::EntityKind;

        // The bounded BFS already clipped to the budget; render reflects that
        // with an honest note (it does not re-derive truncation from the size).
        let nodes: Vec<GraphNode> = (0..MAX_GRAPH_NODES)
            .map(|index| GraphNode {
                id: format!("instrument:N{index}"),
                kind: EntityKind::Instrument,
                label: format!("Node {index}"),
                aliases: Vec::new(),
                props: json!({}),
                valid_from: None,
                valid_to: None,
            })
            .collect();
        let subgraph = Subgraph {
            nodes,
            edges: Vec::new(),
        };
        let request =
            GraphRequest::parse(&params(json!({ "root": "instrument:N0" }))).expect("parse");
        let payload = render_payload(&request, "@0s", &subgraph, /* fetch_truncated */ true);
        assert_eq!(payload["graph"]["node_count"], MAX_GRAPH_NODES);
        assert!(
            payload["graph"]["note"]
                .as_str()
                .expect("truncation note")
                .contains("truncated"),
            "honest truncation note present"
        );
    }

    #[test]
    fn render_omits_the_note_when_within_budget() {
        let subgraph = Subgraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let request = GraphRequest::parse(&params(json!({ "root": "x" }))).expect("parse");
        let payload = render_payload(&request, "@0s", &subgraph, /* fetch_truncated */ false);
        assert!(
            payload["graph"]["note"].is_null(),
            "no truncation note when the budget did not clip"
        );
    }
}
