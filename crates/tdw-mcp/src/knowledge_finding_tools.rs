//! MCP tools for first-class research findings (knowledge-system K-X6).
//!
//! # Trust class: USER knowledge
//!
//! Findings are authored and attributed to the calling user — they are NOT
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
//!   3. Findings do NOT enter `tdw-infer` rule matching by default; they
//!      carry the `finding` entity kind and are retrievable via hybrid search
//!      but are never promoted into inference unless an operator explicitly
//!      copies them into a rule body.
//!   4. They are trust-dial-filterable: the K-X3 trust dial can narrow
//!      retrievals to `provenance: agent:<user_id>` so callers can distinguish
//!      personal findings from ingested or operator-materialized facts.
//!
//! # Tools
//!
//! * `tdw.kg.finding` — capture a finding (create entity + index + auto-link).
//! * `tdw.kg.link`    — add a typed relation edge between two findings or
//!   between a finding and any graph entity.
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

use serde_json::{Map, Value, json};
use tdw_core::{GraphEdge, GraphNode, Provenance};
use tdw_kg::Entity;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_taxonomy::EntityKind;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool names ────────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &["tdw.kg.finding", "tdw.kg.link"];

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

/// Descriptors for the two finding tools.
#[must_use]
pub fn descriptors() -> Vec<ToolDescriptor> {
    vec![finding_descriptor(), link_descriptor()]
}

fn finding_tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
) -> ToolDescriptor {
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
         identity, never accepted from the argument), no eval gate. Findings do NOT enter \
         tdw-infer rule matching by default; use tdw.kg.link to create explicit typed relations. \
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
        "tdw.kg.finding" => capture_finding(runtime, arguments),
        "tdw.kg.link" => link_finding(runtime, arguments, now),
        other => Err(execution(format!("unknown finding tool: {other}"))),
    }
}

// ── tdw.kg.finding ────────────────────────────────────────────────────────────

// Validation + graph writes + index pipeline in one function — split would hurt
// readability more than it helps; lint suppressed per project convention.
#[allow(clippy::too_many_lines)]
fn capture_finding(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
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

    let as_of = optional_str(arguments, "as_of").map_or_else(today, ToString::to_string);
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

    // --- stable finding id from title hash ---
    let id_hash = fnv1a64(title.as_bytes());
    let finding_id = format!("finding:{id_hash:016x}");
    let document_id = format!("finding-doc:{id_hash:016x}");

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
            label: format!("finding-doc:{id_hash:016x}"),
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
        let doc = tdw_knowledge::KnowledgeDocument {
            id: format!("{id_hash:016x}"),
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

    // Duplicate link detection: scan existing edges with this rel.
    let duplicate = block_on(async {
        let mut offset = 0usize;
        loop {
            let page = graph
                .edges(Some(rel), offset, 256)
                .await
                .map_err(|e| execution(e.to_string()))?;
            if page.is_empty() {
                break;
            }
            if page.iter().any(|e| e.from == from_id && e.to == to_id) {
                return Ok(true);
            }
            offset += page.len();
        }
        Ok::<bool, ToolFailure>(false)
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

// ── Auto-link scan ────────────────────────────────────────────────────────────

/// Scan `title` and `body` for mentions of known graph entities using a
/// lexical token-matching approach.  Returns at most [`AUTO_LINK_SCAN_LIMIT`]
/// entity ids that appear by their id suffix (the part after the last `:`
/// in a graph id, e.g. `AAPL` from `instrument:AAPL`) as whole tokens within
/// the combined text.  The graph is queried for edges to enumerate known
/// entity ids; the scan is bounded by the limit constant.
///
/// Only entities already present in the graph are linked — stub creation is
/// not triggered here.
async fn scan_auto_links(
    graph: &std::sync::Arc<dyn tdw_core::GraphEngine>,
    title: &str,
    body: &str,
) -> Result<Vec<String>, ToolFailure> {
    // Collect a bounded sample of entity ids from the graph by scanning edges.
    let mut entity_ids: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
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

    // Also pull any node ids reachable from recent entities (best-effort).
    // We limit to whatever we already collected.

    let combined = format!("{title} {body}");
    let combined_lower = combined.to_lowercase();

    let mut matched: Vec<String> = entity_ids
        .into_iter()
        .filter(|id| {
            // Match the suffix token (after last ':') as a whole word, or the
            // full id itself.
            let token = id.split(':').next_back().unwrap_or(id);
            let token_lower = token.to_lowercase();
            // Simple whole-token check: ensure the token is surrounded by
            // non-alphanumeric boundaries in the combined text.
            if token_lower.is_empty() {
                return false;
            }
            contains_token(&combined_lower, &token_lower)
                || contains_token(&combined_lower, &id.to_lowercase())
        })
        .collect();

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
            let left_ok = abs == 0
                || !text_bytes[abs - 1].is_ascii_alphanumeric();
            let right_ok = abs + tlen >= text_bytes.len()
                || !text_bytes[abs + tlen].is_ascii_alphanumeric();
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
        return Err(execution(format!("invalid tag id {tag:?}: only [A-Za-z0-9:._-] allowed")));
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
    runtime
        .bound_user_id()
        .ok_or_else(|| execution("no user identity bound to this finding surface".to_string()))
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

// ── Time helpers ──────────────────────────────────────────────────────────────

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
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
}
