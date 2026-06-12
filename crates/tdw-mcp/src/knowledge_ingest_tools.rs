//! The MCP knowledge INGEST tool (knowledge-system K-E3).
//!
//! `tdw.kg.ingest` accepts a batch of [`KnowledgeDocument`] values and routes
//! them through the daemon-hosted [`KnowledgeIndexer`] seam — the same full
//! B5 write pipeline (content-hash idempotency, auto-tagging rules, lexical
//! co-index, durable graph stamping) used by every other ingest path.
//!
//! # Trust model
//!
//! Ingestion is a **write surface**: it permanently alters the vector index,
//! lexical index, in-process tag store, and durable graph. The tool is
//! therefore exposed only when a [`KnowledgeRuntime`] is attached WITH the
//! knowledge-attachment flag set (mirroring the B9 write-tool gate). Callers
//! that can reach this tool have already crossed the host-defined attachment
//! boundary — the same boundary that controls the B9 proposal tools. The
//! ingest tool does NOT require the proposal queue or operator authority
//! because it does not enqueue proposals; it writes directly through the
//! indexer. Future per-tenant authority checks can be layered at the
//! `KnowledgeIndexer` level without a public-API change here.
//!
//! Agent identity is host-bound: neither `entity_id` nor any provenance field
//! is accepted as a caller-supplied identity claim.
//!
//! # Idempotency
//!
//! Re-submitting a document whose content hash is already in the manifest
//! returns `outcome: "duplicate"` and writes nothing — safe to call
//! multiple times with the same payload.
//!
//! # Caps (B5/B8 posture)
//!
//! - Batch size: ≤ 64 documents per call.
//! - Per-document body: ≤ 131 072 characters.
//! - Per-document mention count: ≤ 64.
//!
//! Tool errors are never protocol errors.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use tdw_core::GraphEngine;
use tdw_infer::{ChangeSet, InferEngine};
use tdw_knowledge::indexer::{IndexOutcome, KnowledgeIndexer};
use tdw_knowledge::{KnowledgeDocument, validate_document};
use tdw_tags::TagEngine;
use tokio::sync::Mutex;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

/// Inference context passed from `McpServer` into `execute` when all three
/// handles are present: the engine lock, the graph, and the tag engine.
/// Using a named tuple keeps `execute`'s signature readable.
pub type InferCtx = (
    Arc<Mutex<InferEngine>>,
    Arc<dyn GraphEngine>,
    Arc<dyn TagEngine>,
);

/// The name this module owns.
pub const TOOL_NAME: &str = "tdw.kg.ingest";

/// Maximum documents per batch call.
const MAX_BATCH_SIZE: usize = 64;
/// Maximum body length per document (characters, not bytes).
const MAX_BODY_CHARS: usize = 131_072;
/// Maximum mention count per document.
const MAX_MENTION_COUNT: usize = 64;

/// Whether `name` is the knowledge ingest tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

/// Descriptor for `tdw.kg.ingest`, appended to `tools/list` only when a
/// [`KnowledgeRuntime`] is attached with the knowledge-attachment flag set.
/// The tool is a write surface (`readOnlyHint: false`), but content-hash
/// idempotent (`idempotentHint: true` — re-submitting the same payload is
/// a no-op reported as `duplicate`).
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool_with_annotations(
        TOOL_NAME,
        "Knowledge Ingest",
        "Batch-ingest KnowledgeDocuments through the daemon-hosted KnowledgeIndexer (K-E3). \
         Each document runs the full B5 write pipeline: content-hash idempotency check \
         (duplicate → no write), auto-tagging rules, vector embedding, lexical co-index, \
         and durable graph stamping. Returns per-document outcomes: \
         `landed` (written), `duplicate` (same content already indexed), or \
         `error` (validation or write failure — does not abort remaining documents). \
         Batch size capped at 64; body ≤ 131 072 chars; mentions ≤ 64 per doc. \
         Write surface — requires knowledge attachment. Idempotent on content hash.",
        json!({
            "type": "object",
            "properties": {
                "documents": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 64,
                    "description": "Batch of KnowledgeDocuments to ingest (1..=64).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Stable document id (identifier grammar: alphanumeric, hyphen, underscore, colon)."
                            },
                            "body": {
                                "type": "string",
                                "description": "Document body text (non-empty, ≤ 131 072 characters)."
                            },
                            "entity": {
                                "type": "object",
                                "description": "The entity this document describes.",
                                "properties": {
                                    "entity_id": { "type": "string" },
                                    "kind": { "type": "string", "description": "Taxonomy kind (e.g. instrument, company, document)." },
                                    "label": { "type": "string" },
                                    "aliases": { "type": "array", "items": { "type": "string" } }
                                },
                                "required": ["entity_id", "kind", "label"],
                                "additionalProperties": false
                            },
                            "tags": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Initial taxonomy tags (rule engine may add more at index time)."
                            },
                            "source": {
                                "type": ["object", "null"],
                                "description": "Ingestion lineage: {kind: manual} | {kind: provider, provider: string} | {kind: news, source: string, url: string}."
                            },
                            "plane": {
                                "type": ["string", "null"],
                                "description": "Knowledge plane: agent | platform | shared."
                            },
                            "as_of": {
                                "type": ["string", "null"],
                                "description": "Effective date YYYY-MM-DD. Undated documents are invisible to temporal queries."
                            },
                            "mentions": {
                                "type": "array",
                                "items": { "type": "string" },
                                "maxItems": 64,
                                "description": "Entity ids mentioned (written as graph edges). ≤ 64."
                            }
                        },
                        "required": ["id", "body", "entity"],
                        "additionalProperties": false
                    }
                },
                "now": {
                    "type": ["string", "null"],
                    "description": "Override the effective date (YYYY-MM-DD) for testing. Omit to use today's UTC date."
                }
            },
            "required": ["documents"],
            "additionalProperties": false
        }),
        false, // readOnlyHint: this is a write surface
        true,  // idempotentHint: content-hash idempotent
    )
}

/// Dispatch `tdw.kg.ingest`. Routes each document through the daemon-hosted
/// [`KnowledgeIndexer`].
///
/// Returns a per-document report:
/// - `landed`: written (new or changed content).
/// - `duplicate`: same content hash already in the manifest — no write.
/// - `error`: validation or write failure for this document.
///
/// When `infer_ctx` is `Some`, `run_incremental` is fired once after the
/// whole batch completes — only when at least one document landed. The
/// `ChangeSet` covers the standard K-E3 edge types (`described_by`,
/// `mentions`) plus every tag declared across all landed documents.
/// Inference errors are logged and never surfaced to the caller (inference
/// is best-effort; the batch is already durable).
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] if the `documents` field is
/// missing/malformed or the batch exceeds the size cap. Individual document
/// failures are reported inline, not as a top-level error.
pub fn execute(
    indexer: &Arc<Mutex<KnowledgeIndexer>>,
    infer_ctx: Option<InferCtx>,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let raw_docs = arguments
        .get("documents")
        .and_then(Value::as_array)
        .ok_or_else(|| execution("documents must be a non-empty array".to_string()))?;

    if raw_docs.is_empty() {
        return Err(execution("documents must not be empty".to_string()));
    }
    if raw_docs.len() > MAX_BATCH_SIZE {
        return Err(execution(format!(
            "batch size {got} exceeds cap {MAX_BATCH_SIZE}",
            got = raw_docs.len()
        )));
    }

    // Deserialize all documents up front — a deserialization failure is a
    // tool error, not a per-document error (the caller sent a malformed batch).
    let docs: Vec<KnowledgeDocument> = raw_docs
        .iter()
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| execution(format!("document deserialization failed: {error}")))
        })
        .collect::<Result<_, _>>()?;

    // Per-document cap validation (before touching any engine).
    validate_doc_caps(&docs)?;

    // The `now` override is for testing; production paths omit it so today's
    // UTC date is used. Tool args are attacker-controlled, so `now` is
    // validated as a date before passing to the indexer.
    let now_override = arguments
        .get("now")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let now = now_override.as_deref().unwrap_or(&today);
    if now.len() != 10 || !now.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err(execution("now must be YYYY-MM-DD if provided".to_string()));
    }

    // Index each document independently: a per-doc failure is reported inline
    // rather than aborting the whole batch. The indexer lock is acquired once
    // around the whole loop so search never observes a half-applied batch.
    //
    // `landed_tags` accumulates tags from every document that actually landed
    // (not duplicates/errors) so the inference ChangeSet below is accurate.
    let mut report: Vec<Value> = Vec::with_capacity(docs.len());
    let mut landed_tags: Vec<String> = Vec::new();
    for doc in docs {
        let doc_id = doc.id.clone();
        let doc_tags = doc.tags.clone();
        // Pre-validate with the public validate_document gate (control-char
        // checks etc.) before handing to the indexer.
        if let Err(error) = validate_document(&doc) {
            report.push(json!({
                "id": doc_id,
                "outcome": "error",
                "error": error.to_string(),
            }));
            continue;
        }
        let outcome = block_on(async {
            let mut guard = indexer.lock().await;
            guard.index_at(doc, now).await
        });
        match outcome {
            Ok(IndexOutcome::Indexed) => {
                landed_tags.extend(doc_tags);
                report.push(json!({ "id": doc_id, "outcome": "landed" }));
            }
            Ok(IndexOutcome::SkippedUnchanged) => {
                report.push(json!({ "id": doc_id, "outcome": "duplicate" }));
            }
            Err(error) => {
                report.push(json!({
                    "id": doc_id,
                    "outcome": "error",
                    "error": error.to_string(),
                }));
            }
        }
    }

    let landed = report
        .iter()
        .filter(|item| item["outcome"] == "landed")
        .count();
    let duplicates = report
        .iter()
        .filter(|item| item["outcome"] == "duplicate")
        .count();
    let errors = report
        .iter()
        .filter(|item| item["outcome"] == "error")
        .count();

    // K-L1: fire incremental inference after the batch when at least one
    // document landed and the inference context is wired up. Inference is
    // best-effort — errors are logged loudly and never surfaced to the caller
    // because the batch is already durable at this point.
    if landed > 0
        && let Some(ctx) = infer_ctx
    {
        fire_infer_after_ingest(ctx, &landed_tags, now);
    }

    Ok(structured(json!({
        "summary": {
            "total": report.len(),
            "landed": landed,
            "duplicate": duplicates,
            "error": errors,
        },
        "documents": report,
    })))
}

/// Validate per-document body-length and mention-count caps.
///
/// Returns a [`ToolFailure`] naming the first offending document so the batch
/// is rejected cleanly before any engine is touched.
fn validate_doc_caps(docs: &[KnowledgeDocument]) -> Result<(), ToolFailure> {
    for doc in docs {
        if doc.body.chars().count() > MAX_BODY_CHARS {
            return Err(execution(format!(
                "document {:?} body exceeds {MAX_BODY_CHARS} character cap",
                doc.id
            )));
        }
        if doc.mentions.len() > MAX_MENTION_COUNT {
            return Err(execution(format!(
                "document {:?} has {} mentions, cap is {MAX_MENTION_COUNT}",
                doc.id,
                doc.mentions.len()
            )));
        }
    }
    Ok(())
}

/// Fire `run_incremental` after a successful ingest batch (K-L1).
///
/// Builds a [`ChangeSet`] from the standard edge types written by the K-E3
/// indexer (`described_by`, `mentions`) plus any tags that landed, then runs
/// the engine. Errors are logged to stderr and never surfaced — the batch is
/// already durable at this point.
fn fire_infer_after_ingest(ctx: InferCtx, landed_tags: &[String], now: &str) {
    let (infer, graph, tags_engine) = ctx;
    let mut changed = ChangeSet::default();
    changed.edge_types.insert("described_by".to_string());
    changed.edge_types.insert("mentions".to_string());
    for tag in landed_tags {
        changed.tags.insert(tag.clone());
    }
    let result = block_on(async {
        let mut guard = infer.lock().await;
        guard
            .run_incremental(&graph, &tags_engine, now, &changed)
            .await
    });
    match result {
        Ok(rep) if rep.derived_edges > 0 || rep.assigned_tags > 0 => {
            eprintln!(
                "tdw-mcp tdw.kg.ingest: inference after batch: derived {} edge(s), \
                 {} tag(s) in {} iteration(s) (rule-set v{})",
                rep.derived_edges, rep.assigned_tags, rep.iterations, rep.version
            );
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!(
                "tdw-mcp tdw.kg.ingest: inference after batch failed (batch already \
                 durable): {error}"
            );
        }
    }
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}
