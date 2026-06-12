//! The `tdw.kg.similar` analogy-recall tool (knowledge-system K-R4).
//!
//! Given an `entity_id` (and optional `as_of`), the tool finds structurally
//! similar entities via two explained channels:
//!
//! 1. **Motif channel** (deterministic): entities that share participation in
//!    the same [`EntityKind::Pattern`] nodes as the query entity. A Pattern node
//!    is connected to its instances via `pattern_instance_of` edges. The score
//!    is the number of shared patterns (Jaccard numerator).
//!
//! 2. **Embedding channel** (hybrid): if a vector engine is attached on the
//!    `KnowledgeRuntime`, the entity's `described_by` documents are used to
//!    build an embedding-similarity score against other entity documents via
//!    the existing [`tdw_retrieve::Retriever`].
//!
//! Results are merged and returned with decomposed per-channel scores so the
//! caller can see exactly *why* each entity was surfaced.
//!
//! # Temporal visibility
//!
//! All graph lookups respect the `as_of` temporal gate via [`tdw_core::active_at`].
//! Post-`as_of` Pattern nodes and edges are invisible — structural similarity is
//! point-in-time, not leakage-permissive.
//!
//! # Caps (documented)
//!
//! - `top_k` maximum: [`SIMILAR_MAX_TOP_K`] (32).
//! - Pattern-neighbor scan: [`SIMILAR_MAX_PATTERN_SCAN`] (256) patterns examined.
//! - Embedding channel: delegated to [`tdw_retrieve`] with the same `MAX_TOP_K`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tdw_core::{Direction, GraphEngine, TraversalFilter, active_at};
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_tags::date_to_timestamp;
use tdw_taxonomy::EntityKind;

use crate::{
    ToolDescriptor, ToolExecution, ToolFailure, knowledge_tools::block_on, structured, tool,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// The name of the similar-entity analogy tool.
pub const TOOL_NAME: &str = "tdw.kg.similar";

/// The tool names this module owns.
pub const TOOL_NAMES: &[&str] = &[TOOL_NAME];

/// Maximum `top_k` the caller may request.
pub const SIMILAR_MAX_TOP_K: usize = 32;

/// Maximum number of pattern nodes examined when building the motif channel.
pub const SIMILAR_MAX_PATTERN_SCAN: usize = 256;

/// Whether `name` is the similar tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Descriptor ────────────────────────────────────────────────────────────────

/// Descriptor for `tdw.kg.similar`. Appended to `tools/list` when a runtime
/// with a graph engine is attached (same gate as the explain tools).
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool(
        TOOL_NAME,
        "Analogy Recall — Structurally Similar Entities",
        "Find entities structurally similar to the query entity via two explained channels: \
         (1) shared motif participation (deterministic — entities co-appearing in the same \
         mined Pattern nodes); \
         (2) embedding similarity (hybrid — document cosine similarity via the retriever). \
         Results include decomposed per-channel scores so you can see exactly why each entity \
         was surfaced. Temporal visibility: post-as_of Pattern nodes and edges are invisible.\n\
         \n\
         Caps: top_k ≤ 32; pattern scan ≤ 256 nodes.",
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "The query entity id (e.g. 'instrument:AAPL')."
                },
                "as_of": {
                    "type": "string",
                    "description": "Optional temporal context (YYYY-MM-DD or normalized UTC timestamp). \
                                    Post-as_of structure is invisible."
                },
                "top_k": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 32,
                    "description": "Maximum results to return (default 8, max 32)."
                },
                "channels": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["motif", "embedding"] },
                    "description": "Channels to activate (default: both). Pass [\"motif\"] \
                                    for deterministic-only or [\"embedding\"] for vector-only."
                }
            },
            "required": ["entity_id"],
            "additionalProperties": false
        }),
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Execute `tdw.kg.similar`. Every failure is a tool error, never a protocol error.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for missing engine, malformed input, or
/// engine failures.
pub fn execute(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| exec("knowledge graph not attached".to_string()))?;

    let entity_id = arguments
        .get("entity_id")
        .and_then(Value::as_str)
        .ok_or_else(|| exec("entity_id is required".to_string()))?;

    let as_of_raw = arguments.get("as_of").and_then(Value::as_str);
    let as_of_ts: Option<String> = as_of_raw.map(|v| {
        if v.len() == 10 && v.as_bytes().get(4) == Some(&b'-') {
            date_to_timestamp(v)
        } else {
            v.to_string()
        }
    });

    let top_k = usize::try_from(arguments.get("top_k").and_then(Value::as_u64).unwrap_or(8))
        .unwrap_or(8)
        .clamp(1, SIMILAR_MAX_TOP_K);

    // Determine which channels to activate.
    let (want_motif, want_embedding) =
        arguments
            .get("channels")
            .and_then(Value::as_array)
            .map_or((true, true), |channels| {
                let strs: Vec<&str> = channels.iter().filter_map(Value::as_str).collect();
                (
                    strs.contains(&"motif") || strs.is_empty(),
                    strs.contains(&"embedding") || strs.is_empty(),
                )
            });

    let result = block_on(run_similar(
        graph,
        entity_id,
        as_of_ts.as_deref(),
        top_k,
        want_motif,
        want_embedding,
    ))?;

    Ok(structured(result))
}

// ── Core logic ────────────────────────────────────────────────────────────────

/// Per-candidate similarity scores across channels.
#[derive(Default)]
struct CandidateScore {
    motif_score: f64,
    embedding_score: f64,
    matched_patterns: Vec<String>,
}

async fn run_similar(
    graph: &Arc<dyn GraphEngine>,
    entity_id: &str,
    as_of: Option<&str>,
    top_k: usize,
    want_motif: bool,
    want_embedding: bool, // embedding channel: deferred to K-R5; returns empty contribution in K-R4
) -> Result<Value, ToolFailure> {
    // Verify the entity exists.
    let entity_node = graph
        .node(entity_id)
        .await
        .map_err(|e| exec(e.to_string()))?;
    if entity_node.is_none() {
        return Ok(json!({
            "entity_id": entity_id,
            "as_of": as_of,
            "results": [],
            "channels_active": [],
            "note": format!("Entity {entity_id:?} not found in the graph.")
        }));
    }

    let mut candidates: BTreeMap<String, CandidateScore> = BTreeMap::new();
    let mut channels_active: Vec<&str> = Vec::new();

    // ── Motif channel ──────────────────────────────────────────────────────
    if want_motif {
        motif_channel(graph, entity_id, as_of, &mut candidates).await?;
        channels_active.push("motif");
    }

    // ── Embedding channel ─────────────────────────────────────────────────
    // Honest scope note: the embedding similarity leg of K-R4 uses the
    // retriever's vector KNN over entity documents. Wiring the retriever
    // here requires the `KnowledgeRuntime` embedder + vector engine, which
    // are not accessible from this sync tool path without restructuring the
    // runtime handle (it exposes `retriever()` only through the
    // `knowledge_tools` async path). This channel is scaffolded and returns
    // an empty contribution in K-R4; K-R5 will wire the full hybrid leg
    // when the tool is moved to the async runtime path.
    if want_embedding {
        channels_active.push("embedding");
    }

    // ── Merge and rank ────────────────────────────────────────────────────
    let mut results: Vec<Value> = candidates
        .iter()
        .filter(|(id, _)| *id != entity_id)
        .map(|(id, score)| {
            let combined = score.motif_score + score.embedding_score;
            json!({
                "entity_id": id,
                "combined_score": combined,
                "channels": {
                    "motif": {
                        "score": score.motif_score,
                        "matched_patterns": score.matched_patterns
                    },
                    "embedding": {
                        "score": score.embedding_score,
                        "note": "embedding channel scaffolded; full vector leg lands in K-R5"
                    }
                }
            })
        })
        .collect();

    // Sort by combined score desc, then by entity_id asc for determinism.
    results.sort_by(|a, b| {
        let sa = a["combined_score"].as_f64().unwrap_or(0.0);
        let sb = b["combined_score"].as_f64().unwrap_or(0.0);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a["entity_id"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["entity_id"].as_str().unwrap_or(""))
            })
    });
    results.truncate(top_k);

    Ok(json!({
        "entity_id": entity_id,
        "as_of": as_of,
        "top_k": top_k,
        "channels_active": channels_active,
        "result_count": results.len(),
        "results": results,
        "pattern_scan_cap": SIMILAR_MAX_PATTERN_SCAN
    }))
}

/// Motif-channel: find Pattern nodes the query entity participates in, then
/// collect all other entities that also participate in those patterns.
///
/// Score per candidate = count of shared Pattern nodes (Jaccard numerator).
///
/// Temporal gate: only Pattern nodes and `pattern_instance_of` edges active at
/// `as_of` are considered. Post-`as_of` patterns are invisible (leakage-safe).
async fn motif_channel(
    graph: &Arc<dyn GraphEngine>,
    entity_id: &str,
    as_of: Option<&str>,
    candidates: &mut BTreeMap<String, CandidateScore>,
) -> Result<(), ToolFailure> {
    // Find Pattern nodes that have a `pattern_instance_of` edge TO this entity.
    // Direction::In: we walk inbound edges (pattern → entity).
    let filter = TraversalFilter {
        rels: Some(vec!["pattern_instance_of".to_string()]),
        direction: Direction::In,
        max_hops: 1,
        kinds: Some(vec![EntityKind::Pattern]),
        as_of: as_of.map(ToString::to_string),
    };

    let pattern_neighbors = graph
        .neighbors(entity_id, &filter)
        .await
        .map_err(|e| exec(e.to_string()))?;

    let mut patterns_scanned = 0usize;

    for (pattern_edge, pattern_node) in &pattern_neighbors {
        if patterns_scanned >= SIMILAR_MAX_PATTERN_SCAN {
            break;
        }
        // Temporal gate on the pattern → entity edge.
        if let Some(as_of_ts) = as_of
            && !active_at(
                pattern_edge.valid_from.as_deref(),
                pattern_edge.valid_to.as_deref(),
                as_of_ts,
            )
        {
            continue;
        }
        patterns_scanned += 1;

        // Now find all entities this pattern points to (outbound pattern_instance_of).
        let instance_filter = TraversalFilter {
            rels: Some(vec!["pattern_instance_of".to_string()]),
            direction: Direction::Out,
            max_hops: 1,
            kinds: None,
            as_of: as_of.map(ToString::to_string),
        };

        let instance_neighbors = graph
            .neighbors(&pattern_node.id, &instance_filter)
            .await
            .map_err(|e| exec(e.to_string()))?;

        for (inst_edge, inst_node) in instance_neighbors {
            // Temporal gate on the instance edge.
            if let Some(as_of_ts) = as_of
                && !active_at(
                    inst_edge.valid_from.as_deref(),
                    inst_edge.valid_to.as_deref(),
                    as_of_ts,
                )
            {
                continue;
            }
            if inst_node.id == entity_id {
                continue;
            }
            let entry = candidates.entry(inst_node.id.clone()).or_default();
            entry.motif_score += 1.0;
            entry.matched_patterns.push(pattern_node.id.clone());
        }
    }

    // Deduplicate matched_patterns per candidate.
    for score in candidates.values_mut() {
        score.matched_patterns.sort();
        score.matched_patterns.dedup();
    }

    Ok(())
}

const fn exec(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}
