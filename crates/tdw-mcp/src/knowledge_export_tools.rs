//! `tdw.kg.export` — portable research-trail export (knowledge-system K-X10).
//!
//! Assembles a self-contained research trail rooted at one thesis, finding, or
//! entity and serializes it to BOTH structured JSON and human-readable Markdown.
//! The trail bundles, in evidence order:
//!
//! * **Root** — the thesis / finding / entity node, with its kind, label, and
//!   stored props (incl. the pinned `evidence` snippet for findings).
//! * **Findings + linked evidence** — for an entity/thesis root, the findings
//!   that `mention` / `support` / `contradict` it; for a finding root, its own
//!   evidence pin and outbound links.
//! * **Thesis + accumulated evidence** — inbound `supports` / `contradicts`
//!   edges (leakage-safe: only edges active at `as_of` are counted).
//! * **Cited answers** — the document nodes (`described_by` targets) that back
//!   each finding, resolved to their real graph ids / paths so a citation never
//!   dangles.
//! * **Provenance** — the structured provenance of every edge in the trail,
//!   surfaced as a one-liner per the [`crate::knowledge_explain_tools`] vocabulary.
//!
//! # Honesty contract
//!
//! * A node that the root references but that cannot be fetched (missing or
//!   inaccessible) is recorded **explicitly** as `"[evidence unavailable: id]"`
//!   — never silently dropped. The export still succeeds; the gap is visible.
//! * Citations resolve to real graph ids — they are read directly from edge
//!   targets, never invented.
//! * Counts in the response match what the trail actually carries.
//!
//! # Leakage-safe `as_of`
//!
//! Evidence edges are filtered with [`tdw_core::active_at`] — the same predicate
//! the graph engine, retriever, and `tdw.kg.thesis_health` use — so an export at
//! `as_of = 2024-01-01` can never surface evidence that became valid after that
//! date. No lookahead, no independent re-implementation of temporal filtering.
//!
//! # Determinism
//!
//! Both serializations are byte-stable for the same trail:
//! * JSON is built from sorted collections ([`std::collections::BTreeMap`] /
//!   pre-sorted [`Vec`]s) so key order and array order are fixed.
//! * Markdown walks the same pre-sorted structures.
//! * The only time-varying inputs — `as_of` and the caller-injected
//!   `generated_at` — are explicit arguments. Nothing in this module reads the
//!   wall clock (the `now` fallback for `as_of` is injected by the MCP layer).
//!
//! Re-exporting the same trail twice produces byte-identical JSON and Markdown.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use tdw_core::{Direction, GraphEdge, GraphNode, TraversalFilter, active_at};
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_taxonomy::EntityKind;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool};

// ── Tool name ───────────────────────────────────────────────────────────────

/// The tool name owned by this module.
pub const TOOL_NAME: &str = "tdw.kg.export";

/// Whether `name` is the export tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum inbound evidence edges scanned for a thesis/entity root. Bounds the
/// read regardless of graph size; an honest `evidence_truncated` flag fires
/// when exceeded.
const MAX_EVIDENCE_EDGES: usize = 1_024;

/// Maximum outbound edges (links / `described_by` / `mentions`) scanned per finding.
const MAX_OUTBOUND_EDGES: usize = 512;

/// The evidence relation tokens accumulated onto a thesis/entity root.
const EVIDENCE_RELS: &[&str] = &["supports", "contradicts", "relates_to", "mentions"];

// ── Descriptor ────────────────────────────────────────────────────────────────

/// Descriptor for `tdw.kg.export`. Read-only and idempotent: the same root +
/// `as_of` + `generated_at` always produces the same artifact.
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool(
        TOOL_NAME,
        "Export Research Trail",
        "Export a portable, self-contained research trail rooted at one thesis, finding, or \
         entity — findings, linked evidence, accumulated supports/contradicts, cited answers, \
         and full provenance — serialized to BOTH structured JSON and human-readable Markdown. \
         \n\n\
         Read-only: assembles the trail from the graph at read time and writes nothing back. \
         \n\
         Honest + complete: a referenced node that cannot be fetched is recorded explicitly as \
         \"[evidence unavailable: id]\", never silently dropped; citations resolve to real graph \
         ids; the reported counts match the trail. \
         \n\
         Leakage-safe as_of: only evidence active at as_of is included (no lookahead) — the \
         same temporal predicate the graph engine and thesis_health use. \
         \n\
         Deterministic: the serialization uses sorted keys and stable ordering, so the same \
         trail exports byte-identically across runs. \
         \n\
         The generated_at stamp is caller-injected (no wall-clock read inside the library) so \
         exports are reproducible. Requires the graph engine. \
         \n\
         Provide exactly one of thesis_id, finding_id, or entity_id as the trail root.",
        json!({
            "type": "object",
            "properties": {
                "thesis_id": {
                    "type": "string",
                    "description": "Root the trail at this thesis (finding:… node with kind_hint=thesis)."
                },
                "finding_id": {
                    "type": "string",
                    "description": "Root the trail at this finding node."
                },
                "entity_id": {
                    "type": "string",
                    "description": "Root the trail at any graph entity (e.g. instrument:AAPL)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Temporal point of view (YYYY-MM-DD). Only evidence active at \
                                    this date is included (leakage-safe). Defaults to today."
                },
                "generated_at": {
                    "type": "string",
                    "description": "Caller-injected generation timestamp stamped onto the artifact \
                                    (the library never reads the wall clock). Free-form; echoed \
                                    verbatim into both JSON and Markdown."
                }
            },
            "additionalProperties": false
        }),
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Execute `tdw.kg.export`. Every failure is a tool error, never a protocol error.
///
/// `now` is the current instant as `YYYY-MM-DD`, injected by the MCP layer — the
/// `as_of` fallback when the caller omits it. Nothing here reads the clock.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing graph engine, malformed
/// input (zero or multiple roots, bad `as_of`), a missing root node, or an
/// engine failure — never [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;

    // Exactly one root must be specified.
    let thesis_id = optional_str(arguments, "thesis_id");
    let finding_id = optional_str(arguments, "finding_id");
    let entity_id = optional_str(arguments, "entity_id");
    let roots_given = [
        thesis_id.is_some(),
        finding_id.is_some(),
        entity_id.is_some(),
    ]
    .into_iter()
    .filter(|&b| b)
    .count();
    if roots_given != 1 {
        return Err(execution(
            "tdw.kg.export: provide exactly one of thesis_id, finding_id, or entity_id".to_string(),
        ));
    }
    let root_id = thesis_id.or(finding_id).or(entity_id).unwrap_or_default();
    let root_kind_arg = if thesis_id.is_some() {
        "thesis"
    } else if finding_id.is_some() {
        "finding"
    } else {
        "entity"
    };

    let as_of = optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;
    let as_of_ts = tdw_tags::date_to_timestamp(&as_of);

    // generated_at is caller-injected; absent → an explicit sentinel so the
    // artifact always carries a (truthful) stamp rather than a silent gap.
    let generated_at = optional_str(arguments, "generated_at")
        .map_or_else(|| "(generation time not provided)".to_string(), ToString::to_string);

    let trail = block_on(assemble_trail(graph, root_id, root_kind_arg, &as_of_ts))?;

    let json_artifact = trail.to_json(&as_of, &generated_at);
    let markdown = trail.to_markdown(&as_of, &generated_at);

    Ok(structured(json!({
        "root_id": root_id,
        "root_kind": root_kind_arg,
        "as_of": as_of,
        "generated_at": generated_at,
        "findings_count": trail.findings.len(),
        "evidence_count": trail.evidence_edges.len(),
        "citations_count": trail.citations.len(),
        "unavailable_count": trail.unavailable.len(),
        "evidence_truncated": trail.evidence_truncated,
        "json": json_artifact,
        "markdown": markdown,
    })))
}

// ── Trail model ────────────────────────────────────────────────────────────────

/// A node as carried in the trail (only the stable fields we serialize).
struct TrailNode {
    id: String,
    kind: EntityKind,
    label: String,
    valid_from: Option<String>,
    valid_to: Option<String>,
    props: Value,
}

impl TrailNode {
    fn from_graph(node: &GraphNode) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind,
            label: node.label.clone(),
            valid_from: node.valid_from.clone(),
            valid_to: node.valid_to.clone(),
            props: node.props.clone(),
        }
    }
}

/// One evidence edge in the trail (inbound to the root, or the root's own link).
struct TrailEdge {
    from: String,
    rel: String,
    to: String,
    valid_from: Option<String>,
    valid_to: Option<String>,
    provenance: String,
    note: Option<String>,
}

/// One resolved citation: a document/evidence node a finding draws from.
struct Citation {
    /// The id of the node whose evidence is cited (the finding).
    finding_id: String,
    /// The cited document id / path (real graph id — never invented).
    document_id: String,
    /// Optional pinned snippet text (from the finding's `evidence` prop).
    snippet: Option<String>,
    /// Optional source url (from the finding's `evidence` prop).
    source_url: Option<String>,
}

/// The assembled research trail. All collections are pre-sorted so both
/// serializations are deterministic.
struct Trail {
    root: TrailNode,
    /// Finding/thesis nodes in the trail, sorted by id.
    findings: Vec<TrailNode>,
    /// Evidence edges, sorted by `(from, rel, to, valid_from)`.
    evidence_edges: Vec<TrailEdge>,
    /// Resolved citations, sorted by `(finding_id, document_id)`.
    citations: Vec<Citation>,
    /// Ids the root referenced but that could not be fetched, sorted+deduped.
    unavailable: Vec<String>,
    /// True when the inbound evidence scan hit [`MAX_EVIDENCE_EDGES`].
    evidence_truncated: bool,
}

impl Trail {
    /// Deterministic JSON serialization.
    ///
    /// Built from `serde_json::Value` with object keys assembled in a fixed
    /// order and arrays pre-sorted. `serde_json::Map` preserves insertion order
    /// here because we insert keys in a stable sequence; nested objects use the
    /// same discipline. The result is byte-stable for the same trail.
    fn to_json(&self, as_of: &str, generated_at: &str) -> Value {
        let findings: Vec<Value> = self.findings.iter().map(node_to_json).collect();
        let evidence: Vec<Value> = self.evidence_edges.iter().map(edge_to_json).collect();
        let citations: Vec<Value> = self.citations.iter().map(citation_to_json).collect();
        let unavailable: Vec<Value> = self
            .unavailable
            .iter()
            .map(|id| Value::String(format!("[evidence unavailable: {id}]")))
            .collect();

        json!({
            "schema": "tdw.kg.export/v1",
            "as_of": as_of,
            "generated_at": generated_at,
            "root": node_to_json(&self.root),
            "findings": findings,
            "evidence": evidence,
            "citations": citations,
            "provenance_index": self.provenance_index(),
            "unavailable": unavailable,
            "counts": {
                "citations": self.citations.len(),
                "evidence": self.evidence_edges.len(),
                "findings": self.findings.len(),
                "unavailable": self.unavailable.len(),
            },
            "evidence_truncated": self.evidence_truncated,
        })
    }

    /// A sorted provenance index: `{ "agent:user:x": N, ... }` counting how many
    /// trail edges carry each provenance one-liner. Backed by [`BTreeMap`] so the
    /// key order is deterministic.
    fn provenance_index(&self) -> Value {
        let mut counts: BTreeMap<String, u64> = BTreeMap::new();
        for edge in &self.evidence_edges {
            *counts.entry(edge.provenance.clone()).or_insert(0) += 1;
        }
        let mut map = Map::new();
        for (key, value) in counts {
            map.insert(key, json!(value));
        }
        Value::Object(map)
    }

    /// Deterministic Markdown serialization. Walks the same pre-sorted structures
    /// as [`Self::to_json`].
    fn to_markdown(&self, as_of: &str, generated_at: &str) -> String {
        let mut out = String::new();
        self.write_header(&mut out, as_of, generated_at);
        self.write_root_section(&mut out);
        self.write_findings_section(&mut out);
        self.write_evidence_section(&mut out);
        self.write_citations_section(&mut out);
        self.write_provenance_section(&mut out);
        self.write_unavailable_section(&mut out);
        out
    }

    fn write_header(&self, out: &mut String, as_of: &str, generated_at: &str) {
        use std::fmt::Write as _;
        out.push_str("# Research Trail\n\n");
        let _ = writeln!(out, "- **root**: `{}`", self.root.id);
        let _ = writeln!(out, "- **root label**: {}", self.root.label);
        let _ = writeln!(out, "- **root kind**: {:?}", self.root.kind);
        let _ = writeln!(out, "- **as_of**: {as_of}");
        let _ = writeln!(out, "- **generated_at**: {generated_at}");
        let _ = writeln!(
            out,
            "- **counts**: {} finding(s), {} evidence edge(s), {} citation(s), {} unavailable",
            self.findings.len(),
            self.evidence_edges.len(),
            self.citations.len(),
            self.unavailable.len(),
        );
        if self.evidence_truncated {
            let _ = writeln!(
                out,
                "- **note**: evidence scan truncated at {MAX_EVIDENCE_EDGES} edges (counts are a lower bound)"
            );
        }
        out.push('\n');
    }

    fn write_root_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("## Thesis / Root\n\n");
        if let Some(statement) = self.root.props.get("title").and_then(Value::as_str) {
            let _ = writeln!(out, "> {statement}\n");
        } else {
            let _ = writeln!(out, "> {}\n", self.root.label);
        }
        if let Some(horizon) = self.root.props.get("horizon_date").and_then(Value::as_str) {
            let _ = writeln!(out, "- horizon_date: {horizon}\n");
        }
    }

    fn write_findings_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("## Findings\n\n");
        if self.findings.is_empty() {
            out.push_str("_No findings in trail._\n\n");
            return;
        }
        for finding in &self.findings {
            let _ = writeln!(out, "### `{}` — {}\n", finding.id, finding.label);
            if let Some(body) = finding.props.get("body").and_then(Value::as_str) {
                let _ = writeln!(out, "{body}\n");
            }
        }
    }

    fn write_evidence_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("## Evidence\n\n");
        if self.evidence_edges.is_empty() {
            out.push_str("_No evidence edges in trail._\n\n");
            return;
        }
        for edge in &self.evidence_edges {
            let note = edge
                .note
                .as_deref()
                .map_or_else(String::new, |n| format!(" — {n}"));
            let _ = writeln!(
                out,
                "- `{}` —{}→ `{}` (provenance: {}){}",
                edge.from, edge.rel, edge.to, edge.provenance, note
            );
        }
        out.push('\n');
    }

    fn write_citations_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("## Cited Answers\n\n");
        if self.citations.is_empty() {
            out.push_str("_No citations in trail._\n\n");
            return;
        }
        for citation in &self.citations {
            let _ = write!(
                out,
                "- `{}` cites `{}`",
                citation.finding_id, citation.document_id
            );
            if let Some(url) = &citation.source_url {
                let _ = write!(out, " <{url}>");
            }
            out.push('\n');
            if let Some(snippet) = &citation.snippet {
                let _ = writeln!(out, "  > {snippet}");
            }
        }
        out.push('\n');
    }

    fn write_provenance_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        out.push_str("## Provenance\n\n");
        let provenance = self.provenance_index();
        let Some(obj) = provenance.as_object() else {
            return;
        };
        if obj.is_empty() {
            out.push_str("_No provenance recorded._\n\n");
            return;
        }
        for (key, value) in obj {
            let _ = writeln!(out, "- {key}: {value}");
        }
        out.push('\n');
    }

    fn write_unavailable_section(&self, out: &mut String) {
        use std::fmt::Write as _;
        if self.unavailable.is_empty() {
            return;
        }
        out.push_str("## Unavailable Evidence\n\n");
        for id in &self.unavailable {
            let _ = writeln!(out, "- [evidence unavailable: {id}]");
        }
        out.push('\n');
    }
}

// ── JSON node/edge/citation helpers (fixed key order) ──────────────────────────

fn node_to_json(node: &TrailNode) -> Value {
    json!({
        "id": node.id,
        "kind": format!("{:?}", node.kind),
        "label": node.label,
        "props": node.props,
        "valid_from": node.valid_from,
        "valid_to": node.valid_to,
    })
}

fn edge_to_json(edge: &TrailEdge) -> Value {
    json!({
        "from": edge.from,
        "rel": edge.rel,
        "to": edge.to,
        "provenance": edge.provenance,
        "note": edge.note,
        "valid_from": edge.valid_from,
        "valid_to": edge.valid_to,
    })
}

fn citation_to_json(citation: &Citation) -> Value {
    json!({
        "document_id": citation.document_id,
        "finding_id": citation.finding_id,
        "snippet": citation.snippet,
        "source_url": citation.source_url,
    })
}

// ── Trail assembly (read-only) ─────────────────────────────────────────────────

/// Assemble the trail rooted at `root_id`. Read-only — no graph writes.
async fn assemble_trail(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    root_id: &str,
    root_kind_arg: &str,
    as_of_ts: &str,
) -> Result<Trail, ToolFailure> {
    // Root must exist.
    let root_node = graph
        .node(root_id)
        .await
        .map_err(|e| execution(e.to_string()))?
        .ok_or_else(|| execution(format!("root {root_id:?} not found in the graph")))?;

    // If the caller declared a thesis root, verify the kind_hint honestly.
    if root_kind_arg == "thesis" {
        let kind_hint = root_node
            .props
            .get("kind_hint")
            .and_then(Value::as_str)
            .unwrap_or("");
        if kind_hint != "thesis" {
            return Err(execution(format!(
                "{root_id:?} is not a thesis (kind_hint={kind_hint:?}); \
                 use finding_id or entity_id instead"
            )));
        }
    }

    let root = TrailNode::from_graph(&root_node);

    let mut findings: Vec<TrailNode> = Vec::new();
    let mut evidence_edges: Vec<TrailEdge> = Vec::new();
    let mut citations: Vec<Citation> = Vec::new();
    let mut unavailable: Vec<String> = Vec::new();

    // The root finding/thesis itself contributes its own evidence pin + citation.
    if root_node.kind == EntityKind::Finding
        && let Some(citation) = citation_from_finding(&root_node)
    {
        citations.push(citation);
    }

    // Collect inbound evidence edges to the root (supports/contradicts/relates_to/
    // mentions). Leakage-safe: only edges active at as_of are kept.
    let filter = TraversalFilter {
        rels: Some(EVIDENCE_RELS.iter().map(|r| (*r).to_string()).collect()),
        direction: Direction::In,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    let inbound = graph
        .neighbors(root_id, &filter)
        .await
        .map_err(|e| execution(e.to_string()))?;

    // Walk inbound evidence edges. Leakage-safe: only edges active at as_of are
    // kept; the active filter is applied BEFORE the cap so inactive edges never
    // consume a cap slot. Returns the list of finding nodes whose `described_by`
    // documents must be resolved into citations.
    let mut evidence_finding_ids: Vec<String> = Vec::new();
    let mut evidence_truncated = false;
    let mut active_seen = 0usize;
    if root_node.kind == EntityKind::Finding {
        evidence_finding_ids.push(root_node.id.clone());
    }
    for (edge, neighbor) in &inbound {
        if !active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), as_of_ts) {
            continue;
        }
        if active_seen >= MAX_EVIDENCE_EDGES {
            evidence_truncated = true;
            break;
        }
        active_seen += 1;
        evidence_edges.push(trail_edge_from(edge));
        if neighbor.kind == EntityKind::Finding {
            evidence_finding_ids.push(neighbor.id.clone());
            findings.push(TrailNode::from_graph(neighbor));
            if let Some(citation) = citation_from_finding(neighbor) {
                citations.push(citation);
            }
        }
    }

    // Resolve `described_by` document targets into cited answers — reading real
    // graph ids, never inventing. A document or source finding that cannot be
    // fetched is recorded as unavailable, not dropped.
    resolve_described_by_citations(
        graph,
        &evidence_finding_ids,
        &mut citations,
        &mut unavailable,
    )
    .await?;

    // ── Deterministic ordering ────────────────────────────────────────────────
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    findings.dedup_by(|a, b| a.id == b.id);

    evidence_edges.sort_by(|a, b| {
        (&a.from, &a.rel, &a.to, &a.valid_from).cmp(&(&b.from, &b.rel, &b.to, &b.valid_from))
    });

    citations.sort_by(|a, b| {
        (&a.finding_id, &a.document_id).cmp(&(&b.finding_id, &b.document_id))
    });
    citations.dedup_by(|a, b| a.finding_id == b.finding_id && a.document_id == b.document_id);

    unavailable.sort();
    unavailable.dedup();

    Ok(Trail {
        root,
        findings,
        evidence_edges,
        citations,
        unavailable,
        evidence_truncated,
    })
}

/// Resolve the `described_by` document targets of every finding in
/// `finding_ids` into [`Citation`]s, appending real document ids to `citations`
/// and any unfetchable id to `unavailable`. Read-only.
///
/// Each finding's `described_by` edge points at the document node that backs it;
/// we resolve the target id (never invented — read from the edge) and verify the
/// node is fetchable so a dangling citation is recorded as unavailable, not
/// silently emitted.
async fn resolve_described_by_citations(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    finding_ids: &[String],
    citations: &mut Vec<Citation>,
    unavailable: &mut Vec<String>,
) -> Result<(), ToolFailure> {
    let doc_filter = TraversalFilter {
        rels: Some(vec!["described_by".to_string()]),
        direction: Direction::Out,
        max_hops: 1,
        ..TraversalFilter::default()
    };
    for finding_id in finding_ids {
        // Confirm the finding itself is fetchable — a dangling source is recorded.
        if graph
            .node(finding_id)
            .await
            .map_err(|e| execution(e.to_string()))?
            .is_none()
        {
            unavailable.push(finding_id.clone());
            continue;
        }
        let docs = graph
            .neighbors(finding_id, &doc_filter)
            .await
            .map_err(|e| execution(e.to_string()))?;
        for (_edge, doc_node) in docs.iter().take(MAX_OUTBOUND_EDGES) {
            // Verify the document node is fetchable; mark a gap explicitly.
            match graph.node(&doc_node.id).await {
                Ok(Some(_node)) => citations.push(Citation {
                    finding_id: finding_id.clone(),
                    document_id: doc_node.id.clone(),
                    snippet: None,
                    source_url: None,
                }),
                Ok(None) | Err(_) => unavailable.push(doc_node.id.clone()),
            }
        }
    }
    Ok(())
}

/// Build a [`Citation`] from a finding node's pinned `evidence` prop, if present.
/// Reads only real stored fields — never invents a document id.
fn citation_from_finding(node: &GraphNode) -> Option<Citation> {
    let evidence = node.props.get("evidence").and_then(Value::as_object)?;
    let document_id = evidence
        .get("document_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let source_url = evidence
        .get("source_url")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let snippet = evidence
        .get("snippet")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    // A pin with neither a document id nor a url is not a resolvable citation.
    let document_id = document_id.or_else(|| source_url.clone())?;
    Some(Citation {
        finding_id: node.id.clone(),
        document_id,
        snippet,
        source_url,
    })
}

/// Convert a graph edge into a trail edge with a provenance one-liner.
fn trail_edge_from(edge: &GraphEdge) -> TrailEdge {
    let note = edge
        .props
        .get("note")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    TrailEdge {
        from: edge.from.clone(),
        rel: edge.rel.clone(),
        to: edge.to.clone(),
        valid_from: edge.valid_from.clone(),
        valid_to: edge.valid_to.clone(),
        provenance: provenance_one_liner(&edge.provenance),
        note,
    }
}

/// A stable one-liner for a provenance value — mirrors the
/// [`crate::knowledge_explain_tools`] vocabulary so the export and `tdw.kg.why`
/// agree on how provenance reads.
fn provenance_one_liner(provenance: &tdw_core::Provenance) -> String {
    use tdw_core::Provenance;
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

// ── Validation / argument helpers ──────────────────────────────────────────────

/// Validate a YYYY-MM-DD date string (same grammar as the finding tools).
fn validate_date(value: &str) -> Result<(), ToolFailure> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || !value
            .chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
    {
        return Err(execution(format!("as_of must be YYYY-MM-DD, got {value:?}")));
    }
    Ok(())
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

// ── Tests (unit) ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn trail_fixture() -> Trail {
        Trail {
            root: TrailNode {
                id: "finding:thesis1".to_string(),
                kind: EntityKind::Finding,
                label: "AAPL is undervalued".to_string(),
                valid_from: Some("2026-06-01T00:00:00Z".to_string()),
                valid_to: None,
                props: json!({
                    "title": "AAPL is undervalued",
                    "kind_hint": "thesis",
                    "horizon_date": "2026-12-31"
                }),
            },
            findings: vec![TrailNode {
                id: "finding:f1".to_string(),
                kind: EntityKind::Finding,
                label: "Q3 revenue beat".to_string(),
                valid_from: Some("2026-06-02T00:00:00Z".to_string()),
                valid_to: None,
                props: json!({ "title": "Q3 revenue beat", "body": "Revenue above consensus." }),
            }],
            evidence_edges: vec![TrailEdge {
                from: "finding:f1".to_string(),
                rel: "supports".to_string(),
                to: "finding:thesis1".to_string(),
                valid_from: Some("2026-06-02T00:00:00Z".to_string()),
                valid_to: None,
                provenance: "agent:user:analyst-1".to_string(),
                note: Some("revenue beat supports valuation".to_string()),
            }],
            citations: vec![Citation {
                finding_id: "finding:f1".to_string(),
                document_id: "finding-doc:f1".to_string(),
                snippet: Some("Revenue of $X beat consensus of $Y.".to_string()),
                source_url: Some("https://example.com/q3".to_string()),
            }],
            unavailable: vec!["finding-doc:missing".to_string()],
            evidence_truncated: false,
        }
    }

    #[test]
    fn json_is_byte_identical_across_runs() {
        let trail = trail_fixture();
        let a = trail.to_json("2026-06-13", "inject-1");
        let b = trail.to_json("2026-06-13", "inject-1");
        // Serialize both to a string with the same serializer and compare bytes.
        let sa = serde_json::to_string(&a).expect("serialize a");
        let sb = serde_json::to_string(&b).expect("serialize b");
        assert_eq!(sa, sb, "JSON export must be byte-identical across runs");
    }

    #[test]
    fn markdown_is_byte_identical_across_runs() {
        let trail = trail_fixture();
        let a = trail.to_markdown("2026-06-13", "inject-1");
        let b = trail.to_markdown("2026-06-13", "inject-1");
        assert_eq!(a, b, "Markdown export must be byte-identical across runs");
    }

    #[test]
    fn unavailable_is_marked_explicitly_not_dropped() {
        let trail = trail_fixture();
        let md = trail.to_markdown("2026-06-13", "inject-1");
        assert!(
            md.contains("[evidence unavailable: finding-doc:missing]"),
            "missing evidence must be explicit in markdown: {md}"
        );
        let value = trail.to_json("2026-06-13", "inject-1");
        let unavailable = value["unavailable"]
            .as_array()
            .expect("unavailable array");
        assert!(
            unavailable
                .iter()
                .any(|v| v.as_str() == Some("[evidence unavailable: finding-doc:missing]")),
            "missing evidence must be explicit in json: {value}"
        );
    }

    #[test]
    fn citation_resolves_to_real_document_id() {
        let trail = trail_fixture();
        let value = trail.to_json("2026-06-13", "inject-1");
        let citations = value["citations"].as_array().expect("citations array");
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0]["document_id"], "finding-doc:f1");
        assert_eq!(citations[0]["finding_id"], "finding:f1");
    }

    #[test]
    fn counts_match_trail_contents() {
        let trail = trail_fixture();
        let value = trail.to_json("2026-06-13", "inject-1");
        assert_eq!(value["counts"]["findings"], 1);
        assert_eq!(value["counts"]["evidence"], 1);
        assert_eq!(value["counts"]["citations"], 1);
        assert_eq!(value["counts"]["unavailable"], 1);
    }

    #[test]
    fn generated_at_is_echoed_verbatim() {
        let trail = trail_fixture();
        let value = trail.to_json("2026-06-13", "caller-clock-2026");
        assert_eq!(value["generated_at"], "caller-clock-2026");
        let md = trail.to_markdown("2026-06-13", "caller-clock-2026");
        assert!(md.contains("caller-clock-2026"));
    }

    #[test]
    fn validate_date_accepts_iso_and_rejects_others() {
        assert!(validate_date("2026-06-13").is_ok());
        assert!(validate_date("2026-6-13").is_err());
        assert!(validate_date("not-a-date").is_err());
    }

    #[test]
    fn provenance_index_is_sorted_and_counts() {
        let mut trail = trail_fixture();
        trail.evidence_edges.push(TrailEdge {
            from: "finding:f2".to_string(),
            rel: "contradicts".to_string(),
            to: "finding:thesis1".to_string(),
            valid_from: None,
            valid_to: None,
            provenance: "agent:user:analyst-1".to_string(),
            note: None,
        });
        let index = trail.provenance_index();
        let obj = index.as_object().expect("provenance object");
        assert_eq!(obj.get("agent:user:analyst-1"), Some(&json!(2)));
    }
}
