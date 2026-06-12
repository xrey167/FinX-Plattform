//! MCP tools for first-class research findings and theses (K-X6 + K-X7).
//!
//! # Trust class: USER knowledge
//!
//! Findings and theses are authored and attributed to the calling user — they are NOT
//! operator-gated like B9 agent proposals.  The distinction matters:
//!
//! * **Agent proposals** (`tdw.kg.annotate` / `tdw.tags.define` / …) flow
//!   through the [`ProposalQueue`](tdw_knowledge::proposals::ProposalQueue)
//!   gate because agents can be adversarial, autonomous, and high-volume.
//!   They require `Adaptivity ≥ Learning` for submission and operator or
//!   eval-driven promotion before landing.
//!
//! * **User findings** (`tdw.kg.finding`) are personal research notes written
//!   by the human analyst who owns the surface.  They land immediately with
//!   `Provenance::Agent { agent_id: user_id, gated: false }`, attributed to
//!   the bound user identity.  This is appropriate because:
//!   1. The identity is HOST-BOUND (set at runtime construction, never
//!      accepted from the tool argument — identical to B9's host-binding).
//!   2. The same caps, control-character, and length validation that protect
//!      the B9 surface apply here.
//!   3. **Inference boundary**: `PropagateTag` rules CAN reach findings (tag
//!      propagation walks outbound edges from tag-holders — finding edges are
//!      not consumed as chain-join inputs, so auto-tagging is fine and aids
//!      retrieval).  `DeriveEdge` rules do NOT consume finding edges by
//!      default: [`tdw_infer::InferEngine`] excludes
//!      `Provenance::Agent { gated: false }` edges from chain matching unless
//!      the operator opts in via `InferEngine::with_user_authored_inference`.
//!      This prevents user findings from silently minting derived facts the
//!      operator never sanctioned.
//!   4. They are trust-dial-filterable: the K-X3 trust dial can narrow
//!      retrievals to `provenance: agent:<user_id>` so callers can distinguish
//!      personal findings from ingested or operator-materialized facts.
//!
//! # Thesis vs. Finding (K-X7 kind decision)
//!
//! A thesis is represented as a **`Finding` node with `props.kind_hint =
//! "thesis"`** — NOT as a 54th `EntityKind` variant. The reasons:
//!
//! 1. `EntityKind::ALL` is a const array ([54]); adding a 55th variant is a
//!    safe-but-disruptive ordinal shift for every downstream match.
//! 2. A thesis carries identical trust-class semantics to a finding: user
//!    provenance, host-bound identity, same caps, same `supports`/`contradicts`
//!    edge vocabulary.  Encoding this as a property saves an entire taxonomy
//!    extension for a single distinction.
//! 3. The `supports`/`contradicts` edge vocabulary from K-X6 is already the
//!    correct evidence stream — no new relation types are needed.
//!
//! The thesis capture tool (`tdw.kg.thesis`) is a thin wrapper over
//! `capture_finding` that injects `kind_hint = "thesis"` and validates the
//! `falsifiable_statement` field.  Health is read back via
//! `tdw.kg.thesis_health` and surfaced in `tdw.kg.status`.
//!
//! # Tools (K-X6)
//!
//! * `tdw.kg.finding` — capture a finding (create entity + index + auto-link).
//! * `tdw.kg.link`    — add a typed relation edge between two findings or
//!   between a finding and any graph entity.
//!
//! # Tools (K-X7)
//!
//! * `tdw.kg.thesis`        — capture a falsifiable thesis (Finding node with
//!   `kind_hint="thesis"` + optional `horizon_date`).
//! * `tdw.kg.thesis_health` — read-only: compute evidence-aggregation health
//!   for one thesis at an optional `as_of` date; returns supports count,
//!   contradicts count, evidence freshness, and balance trend.
//!
//! # K-M4 seam (contradiction detection hook)
//!
//! When K-M4's contradiction-detection pass finds a new fact that contradicts
//! a thesis's supporting evidence, it must be able to create the corresponding
//! `contradicts` edge from the finding to the thesis via `tdw.kg.link`.
//!
//! **Documented contract** (no dead code — K-M4 implements this):
//! ```text
//! POST tools/call { "name": "tdw.kg.link", "arguments": {
//!   "from_finding_id": "<finding-id>",
//!   "to": "<thesis-id>",
//!   "rel": "contradicts",
//!   "note": "<why-this-contradicts>"
//! }}
//! ```
//! `tdw.kg.link` already exists (K-X6) and accepts `contradicts`. K-M4 calls
//! it with the thesis id as the `to` target. No additional seam code is required
//! in this module; the `VALID_RELS` vocabulary and duplicate-detection logic in
//! `link_finding` are the complete interface. The only K-M4 obligation is to
//! resolve the `thesis_id` from a fact's provenance chain before calling link.
//!
//! # Relation vocabulary (K-X6)
//!
//! | `rel`          | Semantics |
//! |----------------|-----------|
//! | `relates_to`   | Thematic association — the finding is topically connected to the target. |
//! | `supports`     | The finding provides evidence in favour of the target claim or entity. |
//! | `contradicts`  | The finding challenges or refutes the target claim or entity. |
//!
//! These relations are directional (from-finding → to-target) and queryable
//! in both directions via `tdw.kg.traverse`.  Duplicate links (same
//! from/rel/to triple) are idempotently rejected with a loud error so the
//! caller's intent is clear.
//!
//! # K-X7 seam — contradiction evidence on closed edges
//!
//! K-X7 (knowledge-system K-X7, tracked in PR #391) extends this module to
//! emit a `contradicts` evidence edge whenever K-M4 closes a functional-
//! predicate edge that backs a finding.  The contract:
//!
//! * K-M4 closes the superseded edge and sets `invalidated_by` in its props.
//! * K-X7 reads `invalidated_by`, resolves the finding that cites the closed
//!   edge (via `described_by` / `mentioned_by`), and writes a
//!   `contradicts` edge from that finding to the newly-arrived superseding
//!   fact.
//!
//! No dead code is introduced here: K-X7 owns its own hook site and queries
//! the `invalidated_by` prop written by `ContradictionDetector::resolve`.
//! This note documents the seam so both stories stay coherent; the
//! implementation lives in the K-X7 branch.

use serde_json::{Map, Value, json};
use tdw_core::{Direction, GraphEdge, GraphNode, Provenance, TraversalFilter};
use tdw_kg::Entity;
use tdw_knowledge::proposals::validate_agent_id;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_taxonomy::EntityKind;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool names ────────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &[
    "tdw.kg.finding",
    "tdw.kg.link",
    "tdw.kg.thesis",
    "tdw.kg.thesis_health",
];

/// Whether `name` is one of the finding tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum character count for a finding title.
pub const MAX_TITLE_CHARS: usize = 256;
/// Maximum character count for a finding body.
pub const MAX_BODY_CHARS: usize = 8_192;
/// Maximum character count for the evidence snippet.
pub const MAX_SNIPPET_CHARS: usize = 2_048;
/// Maximum character count for a link note.
pub const MAX_NOTE_CHARS: usize = 1_024;
/// Maximum number of tags on a single finding.
pub const MAX_TAGS: usize = 32;
/// Maximum auto-link scan depth (number of known entity ids matched against
/// title+body tokens).  Bounded so a very large graph does not turn one
/// capture into an O(n) full-scan.
const AUTO_LINK_SCAN_LIMIT: usize = 512;

/// The three supported typed-relation tokens.
const VALID_RELS: &[&str] = &["relates_to", "supports", "contradicts"];

// ── Descriptors ───────────────────────────────────────────────────────────────

/// Descriptors for the finding and thesis tools (K-X6 + K-X7).
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        finding_descriptor(),
        link_descriptor(),
        thesis_descriptor(),
        thesis_health_descriptor(),
    ]
}

fn finding_tool(name: &str, title: &str, description: &str, input_schema: Value) -> ToolDescriptor {
    // Findings write into the graph (not read-only) but are idempotent on
    // the same title+body combination (content-hash based).
    tool_with_annotations(name, title, description, input_schema, false, false)
}

fn finding_descriptor() -> ToolDescriptor {
    finding_tool(
        "tdw.kg.finding",
        "Capture Research Finding",
        "Capture a first-class analyst finding and link it to known entities automatically. \
         Creates a Finding node in the knowledge graph, indexes it for hybrid search (so it is \
         retrievable via tdw.kg.search), and runs lexical mention-matching over title+body to \
         auto-link it to known entities — returning the created finding id and every auto-link \
         made so the result is immediately inspectable.\n\
         \n\
         Trust class: USER knowledge — lands immediately with user provenance (host-bound \
         identity, never accepted from the argument), no eval gate. PropagateTag rules CAN \
         reach findings (aids retrieval); DeriveEdge rules do NOT consume finding edges by \
         default (operator opt-in required). Use tdw.kg.link for explicit typed relations. \
         Trust-dial-filterable via provenance field. Requires the knowledge runtime with a \
         graph engine and a bound user id.\n\
         \n\
         Optional `evidence` pin: supply a `document_id` (or `source_url`) and a `snippet` to \
         immutably attach the exact paragraph that justifies the finding; tdw.kg.why on the \
         finding will surface the snippet as a chain step.\n\
         \n\
         Caps: title ≤ 256 chars; body ≤ 8 192 chars; snippet ≤ 2 048 chars; tags ≤ 32; \
         no control characters in any text field.",
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Finding title (required, ≤ 256 chars, no control characters)."
                },
                "body": {
                    "type": "string",
                    "description": "Finding body / detail (optional, ≤ 8 192 chars, no control characters)."
                },
                "source_url": {
                    "type": "string",
                    "description": "URL of the source that prompted this finding (optional, validated)."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional taxonomy tags (≤ 32). Each must be a non-empty, \
                                   colon-separated identifier (e.g. sector:tech)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today)."
                },
                "evidence": {
                    "type": "object",
                    "properties": {
                        "document_id": { "type": "string" },
                        "source_url":  { "type": "string" },
                        "snippet":     { "type": "string", "description": "≤ 2 048 chars." }
                    },
                    "additionalProperties": false,
                    "description": "Pin the exact evidence paragraph that justifies this finding."
                }
            },
            "required": ["title"],
            "additionalProperties": false
        }),
    )
}

fn link_descriptor() -> ToolDescriptor {
    finding_tool(
        "tdw.kg.link",
        "Link Finding to Entity or Finding",
        "Create a typed relation edge from a finding to another finding or any graph entity. \
         Relation must be one of: relates_to, supports, contradicts. Both endpoints must exist. \
         Duplicate links (same from/rel/to) are rejected loudly — idempotency is the caller's \
         responsibility. The edge is queryable in both directions via tdw.kg.traverse.\n\
         \n\
         Trust class: USER knowledge — same host-bound identity and instant-write posture as \
         tdw.kg.finding. Requires graph engine + bound user id.",
        json!({
            "type": "object",
            "properties": {
                "from_finding_id": {
                    "type": "string",
                    "description": "The finding node id (e.g. finding:abc123)."
                },
                "to": {
                    "type": "string",
                    "description": "The target entity or finding id."
                },
                "rel": {
                    "type": "string",
                    "enum": ["relates_to", "supports", "contradicts"],
                    "description": "The typed relation. relates_to = topical; supports = evidence in favour; contradicts = refutes."
                },
                "note": {
                    "type": "string",
                    "description": "Optional annotation on this link (≤ 1 024 chars, no control characters)."
                }
            },
            "required": ["from_finding_id", "to", "rel"],
            "additionalProperties": false
        }),
    )
}

fn thesis_descriptor() -> ToolDescriptor {
    // Theses write into the graph (same posture as tdw.kg.finding).
    finding_tool(
        "tdw.kg.thesis",
        "Capture Thesis",
        "Capture a falsifiable research thesis — a claim the analyst wants to accumulate \
         evidence for or against over time. A thesis is stored as a Finding node with \
         `props.kind_hint = \"thesis\"` so it inherits all Finding trust-class semantics \
         (user provenance, host-bound identity, same caps) while remaining a plain \
         EntityKind::Finding for the taxonomy (no new variant needed).\n\
         \n\
         Evidence accumulates via `tdw.kg.link` `supports` / `contradicts` edges pointing \
         TO the thesis id.  Health is read via `tdw.kg.thesis_health`.\n\
         \n\
         Optional `horizon_date` (YYYY-MM-DD): the date by which the thesis should be \
         confirmed or refuted.  Stored in `props.horizon_date`; surfaced in health output.\n\
         \n\
         Trust class: USER knowledge — lands immediately with user provenance, no eval gate. \
         Requires graph engine + bound user id.\n\
         \n\
         Caps: falsifiable_statement ≤ 256 chars (same as finding title); body ≤ 8 192 chars; \
         tags ≤ 32; no control characters.",
        json!({
            "type": "object",
            "properties": {
                "falsifiable_statement": {
                    "type": "string",
                    "description": "The thesis claim (≤ 256 chars, no control chars). Must be non-empty after trim."
                },
                "body": {
                    "type": "string",
                    "description": "Extended rationale (optional, ≤ 8 192 chars)."
                },
                "horizon_date": {
                    "type": "string",
                    "description": "Date by which this thesis should be confirmed/refuted (optional, YYYY-MM-DD)."
                },
                "source_url": {
                    "type": "string",
                    "description": "URL of the source that prompted this thesis (optional, validated)."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional taxonomy tags (≤ 32, colon-separated identifiers)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today)."
                }
            },
            "required": ["falsifiable_statement"],
            "additionalProperties": false
        }),
    )
}

fn thesis_health_descriptor() -> ToolDescriptor {
    // Health is read-only and idempotent.
    tool_with_annotations(
        "tdw.kg.thesis_health",
        "Thesis Evidence Health",
        "Compute evidence-accumulation health for one thesis at an optional `as_of` date. \
         Scans the graph for `supports` and `contradicts` edges pointing TO the thesis id, \
         counting only edges whose `valid_from ≤ as_of` (temporal honesty — no future-evidence \
         leakage). Returns:\n\
         * `supports_count` / `contradicts_count` — counts of active evidence edges at `as_of`. \
           Exact when `counts_truncated=false`; a lower bound when `counts_truncated=true`.\n\
         * `counts_truncated` — `true` when the active-edge scan exceeded the 1 024-edge cap; \
           inactive (future/tombstoned) edges are skipped before the cap is tested.\n\
         * `evidence_freshness_days` — age in days of the newest evidence edge at `as_of` \
           (`null` when no evidence exists).\n\
         * `balance` — `\"bullish\"` (supports > contradicts), `\"bearish\"` (contradicts > \
           supports), or `\"neutral\"` (equal or zero).\n\
         * `horizon_date` — from the thesis node's props (may be absent).\n\
         * `horizon_overdue` — true when `as_of ≥ horizon_date` and no clear majority.\n\
         \n\
         Health is computed deterministically from the graph at read time. Read-only, idempotent. \
         Requires graph engine.",
        json!({
            "type": "object",
            "properties": {
                "thesis_id": {
                    "type": "string",
                    "description": "The thesis node id (e.g. finding:abc123 with kind_hint=thesis)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Temporal point of view (YYYY-MM-DD). Defaults to today."
                }
            },
            "required": ["thesis_id"],
            "additionalProperties": false
        }),
        true, // readOnlyHint
        true, // idempotentHint
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch one finding tool.  Every failure is a tool error.
///
/// `now` is the current instant as `YYYY-MM-DD` (injected by the MCP layer —
/// nothing inside this module reads the clock directly).
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing engine, missing user id,
/// malformed input, or an engine failure — never [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    name: &str,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.finding" => capture_finding(runtime, arguments, now),
        "tdw.kg.link" => link_finding(runtime, arguments, now),
        "tdw.kg.thesis" => capture_thesis(runtime, arguments, now),
        "tdw.kg.thesis_health" => thesis_health(runtime, arguments, now),
        other => Err(execution(format!("unknown finding tool: {other}"))),
    }
}

// ── tdw.kg.finding ────────────────────────────────────────────────────────────

/// Server-side thesis metadata injected by `capture_thesis` only.
///
/// This struct is constructed exclusively from Rust-validated state inside
/// `capture_thesis` and passed as a typed argument to `capture_finding_inner`.
/// It is **never** derived from caller-supplied MCP arguments — external callers
/// cannot forge a thesis or override the id by passing keys in the JSON args.
///
/// The K-L5 model: identity and kind are decided by the SERVER (which tool path
/// executed + which principal is host-bound), not by the client.
struct ThesisInject<'a> {
    /// Pre-computed FNV-1a hex for the thesis id namespace (without `"finding:"` prefix).
    id_hex: String,
    /// Always `"thesis"` — set by the server path, never the caller.
    kind_hint: &'a str,
    /// Validated `YYYY-MM-DD` horizon date, or `None`.
    horizon_date: Option<String>,
    /// Host-bound user id attributed as thesis author.
    thesis_user_id: &'a str,
}

/// Public entry point for `tdw.kg.finding`.
///
/// Strips/ignores ALL internal-only keys before delegating.  External callers
/// cannot influence `kind_hint`, `_id_override`, `horizon_date`, or
/// `thesis_user_id` through this path — those fields are silently dropped here
/// and only flow through the Rust-typed [`ThesisInject`] channel.
fn capture_finding(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    capture_finding_inner(runtime, arguments, now, None)
}

// Validation + graph writes + index pipeline in one function — split would hurt
// readability more than it helps; lint suppressed per project convention.
#[allow(clippy::too_many_lines)]
fn capture_finding_inner(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
    thesis: Option<&ThesisInject<'_>>,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let user_id = require_user_id(runtime)?;

    // --- argument extraction & validation ---
    let title = require_str(arguments, "title")?;
    validate_title(title)?;

    let body = optional_str(arguments, "body").unwrap_or("");
    validate_text_field(body, "body", MAX_BODY_CHARS)?;

    let source_url = optional_str(arguments, "source_url");
    if let Some(url) = source_url {
        validate_url(url)?;
    }

    let tags = optional_string_array(arguments, "tags")?.unwrap_or_default();
    if tags.len() > MAX_TAGS {
        return Err(execution(format!(
            "tags must have at most {MAX_TAGS} items, got {}",
            tags.len()
        )));
    }
    for tag in &tags {
        validate_tag_id(tag)?;
    }

    let as_of =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;

    // Evidence pin (optional).
    let evidence = arguments.get("evidence").and_then(Value::as_object);
    let evidence_doc_id = evidence
        .and_then(|obj| obj.get("document_id"))
        .and_then(Value::as_str);
    let evidence_url = evidence
        .and_then(|obj| obj.get("source_url"))
        .and_then(Value::as_str);
    let evidence_snippet = evidence
        .and_then(|obj| obj.get("snippet"))
        .and_then(Value::as_str);

    if let Some(snippet) = evidence_snippet {
        validate_text_field(snippet, "evidence.snippet", MAX_SNIPPET_CHARS)?;
    }
    if let Some(url) = evidence_url {
        validate_url(url)?;
    }

    // --- stable finding id ---
    // For plain findings: FNV-1a hash of the title.
    // For theses: server-computed domain-prefixed hash from ThesisInject —
    // never from caller args (forge guard).
    let (finding_id, document_id) = thesis.as_ref().map_or_else(
        || {
            let id_hash = fnv1a64(title.as_bytes());
            (
                format!("finding:{id_hash:016x}"),
                format!("finding-doc:{id_hash:016x}"),
            )
        },
        |t| {
            let hex = &t.id_hex;
            (format!("finding:{hex}"), format!("finding-doc:{hex}"))
        },
    );

    let as_of_ts = tdw_tags::date_to_timestamp(&as_of);

    // Build the search body: title + body concatenated for hybrid retrieval.
    let search_body = if body.is_empty() {
        title.to_string()
    } else {
        format!("{title}\n{body}")
    };

    // --- scan for auto-links against known graph entities ---
    let auto_links = block_on(scan_auto_links(graph, title, body))?;

    // --- props stored on the finding node ---
    let mut props = json!({
        "title": title,
        "user_id": user_id,
        "as_of": as_of_ts,
    });
    if !body.is_empty() {
        props["body"] = json!(body);
    }
    if let Some(url) = source_url {
        props["source_url"] = json!(url);
    }
    if !tags.is_empty() {
        props["tags"] = json!(tags);
    }
    // Thesis-specific props — only written when the call comes through the
    // server-internal ThesisInject channel (i.e. from capture_thesis).
    // External callers using tdw.kg.finding cannot set these regardless of
    // what they pass in the JSON args (forge guard).
    if let Some(t) = &thesis {
        props["kind_hint"] = json!(t.kind_hint);
        if let Some(h) = &t.horizon_date {
            props["horizon_date"] = json!(h);
        }
        props["thesis_user_id"] = json!(t.thesis_user_id);
    }

    // Evidence pin: store immutably with the snippet content hash.
    if evidence.is_some() {
        let mut ev = json!({});
        if let Some(doc_id) = evidence_doc_id {
            ev["document_id"] = json!(doc_id);
        }
        if let Some(url) = evidence_url {
            ev["source_url"] = json!(url);
        }
        if let Some(snippet) = evidence_snippet {
            let snippet_hash = format!("{:016x}", fnv1a64(snippet.as_bytes()));
            ev["snippet"] = json!(snippet);
            ev["snippet_hash"] = json!(snippet_hash);
            ev["as_of"] = json!(&as_of_ts);
        }
        props["evidence"] = ev;
    }

    // --- write finding node + document node + edges ---
    block_on(async {
        let finding_node = GraphNode {
            id: finding_id.clone(),
            kind: EntityKind::Finding,
            label: title.to_string(),
            aliases: Vec::new(),
            props: props.clone(),
            valid_from: Some(as_of_ts.clone()),
            valid_to: None,
        };
        let document_node = GraphNode {
            id: document_id.clone(),
            kind: EntityKind::Document,
            label: document_id.clone(),
            aliases: Vec::new(),
            props: json!({
                "as_of": &as_of_ts,
                "plane": "user",
                "entity_id": &finding_id,
            }),
            valid_from: Some(as_of_ts.clone()),
            valid_to: None,
        };
        graph
            .upsert_nodes(vec![finding_node, document_node])
            .await
            .map_err(|e| execution(e.to_string()))?;

        let user_provenance = Provenance::Agent {
            agent_id: user_id.to_string(),
            gated: false,
        };

        let mut edges = vec![GraphEdge {
            from: finding_id.clone(),
            to: document_id.clone(),
            rel: "described_by".to_string(),
            props: Value::Null,
            provenance: user_provenance.clone(),
            valid_from: Some(as_of_ts.clone()),
            valid_to: None,
        }];

        // Auto-link edges (mentions).
        for target_id in &auto_links {
            edges.push(GraphEdge {
                from: finding_id.clone(),
                to: target_id.clone(),
                rel: "mentions".to_string(),
                props: Value::Null,
                provenance: user_provenance.clone(),
                valid_from: Some(as_of_ts.clone()),
                valid_to: None,
            });
        }

        graph
            .upsert_edges(edges)
            .await
            .map_err(|e| execution(e.to_string()))
    })?;

    // --- index through KnowledgeIndexer for hybrid search ---
    if let Some(indexer) = runtime.finding_indexer() {
        let doc_id_hex = finding_id
            .strip_prefix("finding:")
            .unwrap_or(&finding_id)
            .to_string();
        let doc = tdw_knowledge::KnowledgeDocument {
            id: doc_id_hex,
            body: search_body,
            entity: Entity {
                entity_id: finding_id.clone(),
                kind: EntityKind::Finding,
                label: title.to_string(),
                aliases: Vec::new(),
            },
            tags,
            source: None,
            plane: Some("user".to_string()),
            as_of: Some(as_of.clone()),
            mentions: auto_links.clone(),
        };
        // `std::sync::Mutex::lock` yields a non-Send guard that cannot cross
        // an `async {}` boundary inside the Send-bound `block_on`.  Use
        // `block_in_place` + `Handle::block_on` directly — this pair doesn't
        // require `Send` and matches what `block_on` does on a multi-thread
        // runtime internally.
        let mut guard = indexer
            .lock()
            .map_err(|_| execution("finding indexer mutex poisoned".to_string()))?;
        index_finding_blocking(&mut guard, doc, &as_of)?;
    }

    Ok(structured(json!({
        "finding_id": finding_id,
        "title": title,
        "as_of": as_of,
        "auto_links": auto_links,
        "auto_links_count": auto_links.len(),
        "evidence_pinned": evidence.is_some(),
    })))
}

// ── tdw.kg.link ───────────────────────────────────────────────────────────────

fn link_finding(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let user_id = require_user_id(runtime)?;

    let from_id = require_str(arguments, "from_finding_id")?;
    let to_id = require_str(arguments, "to")?;
    let rel = require_str(arguments, "rel")?;

    // Validate relation token.
    if !VALID_RELS.contains(&rel) {
        return Err(execution(format!(
            "rel must be one of {VALID_RELS:?}, got {rel:?}"
        )));
    }

    let note = optional_str(arguments, "note");
    if let Some(n) = note {
        validate_text_field(n, "note", MAX_NOTE_CHARS)?;
    }

    // Both endpoints must exist.
    block_on(async {
        for endpoint in [from_id, to_id] {
            if graph
                .node(endpoint)
                .await
                .map_err(|e| execution(e.to_string()))?
                .is_none()
            {
                return Err(execution(format!("entity {endpoint:?} does not exist")));
            }
        }
        Ok(())
    })?;

    // Duplicate link detection: use a scoped one-hop neighbors lookup (O(degree)
    // not O(total edges)) — only the outgoing edges from `from_id` with `rel`
    // need to be inspected.
    let duplicate = block_on(async {
        let filter = TraversalFilter {
            rels: Some(vec![rel.to_string()]),
            direction: Direction::Out,
            max_hops: 1,
            ..TraversalFilter::default()
        };
        let neighbors = graph
            .neighbors(from_id, &filter)
            .await
            .map_err(|e| execution(e.to_string()))?;
        Ok::<bool, ToolFailure>(neighbors.iter().any(|(edge, _)| edge.to == to_id))
    })?;

    if duplicate {
        return Err(execution(format!(
            "link {from_id} -{rel}-> {to_id} already exists — duplicate links are rejected \
             (idempotency is the caller's responsibility)"
        )));
    }

    let mut props = json!({});
    if let Some(n) = note {
        props["note"] = json!(n);
    }

    // `now` is the injected date string (YYYY-MM-DD) from the MCP dispatch
    // layer; convert to a timestamp for the valid_from field.
    let as_of_ts = tdw_tags::date_to_timestamp(now);

    block_on(async {
        graph
            .upsert_edges(vec![GraphEdge {
                from: from_id.to_string(),
                to: to_id.to_string(),
                rel: rel.to_string(),
                props,
                provenance: Provenance::Agent {
                    agent_id: user_id.to_string(),
                    gated: false,
                },
                valid_from: Some(as_of_ts.clone()),
                valid_to: None,
            }])
            .await
            .map_err(|e| execution(e.to_string()))
    })?;

    Ok(structured(json!({
        "linked": {
            "from": from_id,
            "rel": rel,
            "to": to_id,
        },
        "note": note,
        "created_at": as_of_ts,
    })))
}

// ── tdw.kg.thesis ─────────────────────────────────────────────────────────────

/// Capture a falsifiable thesis (K-X7).
///
/// A thesis is a Finding node with `props.kind_hint = "thesis"`.  All
/// validation mirrors `capture_finding`; the only additions are:
/// * `falsifiable_statement` is used as the finding title (same 256-char cap).
/// * `horizon_date` (optional YYYY-MM-DD) is stored in `props.horizon_date`.
/// * `props.kind_hint = "thesis"` is always written so health and why tools
///   can identify the node as a thesis without a taxonomy variant.
///
/// # Id namespace isolation
///
/// The id hash is computed over `"thesis:{statement}"` (domain-prefix on the
/// raw FNV-1a input bytes).  A plain finding whose title happens to equal the
/// thesis statement hashes the title directly and therefore produces a different
/// `finding:…` id — the two namespaces cannot collide.
///
/// # Forge guard (K-L5 model)
///
/// `kind_hint`, `horizon_date`, `thesis_user_id`, and the id override are
/// never read from the caller-supplied `arguments` map.  They are constructed
/// here from validated Rust state and passed to [`capture_finding_inner`] via
/// the typed [`ThesisInject`] channel.  External callers cannot forge a thesis
/// or override the id by sending internal keys through `tdw.kg.finding`.
fn capture_thesis(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    // Validate the falsifiable_statement (same 256-char cap as a finding title).
    let statement = require_str(arguments, "falsifiable_statement")?;
    validate_title(statement)?;

    // Horizon date — validated before building the args map.
    let horizon_date = optional_str(arguments, "horizon_date").map(ToString::to_string);
    if let Some(h) = &horizon_date {
        validate_date(h)?;
    }

    let user_id = require_user_id(runtime)?;

    // Domain-prefixed id hash — disjoint from plain finding ids.
    let id_hex = format!("{:016x}", fnv1a64(format!("thesis:{statement}").as_bytes()));

    // ThesisInject is Rust-typed; it never touches the MCP args map.
    let inject = ThesisInject {
        id_hex,
        kind_hint: "thesis",
        horizon_date: horizon_date.clone(),
        thesis_user_id: user_id,
    };

    // Build the argument map with only the keys the finding tool schema accepts.
    // Internal keys (_id_override, kind_hint, etc.) are intentionally absent —
    // they flow through ThesisInject, not through the args map.
    let mut args: Map<String, Value> = Map::new();
    args.insert("title".to_string(), Value::String(statement.to_string()));
    if let Some(body) = optional_str(arguments, "body") {
        args.insert("body".to_string(), Value::String(body.to_string()));
    }
    if let Some(url) = optional_str(arguments, "source_url") {
        args.insert("source_url".to_string(), Value::String(url.to_string()));
    }
    if let Some(tags_val) = arguments.get("tags") {
        args.insert("tags".to_string(), tags_val.clone());
    }
    if let Some(as_of) = optional_str(arguments, "as_of") {
        args.insert("as_of".to_string(), Value::String(as_of.to_string()));
    }

    let result = capture_finding_inner(runtime, &args, now, Some(&inject))?;

    let finding_id = result
        .structured
        .get("finding_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            execution("thesis: capture_finding_inner returned no finding_id".to_string())
        })?
        .to_string();

    let as_of_str =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    let as_of_ts = tdw_tags::date_to_timestamp(&as_of_str);

    Ok(structured(json!({
        "thesis_id": finding_id,
        "falsifiable_statement": statement,
        "horizon_date": horizon_date,
        "as_of": as_of_str,
        "as_of_ts": as_of_ts,
        "kind_hint": "thesis",
        "auto_links": result.structured.get("auto_links").cloned().unwrap_or_else(|| json!([])),
        "auto_links_count": result.structured.get("auto_links_count").cloned().unwrap_or_else(|| json!(0)),
    })))
}

// ── tdw.kg.thesis_health ──────────────────────────────────────────────────────

/// Maximum number of inbound evidence edges scanned for a single thesis health
/// query.  Keeps the read bounded regardless of graph size.
const THESIS_HEALTH_EDGE_CAP: usize = 1_024;

/// Health computation for one thesis (K-X7).
///
/// Scans inbound `supports` and `contradicts` edges TO the thesis id.
/// Uses `active_at(valid_from, valid_to, as_of_ts)` exclusively — the same
/// predicate the graph engine and diff tool use, so temporal-leakage bugs
/// cannot be introduced here independently.
///
/// # Temporal honesty (`as_of` leakage regression contract)
///
/// Only edges with `valid_from ≤ as_of` (open or within window) are counted.
/// Edges with `valid_from > as_of` are silently skipped — they represent
/// evidence that did not yet exist at the queried point in time.  The
/// `evidence_freshness_days` field is computed from the newest edge that is
/// active at `as_of`, not from edges that became valid after it.
fn thesis_health(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let thesis_id = require_str(arguments, "thesis_id")?;
    let as_of = optional_str(arguments, "as_of").unwrap_or(now);
    validate_date(as_of)?;

    let as_of_ts = tdw_tags::date_to_timestamp(as_of);

    // Verify the thesis node exists and carry its props.
    let node = block_on(async {
        graph
            .node(thesis_id)
            .await
            .map_err(|e| execution(e.to_string()))
    })?
    .ok_or_else(|| execution(format!("thesis {thesis_id:?} not found")))?;

    // Verify it is indeed a thesis (kind_hint prop).
    let kind_hint = node
        .props
        .get("kind_hint")
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind_hint != "thesis" {
        return Err(execution(format!(
            "{thesis_id:?} is not a thesis (kind_hint={kind_hint:?}); \
             use tdw.kg.finding for plain findings"
        )));
    }

    let horizon_date = node
        .props
        .get("horizon_date")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    // Collect inbound supports/contradicts edges to this thesis.
    // active_at filter is applied BEFORE the cap — inactive edges never consume
    // a cap slot, so counts are exact for theses with ≤ THESIS_HEALTH_EDGE_CAP
    // active evidence edges.
    let (supports_count, contradicts_count, newest_active_ts, counts_truncated) =
        block_on(async { compute_thesis_health_counts(graph, thesis_id, &as_of_ts).await })?;

    // Evidence freshness in days: age of newest evidence edge at as_of.
    let evidence_freshness_days = newest_active_ts
        .as_deref()
        .map(|newest_ts| compute_age_days(newest_ts, &as_of_ts));

    // Balance signal.
    let balance = match supports_count.cmp(&contradicts_count) {
        std::cmp::Ordering::Greater => "bullish",
        std::cmp::Ordering::Less => "bearish",
        std::cmp::Ordering::Equal => "neutral",
    };

    // Horizon overdue: as_of >= horizon_date AND no clear majority.
    let horizon_overdue = horizon_date
        .as_deref()
        .is_some_and(|h| as_of >= h && balance == "neutral");

    // Honest truncation note — never silently under-count.
    let counts_note: Option<&str> = if counts_truncated {
        Some("active edge count exceeded scan cap; supports/contradicts counts are a lower bound")
    } else {
        None
    };

    Ok(structured(json!({
        "thesis_id": thesis_id,
        "falsifiable_statement": node.props.get("title").cloned().unwrap_or(Value::Null),
        "as_of": as_of,
        "supports_count": supports_count,
        "contradicts_count": contradicts_count,
        "counts_truncated": counts_truncated,
        "counts_note": counts_note,
        "evidence_freshness_days": evidence_freshness_days,
        "balance": balance,
        "horizon_date": horizon_date,
        "horizon_overdue": horizon_overdue,
        "edge_scan_cap": THESIS_HEALTH_EDGE_CAP,
        "health_note": "Computed deterministically from graph at read time; \
                        only edges active at as_of are counted (temporal honesty). \
                        Inactive edges do not consume the scan cap."
    })))
}

/// Scan inbound `supports` and `contradicts` edges to `thesis_id` that are
/// active at `as_of_ts`, bounded by [`THESIS_HEALTH_EDGE_CAP`].
///
/// Returns `(supports_count, contradicts_count, newest_valid_from, truncated)`
/// where:
/// * `newest_valid_from` is the `valid_from` timestamp of the most recently
///   created active evidence edge.
/// * `truncated` is `true` when more than [`THESIS_HEALTH_EDGE_CAP`] active
///   edges exist and the scan was stopped early.  The caller MUST surface this
///   in the response so the user knows the counts are a lower bound.
///
/// # Ordering guarantee
///
/// The `active_at` predicate is applied BEFORE the cap is tested.  Inactive
/// (future or tombstoned) edges do not consume cap slots — only edges that
/// actually count at `as_of_ts` do.  This prevents under-counting on theses
/// with many historically-closed evidence edges.
async fn compute_thesis_health_counts(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    thesis_id: &str,
    as_of_ts: &str,
) -> Result<(usize, usize, Option<String>, bool), ToolFailure> {
    use tdw_core::{Direction, TraversalFilter, active_at};

    let filter = TraversalFilter {
        rels: Some(vec!["supports".to_string(), "contradicts".to_string()]),
        direction: Direction::In,
        max_hops: 1,
        ..TraversalFilter::default()
    };

    // neighbors returns (edge, node) pairs — edge.to == thesis_id for In direction.
    let neighbors = graph
        .neighbors(thesis_id, &filter)
        .await
        .map_err(|e| execution(e.to_string()))?;

    let mut supports_count: usize = 0;
    let mut contradicts_count: usize = 0;
    let mut newest_active_ts: Option<String> = None;
    let mut active_scanned: usize = 0;
    let mut truncated = false;

    for (edge, _node) in &neighbors {
        // Temporal honesty: inactive edges are skipped and do NOT consume a cap slot.
        if !active_at(
            edge.valid_from.as_deref(),
            edge.valid_to.as_deref(),
            as_of_ts,
        ) {
            continue;
        }

        // Cap is applied only to active edges — ensures counts are exact up to the cap.
        if active_scanned >= THESIS_HEALTH_EDGE_CAP {
            truncated = true;
            break;
        }
        active_scanned += 1;

        match edge.rel.as_str() {
            "supports" => supports_count += 1,
            "contradicts" => contradicts_count += 1,
            _ => {}
        }

        // Track the newest valid_from among active evidence edges.
        if let Some(vf) = edge.valid_from.as_deref() {
            match newest_active_ts.as_deref() {
                None => newest_active_ts = Some(vf.to_string()),
                Some(current) if vf > current => newest_active_ts = Some(vf.to_string()),
                _ => {}
            }
        }
    }

    Ok((
        supports_count,
        contradicts_count,
        newest_active_ts,
        truncated,
    ))
}

/// Age in days between two UTC timestamps.
///
/// Parses the leading `YYYY-MM-DD` portion of each timestamp and delegates to
/// [`chrono::NaiveDate::signed_duration_since`] for exact Gregorian arithmetic.
/// The previous Julian-day formula omitted the Jan/Feb year-shift and produced
/// off-by-one errors for date pairs that span a month boundary in those two
/// months (Gemini K-X7 #391).
fn compute_age_days(from_ts: &str, to_ts: &str) -> i64 {
    use chrono::NaiveDate;
    fn parse_date(ts: &str) -> Option<NaiveDate> {
        // Accept both bare dates ("YYYY-MM-DD") and RFC 3339 timestamps
        // ("YYYY-MM-DDT…") by slicing the leading 10 bytes.
        NaiveDate::parse_from_str(ts.get(0..10)?, "%Y-%m-%d").ok()
    }
    match (parse_date(from_ts), parse_date(to_ts)) {
        (Some(from), Some(to)) => to.signed_duration_since(from).num_days().abs(),
        _ => 0,
    }
}

// ── Auto-link scan ────────────────────────────────────────────────────────────

/// Scan `title` and `body` for mentions of known graph entities using a
/// lexical token-matching approach.  Returns at most [`AUTO_LINK_SCAN_LIMIT`]
/// entity ids that appear by their id suffix (the part after the last `:`
/// in a graph id, e.g. `AAPL` from `instrument:AAPL`) as whole tokens within
/// the combined text.  The graph is queried for edges to enumerate known
/// entity ids; the scan is bounded by the limit constant.
///
/// Tombstoned / merged entities (`props.merged_into` present) and entities
/// with a closed `valid_to` window are excluded so auto-links never point at
/// dead nodes.
///
/// Only entities already present in the graph are linked — stub creation is
/// not triggered here.
async fn scan_auto_links(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    title: &str,
    body: &str,
) -> Result<Vec<String>, ToolFailure> {
    // Collect a bounded sample of entity ids from the graph by scanning edges.
    let mut entity_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut offset = 0usize;
    let page_size = 256usize;

    'scan: loop {
        let page = graph
            .edges(None, offset, page_size)
            .await
            .map_err(|e| execution(e.to_string()))?;
        if page.is_empty() {
            break;
        }
        for edge in &page {
            entity_ids.insert(edge.from.clone());
            entity_ids.insert(edge.to.clone());
            if entity_ids.len() >= AUTO_LINK_SCAN_LIMIT {
                break 'scan;
            }
        }
        offset += page.len();
        if offset >= AUTO_LINK_SCAN_LIMIT * 4 {
            break;
        }
    }

    let combined = format!("{title} {body}");
    let combined_lower = combined.to_lowercase();

    // Filter: (a) must mention the token, (b) must not be tombstoned/merged,
    // (c) must not have a closed valid_to window.
    let mut matched: Vec<String> = Vec::new();
    for id in entity_ids {
        let token = id.split(':').next_back().unwrap_or(id.as_str());
        let token_lower = token.to_lowercase();
        if token_lower.is_empty() {
            continue;
        }
        if !contains_token(&combined_lower, &token_lower)
            && !contains_token(&combined_lower, &id.to_lowercase())
        {
            continue;
        }
        // Check tombstone/merge status and validity window.
        if let Ok(Some(node)) = graph.node(&id).await {
            // Skip merged/tombstoned nodes — they redirect to a successor.
            if node.props.get("merged_into").is_some() {
                continue;
            }
            // Skip nodes whose valid_to has passed (closed window).
            if let Some(valid_to) = node.valid_to.as_deref() {
                // A non-empty valid_to means the entity is no longer current.
                if !valid_to.is_empty() {
                    continue;
                }
            }
        }
        matched.push(id);
    }

    matched.sort();
    matched.dedup();
    Ok(matched)
}

/// Bridge the async [`KnowledgeIndexer::index_at`] from a sync context while
/// holding a non-`Send` `MutexGuard`.  Uses `block_in_place` + `Handle::block_on`
/// directly (the same pair `block_on` uses internally on a multi-thread runtime)
/// so the `Send` bound does not apply.
fn index_finding_blocking(
    indexer: &mut tdw_knowledge::indexer::KnowledgeIndexer,
    doc: tdw_knowledge::KnowledgeDocument,
    as_of: &str,
) -> Result<(), ToolFailure> {
    use tokio::runtime::{Builder, Handle, RuntimeFlavor};
    let fut = indexer.index_at(doc, as_of);
    let result = match Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        Ok(handle) => handle.block_on(fut),
        Err(_) => Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| execution(format!("runtime build: {e}")))?
            .block_on(fut),
    };
    result.map(|_| ()).map_err(|e| execution(e.to_string()))
}

/// Whether `text` contains `token` as a whole word (bounded by non-alphanumeric
/// or string boundaries on both sides).
fn contains_token(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let text_bytes = text.as_bytes();
    let token_bytes = token.as_bytes();
    let tlen = token_bytes.len();

    let mut start = 0usize;
    while start + tlen <= text_bytes.len() {
        if let Some(pos) = text[start..].find(token) {
            let abs = start + pos;
            let left_ok = abs == 0 || !text_bytes[abs - 1].is_ascii_alphanumeric();
            let right_ok =
                abs + tlen >= text_bytes.len() || !text_bytes[abs + tlen].is_ascii_alphanumeric();
            if left_ok && right_ok {
                return true;
            }
            start = abs + 1;
        } else {
            break;
        }
    }
    false
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Validate a text field: non-empty (after trim for title; body can be empty),
/// length-capped, and no control characters except `\n`.
fn validate_text_field(value: &str, field: &str, max_chars: usize) -> Result<(), ToolFailure> {
    if value.chars().count() > max_chars {
        return Err(execution(format!(
            "{field} must be at most {max_chars} characters"
        )));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(execution(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(())
}

/// Validate `title` specifically (non-empty after trim).
fn validate_title(title: &str) -> Result<(), ToolFailure> {
    if title.trim().is_empty() {
        return Err(execution("title must not be empty".to_string()));
    }
    validate_text_field(title, "title", MAX_TITLE_CHARS)
}

/// Validate a YYYY-MM-DD date string.
fn validate_date(value: &str) -> Result<(), ToolFailure> {
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || !value
            .chars()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
    {
        return Err(execution(format!(
            "as_of must be YYYY-MM-DD, got {value:?}"
        )));
    }
    Ok(())
}

/// Validate a URL: must start with `http://` or `https://` and contain no
/// control characters.  Minimal validation — deep URL parsing is not in scope.
fn validate_url(url: &str) -> Result<(), ToolFailure> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(execution(format!(
            "source_url must start with http:// or https://, got {url:?}"
        )));
    }
    if url.chars().any(char::is_control) {
        return Err(execution(
            "source_url must not contain control characters".to_string(),
        ));
    }
    Ok(())
}

/// Validate a tag id token: non-empty, `[A-Za-z0-9:._-]+`.
fn validate_tag_id(tag: &str) -> Result<(), ToolFailure> {
    if tag.is_empty()
        || tag
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(execution(format!(
            "invalid tag id {tag:?}: only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

// ── Runtime accessors ─────────────────────────────────────────────────────────

fn require_graph(
    runtime: &KnowledgeRuntime,
) -> Result<&std::sync::Arc<dyn tdw_core::GraphEngine>, ToolFailure> {
    runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))
}

fn require_user_id(runtime: &KnowledgeRuntime) -> Result<&str, ToolFailure> {
    let user_id = runtime
        .bound_user_id()
        .ok_or_else(|| execution("no user identity bound to this finding surface".to_string()))?;
    // Reuse B9 grammar validation: same charset ([A-Za-z0-9:._-]), length, and
    // non-empty checks — the user id is host-bound, but we want the same
    // guarantees before it lands in provenance strings.
    validate_agent_id(user_id).map_err(|e| execution(e.to_string()))?;
    Ok(user_id)
}

// ── Argument helpers ──────────────────────────────────────────────────────────

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
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

// ── FNV-1a hash ───────────────────────────────────────────────────────────────

/// Stable FNV-1a 64-bit hash for deterministic finding ids.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_relations_are_accepted() {
        for rel in VALID_RELS {
            assert!(VALID_RELS.contains(rel), "rel {rel:?} not in vocabulary");
        }
    }

    #[test]
    fn invalid_relation_rejected() {
        // validate_rel is exercised through VALID_RELS check in link_finding
        let bad = "invented_rel";
        assert!(!VALID_RELS.contains(&bad));
    }

    #[test]
    fn validate_text_field_accepts_normal_text() {
        assert!(validate_text_field("hello world", "title", MAX_TITLE_CHARS).is_ok());
        assert!(validate_text_field("line1\nline2", "body", MAX_BODY_CHARS).is_ok());
    }

    #[test]
    fn validate_text_field_rejects_control_chars() {
        // ESC character is a control char that is not \n or \t.
        let with_esc = "hello\x1b[31mred\x1b[0m";
        assert!(validate_text_field(with_esc, "title", MAX_TITLE_CHARS).is_err());
    }

    #[test]
    fn validate_text_field_rejects_overlong() {
        let overlong = "x".repeat(MAX_TITLE_CHARS + 1);
        assert!(validate_text_field(&overlong, "title", MAX_TITLE_CHARS).is_err());
    }

    #[test]
    fn validate_url_accepts_https_and_rejects_bare_path() {
        assert!(validate_url("https://example.com/report").is_ok());
        assert!(validate_url("http://internal/doc").is_ok());
        assert!(validate_url("ftp://old-server/file").is_err());
        assert!(validate_url("not-a-url").is_err());
    }

    #[test]
    fn validate_tag_id_accepts_valid_tokens() {
        assert!(validate_tag_id("sector:tech").is_ok());
        assert!(validate_tag_id("my-tag").is_ok());
        assert!(validate_tag_id("").is_err());
        assert!(validate_tag_id("bad tag").is_err());
        assert!(validate_tag_id("bad;tag").is_err());
    }

    #[test]
    fn validate_date_accepts_yyyy_mm_dd() {
        assert!(validate_date("2026-06-12").is_ok());
        assert!(validate_date("2026-6-12").is_err());
        assert!(validate_date("26-06-12").is_err());
        assert!(validate_date("not-date").is_err());
    }

    #[test]
    fn fnv1a64_is_deterministic() {
        let h1 = fnv1a64(b"AAPL revenue beat");
        let h2 = fnv1a64(b"AAPL revenue beat");
        assert_eq!(h1, h2);
        assert_ne!(h1, fnv1a64(b"different title"));
    }

    #[test]
    fn contains_token_whole_word_matching() {
        // Production code lowercases both text and token before calling this
        // function — test against pre-lowercased inputs to match that contract.
        assert!(contains_token("aapl beat expectations", "aapl"));
        assert!(!contains_token("aaplbeat", "aapl"));
        assert!(contains_token("instrument:aapl beat", "aapl"));
        assert!(!contains_token("", "aapl"));
        assert!(!contains_token("hello world", ""));
        // Colon is non-alphanumeric, so the token boundary is satisfied.
        assert!(contains_token("instrument:aapl", "aapl"));
    }

    // ── compute_age_days ─────────────────────────────────────────────────────
    // The old Julian-day formula omitted the Jan/Feb year-shift, causing
    // off-by-one errors for date pairs that cross a Jan or Feb boundary.
    // These tests verify the corrected chrono-based implementation.

    #[test]
    fn compute_age_days_same_date_is_zero() {
        assert_eq!(compute_age_days("2024-03-15", "2024-03-15"), 0);
    }

    #[test]
    fn compute_age_days_simple_delta() {
        // 10 days apart within a single month, no boundary issues.
        assert_eq!(compute_age_days("2024-03-01", "2024-03-11"), 10);
        // Symmetric: absolute value.
        assert_eq!(compute_age_days("2024-03-11", "2024-03-01"), 10);
    }

    #[test]
    fn compute_age_days_jan_to_feb_span() {
        // Jan 31 → Feb 01 is exactly 1 day.  The old formula returned 0 here
        // because it applied (153*m+2)/5 without shifting Jan/Feb to months
        // 13/14 of the prior year, collapsing the month-length contribution.
        assert_eq!(compute_age_days("2024-01-31", "2024-02-01"), 1);
        // Jan 01 → Feb 01 is 31 days in any year.
        assert_eq!(compute_age_days("2024-01-01", "2024-02-01"), 31);
    }

    #[test]
    fn compute_age_days_cross_year_boundary() {
        // 2023-12-31 → 2024-01-01 is exactly 1 day.
        assert_eq!(compute_age_days("2023-12-31", "2024-01-01"), 1);
        // 2023-11-15 → 2024-02-15 spans the year boundary and two
        // months — 92 days (Nov: 15 remaining, Dec: 31, Jan: 31, Feb: 15).
        assert_eq!(compute_age_days("2023-11-15", "2024-02-15"), 92);
    }

    #[test]
    fn compute_age_days_rfc3339_timestamps_accepted() {
        // The function must accept full RFC 3339 strings, not just bare dates.
        assert_eq!(
            compute_age_days("2024-01-31T00:00:00Z", "2024-02-01T12:00:00Z"),
            1,
        );
    }

    #[test]
    fn compute_age_days_malformed_returns_zero() {
        assert_eq!(compute_age_days("not-a-date", "2024-01-01"), 0);
        assert_eq!(compute_age_days("2024-01-01", ""), 0);
        assert_eq!(compute_age_days("", ""), 0);
    }
}
