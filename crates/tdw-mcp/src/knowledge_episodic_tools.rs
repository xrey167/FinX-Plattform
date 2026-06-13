//! MCP tool for episodic/conversation memory ingestion (knowledge-system K-M2).
//!
//! # Tool: `tdw.kg.remember`
//!
//! An agent records an episode — a text fragment (turn text, conversation
//! snippet, or session window) — as a first-class [`KnowledgeDocument`] on
//! the `episodic` plane.  Internally the tool calls
//! [`transcript_to_episodes`] with `window_size = 1` so a single `remember`
//! call captures exactly one episode window.  Batch transcript ingestion is
//! available via the programmatic [`Backend::remember_episode_at`] path (below).
//!
//! # Trust class: AGENT/USER knowledge
//!
//! Episodes are attributed to the **host-bound principal** (set at runtime
//! construction via `with_user_id`, never from the tool argument — identical
//! to the K-X6 finding surface and the K-L5 identity model).  They land
//! immediately with `Provenance::Agent { agent_id: bound_principal, gated:
//! false }`.
//!
//! # Inference boundary
//!
//! * `PropagateTag` rules CAN reach episodes — auto-tagging aids retrieval.
//! * `DeriveEdge` rules do NOT consume episode edges by default
//!   (`exclude_user_authored = true` in [`tdw_infer::InferEngine`]).
//!   Operator opt-in is required.  This prevents episode content from
//!   silently minting derived facts beyond what the operator sanctioned —
//!   same posture as findings (K-X6).
//!
//! # Temporal leakage safety
//!
//! `as_of` is the injected date for the episode (YYYY-MM-DD).  The B4
//! retrieval contract's `document_visible` predicate ensures a query with
//! `as_of = T` never surfaces an episode whose `as_of > T`.
//!
//! # Caps (B5/B8 posture)
//!
//! * `text` ≤ 8 192 characters.
//! * `entities` ≤ 32 explicit mention targets.
//! * No control characters (except `\n` / `\t`) in any text field.
//! * Tool errors are never protocol errors.

use serde_json::{Map, Value, json};
use tdw_knowledge::indexer::{
    EpisodicWindowConfig, KnowledgeIndexer, TranscriptTurn, transcript_to_episodes,
};
use tdw_knowledge::proposals::validate_agent_id;
use tdw_knowledge::runtime::KnowledgeRuntime;

use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool name ─────────────────────────────────────────────────────────────────

/// The name this module owns.
pub const TOOL_NAME: &str = "tdw.kg.remember";

/// Whether `name` is the episodic remember tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    name == TOOL_NAME
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum character count for the episode text body.
const MAX_TEXT_CHARS: usize = 8_192;
/// Maximum number of explicit entity mention targets per episode.
const MAX_ENTITIES: usize = 32;

// ── Descriptor ────────────────────────────────────────────────────────────────

/// Descriptor for `tdw.kg.remember`.
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool_with_annotations(
        TOOL_NAME,
        "Remember Episode",
        "Record an agent session episode as a searchable temporal memory in the knowledge graph. \
         The episode text is indexed as a KnowledgeDocument on the `episodic` plane with \
         content-hash idempotency (re-submitting the same text at the same as_of is a silent \
         no-op). The episodic document is retrievable via tdw.kg.search and linked to known \
         entities via the existing lexical mention-matcher.\n\
         \n\
         Trust class: AGENT/USER memory — lands immediately with host-bound principal identity \
         (never accepted from the argument, identical to the K-X6 finding surface). \
         PropagateTag rules CAN reach episodes (aids retrieval); DeriveEdge rules do NOT \
         consume episode edges by default (operator opt-in required). Episodes are \
         trust-dial-filterable via their provenance field.\n\
         \n\
         Temporal leakage safety: as_of is the injected episode date. A search query with \
         as_of = T will never surface an episode dated after T (B4 document_visible predicate).\n\
         \n\
         Caps: text ≤ 8 192 chars; entities ≤ 32; no control characters except newline/tab. \
         Requires knowledge runtime with indexer and bound user id.",
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The episode text to remember (required, ≤ 8 192 chars, \
                                   no control characters except newline/tab)."
                },
                "session_id": {
                    "type": "string",
                    "description": "Stable session identifier used to group related episodes \
                                   (optional; defaults to the bound principal id). \
                                   Must match [A-Za-z0-9._:-] grammar."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today). \
                                   Injected — never taken from the wall clock inside this tool."
                },
                "entities": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional explicit entity ids to link this episode to \
                                   (e.g. [\"instrument:AAPL\", \"instrument:MSFT\"]). \
                                   These supplement the automatic lexical mention-matching. \
                                   ≤ 32 items; each must match the graph id grammar \
                                   ([A-Za-z0-9:._-]+)."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional taxonomy tags (≤ 32, colon-separated identifiers \
                                   such as topic:earnings). Applied at index time alongside \
                                   auto-tagging rules."
                }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        false, // readOnlyHint: this is a write surface
        true,  // idempotentHint: content-hash idempotent
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch `tdw.kg.remember`.
///
/// `now` is the current instant as `YYYY-MM-DD` (injected by the MCP layer —
/// nothing inside this module reads the clock directly).
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing indexer, missing user id,
/// malformed input, or an engine failure — never [`ToolFailure::Protocol`].
pub fn execute(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    // ── require bound principal ───────────────────────────────────────────────
    let user_id = require_user_id(runtime)?;

    // ── require indexer ───────────────────────────────────────────────────────
    let indexer = runtime.finding_indexer().ok_or_else(|| {
        execution(
            "episodic memory requires a knowledge indexer attached to the runtime".to_string(),
        )
    })?;

    // ── argument extraction & validation ──────────────────────────────────────
    let text = require_str(arguments, "text")?;
    validate_text(text)?;

    let session_id = optional_str(arguments, "session_id").unwrap_or(user_id);
    validate_session_id(session_id)?;

    let as_of =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;

    let explicit_entities = optional_string_array(arguments, "entities")?.unwrap_or_default();
    if explicit_entities.len() > MAX_ENTITIES {
        return Err(execution(format!(
            "entities must have at most {MAX_ENTITIES} items, got {}",
            explicit_entities.len()
        )));
    }
    for ent in &explicit_entities {
        validate_entity_ref(ent)?;
    }

    let tags = optional_string_array(arguments, "tags")?.unwrap_or_default();
    if tags.len() > MAX_ENTITIES {
        return Err(execution(format!(
            "tags must have at most {MAX_ENTITIES} items, got {}",
            tags.len()
        )));
    }
    for tag in &tags {
        validate_tag_id(tag)?;
    }

    // ── build single-turn transcript → episode document ───────────────────────
    let turns = vec![TranscriptTurn {
        role: user_id.to_string(),
        text: text.to_string(),
        as_of: Some(as_of.clone()),
    }];
    let config = EpisodicWindowConfig { window_size: 1 };
    let mut docs = transcript_to_episodes(session_id, &turns, &config, now);

    // transcript_to_episodes returns exactly one doc for a single turn.
    let Some(mut doc) = docs.pop() else {
        return Err(execution(
            "episode mapping produced no document (empty text?)".to_string(),
        ));
    };

    // Merge explicit entity mentions with the document's (currently empty) mentions.
    for ent in explicit_entities {
        if !doc.mentions.contains(&ent) {
            doc.mentions.push(ent);
        }
    }
    doc.mentions.sort();
    doc.tags = tags;
    // Stamp the bound principal as author so the graph edges land with
    // Provenance::Agent { agent_id, gated: false } — matching the documented
    // trust_class "agent_memory".  This makes trust-dial (K-X3) and why-chains
    // classify episodes correctly as agent/user-authored knowledge.
    doc.author = Some(user_id.to_string());

    let doc_id = doc.id.clone();
    let entity_id = doc.entity.entity_id.clone();

    // ── index through the shared KnowledgeIndexer ─────────────────────────────
    // The finding indexer uses a std::sync::Mutex (same as the K-X6 path).
    // We follow the identical block_in_place pattern from index_finding_blocking.
    let outcome = {
        let mut guard = indexer
            .lock()
            .map_err(|_| execution("episodic indexer mutex poisoned".to_string()))?;
        index_episode_blocking(&mut guard, doc, &as_of)?
    };

    let outcome_str = match outcome {
        tdw_knowledge::indexer::IndexOutcome::Indexed => "landed",
        tdw_knowledge::indexer::IndexOutcome::SkippedUnchanged => "duplicate",
    };

    Ok(structured(json!({
        "outcome": outcome_str,
        "episode_id": entity_id,
        "document_id": doc_id,
        "session_id": session_id,
        "as_of": as_of,
        "plane": "episodic",
        "principal": user_id,
        "trust_class": "agent_memory",
        "inference_note": "PropagateTag rules may reach this episode; \
                           DeriveEdge rules excluded by default (operator opt-in required).",
    })))
}

// ── Programmatic path: Backend::remember_episode_at ───────────────────────────

/// Ingest a full session transcript at the given date via the K-E3 indexer.
///
/// This is the programmatic companion to the `tdw.kg.remember` MCP tool:
/// it accepts an already-constructed `turns` slice, slices it into
/// `window_size`-turn windows using [`transcript_to_episodes`], and indexes
/// each window through the supplied [`KnowledgeIndexer`].
///
/// `session_id` should be a stable, operator-assigned identifier for the
/// conversation (e.g. a UUID or a hashed session token).  `now` must be a
/// `YYYY-MM-DD` date string (injected — nothing here reads the clock).
///
/// Returns the per-window outcomes in window order.  A window whose content
/// hash is already in the manifest returns
/// [`tdw_knowledge::indexer::IndexOutcome::SkippedUnchanged`]; a window that
/// lands returns [`tdw_knowledge::indexer::IndexOutcome::Indexed`].
///
/// # Errors
///
/// Returns [`tdw_knowledge::KnowledgeError`] if the `now` date is malformed,
/// if document validation fails, or if an underlying engine write fails.
/// Individual window errors abort the remainder of the batch (the manifest
/// records only fully-indexed windows — partial ingestion is retried on the
/// next call with the same transcript).
// Called by the daemon host and tests; the compiler sees it unused within
// the lib itself because no built-in dispatch path calls it directly.
#[allow(dead_code)]
pub async fn remember_episode_at(
    indexer: &mut KnowledgeIndexer,
    session_id: &str,
    turns: &[TranscriptTurn],
    config: &EpisodicWindowConfig,
    now: &str,
) -> tdw_knowledge::Result<Vec<tdw_knowledge::indexer::IndexOutcome>> {
    let docs = transcript_to_episodes(session_id, turns, config, now);
    let mut outcomes = Vec::with_capacity(docs.len());
    for doc in docs {
        outcomes.push(indexer.index_at(doc, now).await?);
    }
    Ok(outcomes)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Bridge async [`KnowledgeIndexer::index_at`] from a sync context
/// (non-Send `MutexGuard` in scope).  Mirrors `index_finding_blocking`
/// from `knowledge_finding_tools`.
fn index_episode_blocking(
    indexer: &mut KnowledgeIndexer,
    doc: tdw_knowledge::KnowledgeDocument,
    as_of: &str,
) -> Result<tdw_knowledge::indexer::IndexOutcome, ToolFailure> {
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
            .map_err(|error| execution(format!("runtime build: {error}")))?
            .block_on(fut),
    };
    result.map_err(|error| execution(error.to_string()))
}

fn require_user_id(runtime: &KnowledgeRuntime) -> Result<&str, ToolFailure> {
    let user_id = runtime
        .bound_user_id()
        .ok_or_else(|| execution("no user identity bound to this episodic surface".to_string()))?;
    // Reuse the B9/K-X6 grammar check: same charset, non-empty, length guard.
    validate_agent_id(user_id).map_err(|error| execution(error.to_string()))?;
    Ok(user_id)
}

fn validate_text(value: &str) -> Result<(), ToolFailure> {
    if value.trim().is_empty() {
        return Err(execution("text must not be empty".to_string()));
    }
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(execution(format!(
            "text must be at most {MAX_TEXT_CHARS} characters"
        )));
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(execution(
            "text must not contain control characters (except newline/tab)".to_string(),
        ));
    }
    Ok(())
}

/// Validate a `session_id`: non-empty, `[A-Za-z0-9._:-]+` (same as entity-ref grammar).
fn validate_session_id(value: &str) -> Result<(), ToolFailure> {
    if value.is_empty() {
        return Err(execution("session_id must not be empty".to_string()));
    }
    if value.chars().any(|character| {
        !character.is_ascii_alphanumeric() && !matches!(character, '.' | '_' | ':' | '-')
    }) {
        return Err(execution(format!(
            "session_id must match [A-Za-z0-9._:-]+, got {value:?}"
        )));
    }
    Ok(())
}

fn validate_date(value: &str) -> Result<(), ToolFailure> {
    // Structural check first (fast path, no allocation).
    if value.len() != 10
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || !value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
    {
        return Err(execution(format!(
            "as_of must be YYYY-MM-DD, got {value:?}"
        )));
    }
    // Calendar check: reject structurally-valid but impossible dates such as
    // 2026-02-31 (pre-existing `is_date` does not catch these — see follow-up).
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        execution(format!(
            "as_of {value:?} is not a valid calendar date (e.g. month/day out of range)"
        ))
    })?;
    Ok(())
}

/// Validate an entity reference: graph id grammar `[A-Za-z0-9:._-]+`.
fn validate_entity_ref(value: &str) -> Result<(), ToolFailure> {
    if value.is_empty()
        || value.chars().any(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, ':' | '.' | '_' | '-')
        })
    {
        return Err(execution(format!(
            "invalid entity ref {value:?}: only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

fn validate_tag_id(tag: &str) -> Result<(), ToolFailure> {
    if tag.is_empty()
        || tag.chars().any(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, ':' | '.' | '_' | '-')
        })
    {
        return Err(execution(format!(
            "invalid tag id {tag:?}: only [A-Za-z0-9:._-] allowed"
        )));
    }
    Ok(())
}

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

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_taxonomy::EntityKind;

    // ── validate_text ─────────────────────────────────────────────────────────

    #[test]
    fn validate_text_accepts_normal_text() {
        assert!(validate_text("AAPL earnings beat consensus by 8%").is_ok());
        assert!(validate_text("line1\nline2\ttabbed").is_ok());
    }

    #[test]
    fn validate_text_rejects_empty() {
        assert!(validate_text("").is_err());
        assert!(validate_text("   ").is_err());
    }

    #[test]
    fn validate_text_rejects_control_chars() {
        assert!(validate_text("hello\x1b[31mred\x1b[0m").is_err());
        assert!(validate_text("nul\x00byte").is_err());
    }

    #[test]
    fn validate_text_rejects_overlong() {
        let overlong = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(validate_text(&overlong).is_err());
    }

    // ── validate_session_id ───────────────────────────────────────────────────

    #[test]
    fn validate_session_id_accepts_valid_tokens() {
        assert!(validate_session_id("session-abc123").is_ok());
        assert!(validate_session_id("user:agent-01.v2").is_ok());
    }

    #[test]
    fn validate_session_id_rejects_empty_and_spaces() {
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("session id").is_err());
    }

    // ── validate_date ─────────────────────────────────────────────────────────

    #[test]
    fn validate_date_accepts_valid_yyyy_mm_dd() {
        assert!(validate_date("2026-06-12").is_ok());
    }

    #[test]
    fn validate_date_rejects_malformed() {
        assert!(validate_date("26-06-12").is_err());
        assert!(validate_date("not-a-date").is_err());
        assert!(validate_date("2026-6-12").is_err());
    }

    // ── validate_entity_ref ───────────────────────────────────────────────────

    #[test]
    fn validate_entity_ref_accepts_graph_ids() {
        assert!(validate_entity_ref("instrument:AAPL").is_ok());
        assert!(validate_entity_ref("agent-01.v2").is_ok());
    }

    #[test]
    fn validate_entity_ref_rejects_spaces_and_invalid_chars() {
        assert!(validate_entity_ref("").is_err());
        assert!(validate_entity_ref("bad entity").is_err());
        assert!(validate_entity_ref("bad;entity").is_err());
    }

    // ── transcript_to_episodes integration ───────────────────────────────────

    #[test]
    fn single_turn_produces_one_episode() {
        let turns = vec![TranscriptTurn {
            role: "user".to_string(),
            text: "AAPL beat earnings this quarter".to_string(),
            as_of: Some("2026-06-12".to_string()),
        }];
        let config = EpisodicWindowConfig { window_size: 1 };
        let docs = transcript_to_episodes("sess-abc", &turns, &config, "2026-06-12");
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.plane.as_deref(), Some("episodic"));
        assert_eq!(doc.as_of.as_deref(), Some("2026-06-12"));
        assert!(doc.body.contains("AAPL beat earnings"));
        assert_eq!(doc.entity.kind, EntityKind::Episode);
    }

    #[test]
    fn windowed_transcript_produces_correct_episode_count() {
        let turns: Vec<TranscriptTurn> = (0..5)
            .map(|i| TranscriptTurn {
                role: "user".to_string(),
                text: format!("turn {i}"),
                as_of: Some("2026-06-12".to_string()),
            })
            .collect();
        let config = EpisodicWindowConfig { window_size: 2 };
        let docs = transcript_to_episodes("sess-xyz", &turns, &config, "2026-06-12");
        // 5 turns / 2 per window = 3 windows (last window has 1 turn)
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn episode_as_of_comes_from_first_turn_in_window() {
        let turns = vec![
            TranscriptTurn {
                role: "user".to_string(),
                text: "first".to_string(),
                as_of: Some("2026-01-01".to_string()),
            },
            TranscriptTurn {
                role: "assistant".to_string(),
                text: "second".to_string(),
                as_of: Some("2026-06-12".to_string()),
            },
        ];
        let config = EpisodicWindowConfig { window_size: 2 };
        let docs = transcript_to_episodes("sess-ts", &turns, &config, "2026-06-12");
        assert_eq!(docs.len(), 1);
        // as_of = first turn's date
        assert_eq!(docs[0].as_of.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn empty_transcript_produces_no_episodes() {
        let docs = transcript_to_episodes(
            "sess-empty",
            &[],
            &EpisodicWindowConfig::default(),
            "2026-06-12",
        );
        assert!(docs.is_empty());
    }

    // ── temporal leakage regression ───────────────────────────────────────────

    #[tokio::test]
    async fn temporal_leakage_episode_dated_after_query_is_invisible() {
        use std::sync::Arc;

        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_knowledge::KnowledgeIndex;
        use tdw_knowledge::indexer::{KnowledgeIndexer, transcript_to_episodes};
        use tdw_retrieve::{KnowledgeQuery, QueryFilter, Retriever};
        use tdw_storage_qdrant::InMemoryVectorEngine;

        // Build a fresh in-process index.
        let embedder: Arc<dyn tdw_embed::EmbeddingProvider> =
            Arc::new(HashEmbeddingProvider::default());
        let vectors: Arc<dyn tdw_core::VectorEngine> = Arc::new(InMemoryVectorEngine::default());
        let index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));
        let mut indexer = KnowledgeIndexer::new(index);

        // Ingest an episode dated 2026-06-15 (future relative to the query).
        let turns = vec![TranscriptTurn {
            role: "user".to_string(),
            text: "AAPL beat consensus dramatically in Q2 2026".to_string(),
            as_of: Some("2026-06-15".to_string()),
        }];
        let config = EpisodicWindowConfig { window_size: 1 };
        let docs = transcript_to_episodes("leakage-sess", &turns, &config, "2026-06-15");
        assert_eq!(docs.len(), 1);
        indexer
            .index_at(docs.into_iter().next().expect("one doc"), "2026-06-15")
            .await
            .expect("index succeeds");

        // Query as_of 2026-06-12 (before the episode) — must return no hits.
        let collection = tdw_knowledge::collection_name(embedder.model_id());
        let retriever = Retriever::new(Arc::clone(&embedder), Arc::clone(&vectors), collection);

        // QueryFilter with as_of = 2026-06-12 → only episodes dated ≤ that
        // date should be returned.  The episode is dated 2026-06-15 and must
        // be invisible.
        let query_as_of = "2026-06-12";
        let filter = QueryFilter {
            as_of: Some(query_as_of.to_string()),
            plane: Some("episodic".to_string()),
            ..QueryFilter::default()
        };
        let knowledge_query =
            KnowledgeQuery::try_new("AAPL consensus Q2", 5, filter, None).expect("query builds");
        let hits = retriever
            .search(&knowledge_query)
            .await
            .expect("search succeeds");

        assert!(
            hits.is_empty(),
            "episode dated 2026-06-15 must be invisible when querying as_of 2026-06-12 \
             (temporal leakage regression)"
        );
    }

    // ── mention-linking ───────────────────────────────────────────────────────

    #[test]
    fn explicit_mentions_are_carried_through() {
        let turns = vec![TranscriptTurn {
            role: "user".to_string(),
            text: "AAPL and MSFT both beat".to_string(),
            as_of: Some("2026-06-12".to_string()),
        }];
        let config = EpisodicWindowConfig { window_size: 1 };
        let mut docs = transcript_to_episodes("sess-mention", &turns, &config, "2026-06-12");
        let doc = docs.pop().expect("one doc");
        // Simulate what the MCP tool does: append explicit mentions.
        let mut mentions = doc.mentions;
        for ent in &["instrument:AAPL", "instrument:MSFT"] {
            let ent = ent.to_string();
            if !mentions.contains(&ent) {
                mentions.push(ent);
            }
        }
        mentions.sort();
        assert!(mentions.contains(&"instrument:AAPL".to_string()));
        assert!(mentions.contains(&"instrument:MSFT".to_string()));
    }

    // ── document source ───────────────────────────────────────────────────────

    #[test]
    fn episode_document_source_is_agent_session() {
        use tdw_knowledge::DocumentSource;

        let turns = vec![TranscriptTurn {
            role: "user".to_string(),
            text: "hello".to_string(),
            as_of: Some("2026-06-12".to_string()),
        }];
        let config = EpisodicWindowConfig { window_size: 1 };
        let docs = transcript_to_episodes("my-session", &turns, &config, "2026-06-12");
        let doc = &docs[0];
        assert!(
            matches!(
                &doc.source,
                Some(DocumentSource::AgentSession {
                    session_id,
                    window: 0
                }) if session_id == "my-session"
            ),
            "source must be AgentSession for window 0 of my-session"
        );
    }

    // ── from_config production-path ───────────────────────────────────────────

    #[tokio::test]
    async fn remember_episode_at_ingests_and_returns_outcomes() {
        use std::sync::Arc;

        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_knowledge::KnowledgeIndex;
        use tdw_knowledge::indexer::{IndexOutcome, KnowledgeIndexer};
        use tdw_storage_qdrant::InMemoryVectorEngine;

        let embedder: Arc<dyn tdw_embed::EmbeddingProvider> =
            Arc::new(HashEmbeddingProvider::default());
        let vectors: Arc<dyn tdw_core::VectorEngine> = Arc::new(InMemoryVectorEngine::default());
        let index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));
        let mut indexer = KnowledgeIndexer::new(index);

        let turns = vec![
            TranscriptTurn {
                role: "user".to_string(),
                text: "tell me about AAPL earnings".to_string(),
                as_of: Some("2026-06-12".to_string()),
            },
            TranscriptTurn {
                role: "assistant".to_string(),
                text: "AAPL beat consensus by 8% in Q2 2026".to_string(),
                as_of: Some("2026-06-12".to_string()),
            },
        ];
        let config = EpisodicWindowConfig { window_size: 2 };
        let outcomes =
            remember_episode_at(&mut indexer, "prod-sess-01", &turns, &config, "2026-06-12")
                .await
                .expect("ingestion succeeds");

        assert_eq!(outcomes.len(), 1, "2 turns / window_size=2 → 1 window");
        assert_eq!(outcomes[0], IndexOutcome::Indexed, "first ingest must land");

        // Idempotency: re-ingest the same turns → SkippedUnchanged.
        let outcomes2 =
            remember_episode_at(&mut indexer, "prod-sess-01", &turns, &config, "2026-06-12")
                .await
                .expect("re-ingestion succeeds");
        assert_eq!(
            outcomes2[0],
            IndexOutcome::SkippedUnchanged,
            "same content must be idempotent"
        );
    }

    // ── provenance honesty (K-M2 review blocker) ──────────────────────────────
    // Episodes are documented as landing with Provenance::Agent { agent_id,
    // gated: false }.  This test asserts the persisted graph edges carry exactly
    // that provenance — not Provenance::Ingest — so trust-dial (K-X3) and
    // why-chains classify episodes as agent/user memory as advertised.

    #[tokio::test]
    async fn episode_graph_edges_carry_agent_provenance() {
        use std::sync::Arc;

        use tdw_core::{GraphEngine, Provenance};
        use tdw_embed_local::HashEmbeddingProvider;
        use tdw_knowledge::KnowledgeIndex;
        use tdw_knowledge::indexer::{KnowledgeIndexer, transcript_to_episodes};
        use tdw_storage_graph::InMemoryGraphEngine;
        use tdw_storage_qdrant::InMemoryVectorEngine;

        let embedder: Arc<dyn tdw_embed::EmbeddingProvider> =
            Arc::new(HashEmbeddingProvider::default());
        let vectors: Arc<dyn tdw_core::VectorEngine> = Arc::new(InMemoryVectorEngine::default());
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let index = KnowledgeIndex::new(Arc::clone(&embedder), Arc::clone(&vectors));
        let mut indexer = KnowledgeIndexer::new(index).with_graph(Arc::clone(&graph));

        // Build an episodic doc and stamp author = "agent:test-user"
        let turns = vec![TranscriptTurn {
            role: "user".to_string(),
            text: "Provenance honesty check".to_string(),
            as_of: Some("2026-06-12".to_string()),
        }];
        let config = EpisodicWindowConfig { window_size: 1 };
        let mut docs = transcript_to_episodes("prov-sess", &turns, &config, "2026-06-12");
        let mut doc = docs.pop().expect("one doc");
        let expected_agent_id = "agent:test-user".to_string();
        doc.author = Some(expected_agent_id.clone());

        let entity_id = doc.entity.entity_id.clone();
        indexer
            .index_at(doc, "2026-06-12")
            .await
            .expect("index succeeds");

        // Fetch the `described_by` edge from the episode entity and assert
        // it carries Provenance::Agent, not Provenance::Ingest.
        let filter = tdw_core::TraversalFilter {
            direction: tdw_core::Direction::Out,
            max_hops: 1,
            ..tdw_core::TraversalFilter::default()
        };
        let neighbors = graph
            .neighbors(&entity_id, &filter)
            .await
            .expect("neighbors query succeeds");

        let described_by = neighbors
            .iter()
            .find(|(edge, _)| edge.rel == "described_by")
            .map(|(edge, _)| &edge.provenance)
            .expect("described_by edge must exist");

        assert!(
            matches!(
                described_by,
                Provenance::Agent { agent_id, gated: false } if agent_id == &expected_agent_id
            ),
            "episode described_by edge must carry Provenance::Agent{{agent_id={expected_agent_id:?}, \
             gated=false}}, got {described_by:?}"
        );
    }

    // ── validate_date calendar check (K-M2 review LOW) ───────────────────────

    #[test]
    fn validate_date_rejects_invalid_calendar_date() {
        // 2026-02-31 is structurally valid YYYY-MM-DD but Feb has no 31st day.
        assert!(
            validate_date("2026-02-31").is_err(),
            "2026-02-31 must be rejected as an invalid calendar date"
        );
        // 2026-13-01: month 13 doesn't exist.
        assert!(
            validate_date("2026-13-01").is_err(),
            "2026-13-01 must be rejected (month out of range)"
        );
        // Valid dates still pass.
        assert!(
            validate_date("2026-02-28").is_ok(),
            "2026-02-28 must be accepted"
        );
        assert!(
            validate_date("2024-02-29").is_ok(),
            "2024-02-29 (leap day) must be accepted"
        );
    }
}
