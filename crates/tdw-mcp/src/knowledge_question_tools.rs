//! MCP tools for standing open questions (knowledge-system K-X8).
//!
//! # What this module implements
//!
//! An **open question** is a first-class "I need to find out X" node that parks
//! an analyst's unresolved question alongside match criteria.  The cron-driven
//! matching engine checks newly-arrived facts against all open questions and fires
//! a `tdw-alerts`-style alert (via `eprintln!`, the honest production path) when
//! a candidate match is found.  On resolution the answering fact is recorded;
//! dismissal as "checked absent" writes a queryable **negative-knowledge** assertion.
//!
//! # Tools
//!
//! | Tool | Action |
//! |------|--------|
//! | `tdw.kg.ask`        | Park a new open question with optional match criteria. |
//! | `tdw.kg.resolve`    | Resolve a question (record the answering fact). |
//! | `tdw.kg.dismiss`    | Dismiss as "checked absent" → negative-knowledge assertion. |
//! | `tdw.kg.questions`  | List the calling principal's open questions. |
//!
//! # Matching engine
//!
//! The cron task ([`tick_question_check`]) is called by `tdw-backend` on a
//! configured cadence.  For each open question it:
//!
//! 1. Runs the **deterministic primary match**: if `match_entity_id` is set,
//!    checks whether any new edge from/to that entity appeared since
//!    `last_checked_as_of`.
//! 2. Runs the **deterministic secondary match**: if `match_tag` is set, checks
//!    whether the target entity gained that tag since `last_checked_as_of`.
//! 3. Runs the **deterministic tertiary match**: if `match_predicate` is set,
//!    checks whether any new edge with that relation appeared near the target
//!    entity since `last_checked_as_of`.
//! 4. On ANY candidate match: fires a `[tdw] QUESTION-MATCH` alert (same
//!    `eprintln!` posture as watchlists) with the question text and matching fact.
//!    Advances `last_checked_as_of` for dedup (K-X5 precedent).
//! 5. If zero open questions exist: logs a note and returns 0 — no spurious work.
//!
//! # Negative knowledge (dismiss path)
//!
//! When a question is dismissed with `tdw.kg.dismiss`, a `checked_absent` edge
//! is written from the `OpenQuestion` node to the `match_entity_id` target (when
//! set).  The edge carries:
//! * `provenance: Provenance::Agent { agent_id: user_id, gated: false }` — user
//!   attribution so the assertion is filterable.
//! * `props.reason` — the dismissal note.
//! * `props.dismissed_as_of` — the dismissal date.
//!
//! This makes the negative assertion queryable via `tdw.kg.traverse` so future
//! extraction and `tdw.kg.ask` can check "has this already been verified absent?"
//! before re-asking.
//!
//! # Trust class
//!
//! Same as `Finding` (K-X6 / K-X7): user provenance, host-bound identity
//! (K-L5), instant-write, `DeriveEdge` rules do NOT consume `OpenQuestion` edges
//! by default.
//!
//! # Persistence
//!
//! The `OpenQuestionStore` follows the `WatchlistStore` caller-owns/JSON-file
//! precedent.  When `TDW_QUESTIONS_DIR` is set, mutations are persisted to
//! `<dir>/questions.json` after every write.  Unset → in-memory only.

#![allow(clippy::too_many_lines)] // cohesive cron tick + tool dispatch

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tdw_core::{Direction, GraphEngine, TraversalFilter, active_at};
use tdw_knowledge::runtime::{KnowledgeRuntime, QuestionsFreshness};
use tdw_taxonomy::EntityKind;

use crate::knowledge_tools::block_on;
use crate::{ToolDescriptor, ToolExecution, ToolFailure, structured, tool_with_annotations};

// ── Tool names ────────────────────────────────────────────────────────────────

/// The names this module owns.
pub const TOOL_NAMES: &[&str] = &[
    "tdw.kg.ask",
    "tdw.kg.resolve",
    "tdw.kg.dismiss",
    "tdw.kg.questions",
];

/// Whether `name` is one of the question tools.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Caps ──────────────────────────────────────────────────────────────────────

/// Maximum character count for a question statement.
pub const MAX_QUESTION_CHARS: usize = 256;
/// Maximum character count for a resolution/dismissal note.
pub const MAX_NOTE_CHARS: usize = 1_024;
/// Maximum number of open questions per principal.
pub const MAX_QUESTIONS_PER_PRINCIPAL: usize = 128;
/// Maximum edge scan window for the matching engine per question per tick.
const MATCH_ENGINE_EDGE_CAP: usize = 512;

// ── Domain model ──────────────────────────────────────────────────────────────

/// Status of an open question.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    /// Awaiting a matching fact.
    Open,
    /// A matching fact was found and recorded.
    Resolved,
    /// Verified as absent — negative-knowledge written.
    Dismissed,
}

impl QuestionStatus {
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Dismissed => "dismissed",
        }
    }
}

/// A persisted open question entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionEntry {
    /// Stable, unique identifier for this question (FNV-1a hex of
    /// `"<principal_id>:<question_text>"`).
    pub question_id: String,
    /// The principal (user id) who parked this question.
    pub principal_id: String,
    /// The question text (≤ 256 chars).
    pub question: String,
    /// Deterministic primary match: entity whose neighborhood is scanned.
    #[serde(default)]
    pub match_entity_id: Option<String>,
    /// Deterministic secondary match: tag the answering fact must carry.
    #[serde(default)]
    pub match_tag: Option<String>,
    /// Deterministic tertiary match: relation name the answering fact must use.
    #[serde(default)]
    pub match_predicate: Option<String>,
    /// Optional free-text semantic anchor for future embedder matching.
    #[serde(default)]
    pub semantic_anchor: Option<String>,
    /// Current status.
    pub status: QuestionStatus,
    /// Creation date (`YYYY-MM-DD`).
    pub created_as_of: String,
    /// Last time the matching engine checked this question (`YYYY-MM-DD`).
    /// Starts equal to `created_as_of`.
    pub last_checked_as_of: String,
    /// Running count of candidate-match alerts fired for this question.
    pub matches_fired: u64,
    /// Id of the fact/finding that resolved this question (set on resolution).
    #[serde(default)]
    pub resolved_by: Option<String>,
    /// Note written when resolving or dismissing.
    #[serde(default)]
    pub resolution_note: Option<String>,
}

impl QuestionEntry {
    /// True when this question is still active (open).
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.status, QuestionStatus::Open)
    }
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// In-memory open-question store.
#[derive(Debug, Default)]
pub struct OpenQuestionStore {
    /// Keyed by `question_id`.
    entries: BTreeMap<String, QuestionEntry>,
    /// Cumulative match alerts fired across all questions.
    pub total_matches_fired: u64,
    /// Unix-epoch ms of the last cron tick.  `0` means never checked.
    pub last_check_ms: i64,
}

impl OpenQuestionStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace an entry.
    pub fn upsert(&mut self, entry: QuestionEntry) {
        self.entries.insert(entry.question_id.clone(), entry);
    }

    /// Remove an entry by id.  Returns `true` when it existed.
    pub fn remove(&mut self, question_id: &str) -> bool {
        self.entries.remove(question_id).is_some()
    }

    /// All entries for a given principal, sorted by creation date.
    #[must_use]
    pub fn by_principal(&self, principal_id: &str) -> Vec<&QuestionEntry> {
        let mut entries: Vec<&QuestionEntry> = self
            .entries
            .values()
            .filter(|e| e.principal_id == principal_id)
            .collect();
        entries.sort_by(|a, b| a.created_as_of.cmp(&b.created_as_of));
        entries
    }

    /// Count of entries (all statuses) for a principal.
    #[must_use]
    pub fn count_for_principal(&self, principal_id: &str) -> usize {
        self.entries
            .values()
            .filter(|e| e.principal_id == principal_id)
            .count()
    }

    /// All open (non-resolved, non-dismissed) entries.
    #[must_use]
    pub fn all_open(&self) -> Vec<QuestionEntry> {
        self.entries
            .values()
            .filter(|e| e.is_open())
            .cloned()
            .collect()
    }

    /// Total count across all statuses and principals.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }

    /// Count of open (awaiting answer) questions across all principals.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.entries.values().filter(|e| e.is_open()).count()
    }

    /// Update `last_checked_as_of` and increment `matches_fired`.
    pub fn record_match(&mut self, question_id: &str, as_of: &str) {
        if let Some(entry) = self.entries.get_mut(question_id) {
            entry.last_checked_as_of = as_of.to_string();
            entry.matches_fired = entry.matches_fired.saturating_add(1);
            self.total_matches_fired = self.total_matches_fired.saturating_add(1);
        }
    }

    /// Advance `last_checked_as_of` without firing a match (dedup advance).
    pub fn advance_checked(&mut self, question_id: &str, as_of: &str) {
        if let Some(entry) = self.entries.get_mut(question_id) {
            entry.last_checked_as_of = as_of.to_string();
        }
    }

    /// Persist to `path` as JSON.
    ///
    /// The (cheap, CPU-bound) serialization runs inline so the `&self` borrow is
    /// not held across the offloaded I/O.  The blocking write-temp-then-rename is
    /// offloaded to [`tokio::task::spawn_blocking`] when an ambient tokio runtime
    /// exists, so it never stalls an async worker thread (this is reached from
    /// the async `tick_question_check` cron path).  When there is no ambient
    /// runtime (plain sync callers, unit tests) it falls back to a direct sync
    /// write+rename — same atomic temp-then-rename pattern either way.
    ///
    /// # Errors
    ///
    /// Returns an error string if serialization or file I/O fails.
    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.entries)
            .map_err(|e| format!("questions serialize failed: {e}"))?;
        let path = path.to_path_buf();
        match tokio::runtime::Handle::try_current() {
            // Ambient runtime present: do not block the async worker — offload
            // the write+rename to the blocking pool and wait for it.
            Ok(handle) => tokio::task::block_in_place(|| {
                handle.block_on(async {
                    tokio::task::spawn_blocking(move || Self::write_atomic(&path, &json))
                        .await
                        .map_err(|e| format!("questions persist task join failed: {e}"))?
                })
            }),
            // No ambient runtime (sync callers, unit tests): direct sync I/O.
            Err(_) => Self::write_atomic(&path, &json),
        }
    }

    /// Atomic write: serialize-to-temp then rename over the destination.
    ///
    /// # Errors
    ///
    /// Returns an error string if either the temp write or the rename fails.
    fn write_atomic(path: &Path, json: &str) -> Result<(), String> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, json.as_bytes())
            .map_err(|e| format!("questions write to {} failed: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            format!(
                "questions rename {} → {} failed: {e}",
                tmp.display(),
                path.display()
            )
        })?;
        Ok(())
    }

    /// Load from `path`.  Missing file → empty store.  Corrupt → empty store
    /// with a loud warning.
    #[must_use]
    pub fn load_from_file(path: &Path) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!(
                    "[tdw] WARN: could not read questions file {}: {e} — starting empty",
                    path.display()
                );
                return Self::default();
            }
        };
        match serde_json::from_str::<BTreeMap<String, QuestionEntry>>(&contents) {
            Ok(entries) => Self {
                entries,
                total_matches_fired: 0,
                last_check_ms: 0,
            },
            Err(e) => {
                eprintln!(
                    "[tdw] WARN: questions file {} corrupt ({e}) — starting empty",
                    path.display()
                );
                Self::default()
            }
        }
    }
}

/// Environment variable naming the directory for question persistence.
pub const TDW_QUESTIONS_DIR_ENV: &str = "TDW_QUESTIONS_DIR";

/// Canonical path for the questions JSON file inside `dir`.
#[must_use]
pub fn questions_file_path(dir: &Path) -> PathBuf {
    dir.join("questions.json")
}

/// Load from `TDW_QUESTIONS_DIR` when set; empty store otherwise.
#[must_use]
pub fn load_store_from_env() -> OpenQuestionStore {
    match std::env::var(TDW_QUESTIONS_DIR_ENV) {
        Ok(dir) if !dir.trim().is_empty() => {
            let path = questions_file_path(Path::new(dir.trim()));
            eprintln!(
                "[tdw] open questions: loading persisted state from {}",
                path.display()
            );
            OpenQuestionStore::load_from_file(&path)
        }
        _ => OpenQuestionStore::default(),
    }
}

/// Persist to `TDW_QUESTIONS_DIR` when set; no-op otherwise.
pub fn save_store_to_env(store: &OpenQuestionStore) {
    if let Ok(dir) = std::env::var(TDW_QUESTIONS_DIR_ENV) {
        if dir.trim().is_empty() {
            return;
        }
        let path = questions_file_path(Path::new(dir.trim()));
        if let Err(e) = store.save_to_file(&path) {
            eprintln!("[tdw] ERROR: question store persist failed: {e}");
        }
    }
}

// ── Descriptors ───────────────────────────────────────────────────────────────

/// Descriptors for the four question tools.
#[must_use]
pub(crate) fn descriptors() -> Vec<ToolDescriptor> {
    vec![
        ask_descriptor(),
        resolve_descriptor(),
        dismiss_descriptor(),
        questions_descriptor(),
    ]
}

fn ask_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.ask",
        "Park Open Question",
        "Park a standing open question in the knowledge graph so the matching engine can alert \
         you when incoming facts appear to answer it.  The question is stored as an \
         `OpenQuestion` node with optional deterministic match criteria — entity id, tag, and/or \
         predicate — that the cron engine checks on every tick.\n\
         \n\
         Match criteria (all optional; at least one is recommended for deterministic matching):\n\
         * `match_entity_id` — alert when new edges appear near this entity.\n\
         * `match_tag`       — alert when an entity gains or loses this tag.\n\
         * `match_predicate` — alert when a new edge with this relation name appears.\n\
         * `semantic_anchor` — free-text phrase for future embedder-powered matching \
           (stored; not yet active in the matching engine).\n\
         \n\
         Trust class: USER knowledge — same as `tdw.kg.finding`: user provenance, host-bound \
         identity (K-L5), instant-write. Caps: question ≤ 256 chars; no control characters; \
         at most 128 open questions per principal.\n\
         \n\
         Use `tdw.kg.resolve` to record the answer or `tdw.kg.dismiss` to mark it as \
         checked-absent (which writes a queryable negative-knowledge assertion).",
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question text (≤ 256 chars, no control characters). Required."
                },
                "match_entity_id": {
                    "type": "string",
                    "description": "Graph entity id the answer is expected to be about \
                                   (e.g. instrument:AAPL). Triggers deterministic edge-scan matching."
                },
                "match_tag": {
                    "type": "string",
                    "description": "Tag id the answering fact must carry (e.g. sector:tech). \
                                   Triggers deterministic tag-gain matching."
                },
                "match_predicate": {
                    "type": "string",
                    "description": "Relation name the answering fact must use (e.g. ceo_of). \
                                   Triggers deterministic predicate-scan matching."
                },
                "semantic_anchor": {
                    "type": "string",
                    "description": "Free-text phrase for future embedder-powered matching \
                                   (stored but not yet active in the matching engine)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today)."
                }
            },
            "required": ["question"],
            "additionalProperties": false
        }),
        false, // not read-only
        false, // not idempotent (creates a new entry)
    )
}

fn resolve_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.resolve",
        "Resolve Open Question",
        "Mark an open question as resolved: record the fact or finding that answered it. \
         Stores `resolved_by` (the answering fact id) and `resolution_note` on the question \
         node and transitions its status to `resolved`. The question remains in the graph and \
         is queryable via `tdw.kg.traverse` from the `OpenQuestion` node.\n\
         \n\
         Use `tdw.kg.dismiss` instead when the question was checked and the answer is \
         definitively absent — that path writes a queryable negative-knowledge assertion.\n\
         \n\
         Requires graph engine + bound user id.",
        json!({
            "type": "object",
            "properties": {
                "question_id": {
                    "type": "string",
                    "description": "The question node id (e.g. openquestion:abc123)."
                },
                "resolved_by": {
                    "type": "string",
                    "description": "Id of the fact, finding, or entity that answers this question."
                },
                "note": {
                    "type": "string",
                    "description": "Optional resolution note (≤ 1 024 chars, no control characters)."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today)."
                }
            },
            "required": ["question_id", "resolved_by"],
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn dismiss_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.dismiss",
        "Dismiss Open Question (Negative Knowledge)",
        "Dismiss an open question as definitively checked and absent. Transitions the \
         question's status to `dismissed` and, when `match_entity_id` is set on the \
         question, writes a `checked_absent` edge from the `OpenQuestion` node to the \
         target entity.  This edge is:\n\
         * Queryable via `tdw.kg.traverse` — future asks and extraction can check \
           `checked_absent` edges before re-asking the same question.\n\
         * Provenance-attributed — user identity is host-bound (K-L5) so the assertion \
           is trust-dial-filterable.\n\
         * Temporally stamped — `props.dismissed_as_of` records when the absence was \
           verified so the assertion ages naturally.\n\
         \n\
         This is the negative-knowledge path: the dismissed question prevents both \
         re-asking and re-derivation of the same false conclusion.\n\
         \n\
         Requires graph engine + bound user id.",
        json!({
            "type": "object",
            "properties": {
                "question_id": {
                    "type": "string",
                    "description": "The question node id (e.g. openquestion:abc123)."
                },
                "note": {
                    "type": "string",
                    "description": "Required dismissal reason (≤ 1 024 chars, no control characters). \
                                   Explains why this is definitively absent."
                },
                "as_of": {
                    "type": "string",
                    "description": "Effective date YYYY-MM-DD (optional, defaults to today)."
                }
            },
            "required": ["question_id", "note"],
            "additionalProperties": false
        }),
        false,
        false,
    )
}

fn questions_descriptor() -> ToolDescriptor {
    tool_with_annotations(
        "tdw.kg.questions",
        "List Open Questions",
        "List open questions for the calling principal. Returns the question id, text, match \
         criteria, status, creation date, last-check date, and candidate-match count. \
         Includes resolved and dismissed questions (with their outcomes) so the full history \
         is visible.\n\
         \n\
         Read-only. Idempotent. Requires bound user id.",
        json!({
            "type": "object",
            "properties": {
                "status_filter": {
                    "type": "string",
                    "enum": ["open", "resolved", "dismissed", "all"],
                    "description": "Which statuses to include. Defaults to 'open'."
                }
            },
            "additionalProperties": false
        }),
        true, // readOnlyHint
        true, // idempotentHint
    )
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch one question tool.
///
/// `store` is the shared question store.
/// `runtime` supplies the graph engine and bound user identity.
/// `now` is the current date `YYYY-MM-DD` injected by the MCP layer.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for missing identity, malformed input,
/// cap violations, or engine failures — never [`ToolFailure::Protocol`].
pub(crate) fn execute(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<OpenQuestionStore>>,
    name: &str,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.ask" => ask(runtime, store, arguments, now),
        "tdw.kg.resolve" => resolve(runtime, store, arguments, now),
        "tdw.kg.dismiss" => dismiss(runtime, store, arguments, now),
        "tdw.kg.questions" => questions(runtime, store, arguments),
        other => Err(execution(format!("unknown question tool: {other}"))),
    }
}

// ── tdw.kg.ask ────────────────────────────────────────────────────────────────

fn ask(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<OpenQuestionStore>>,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let principal_id = require_user_id(runtime)?;

    let question = require_str(arguments, "question")?;
    validate_text_field(question, "question", MAX_QUESTION_CHARS)?;
    if question.trim().is_empty() {
        return Err(execution(
            "question must not be empty after trim".to_string(),
        ));
    }

    let match_entity_id = optional_str(arguments, "match_entity_id").map(ToString::to_string);
    let match_tag = optional_str(arguments, "match_tag").map(ToString::to_string);
    let match_predicate = optional_str(arguments, "match_predicate").map(ToString::to_string);
    let semantic_anchor = optional_str(arguments, "semantic_anchor").map(ToString::to_string);

    // Validate any graph ids supplied as criteria.
    if let Some(eid) = &match_entity_id {
        validate_graph_id(eid, "match_entity_id")?;
    }
    if let Some(tag) = &match_tag {
        validate_graph_id(tag, "match_tag")?;
    }
    if let Some(pred) = &match_predicate {
        validate_graph_id(pred, "match_predicate")?;
    }

    let as_of =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;

    // Stable question id: FNV-1a of "<principal_id>:<question_text>".
    let question_id_hex = format!(
        "{:016x}",
        fnv1a64(format!("{principal_id}:{question}").as_bytes())
    );
    let question_id = format!("openquestion:{question_id_hex}");
    let as_of_ts = tdw_tags::date_to_timestamp(&as_of);

    // Write the OpenQuestion node to the graph.
    block_on(async {
        let mut props = json!({
            "question": question,
            "user_id": principal_id,
            "as_of": as_of_ts,
            "status": "open",
        });
        if let Some(ref eid) = match_entity_id {
            props["match_entity_id"] = json!(eid);
        }
        if let Some(ref tag) = match_tag {
            props["match_tag"] = json!(tag);
        }
        if let Some(ref pred) = match_predicate {
            props["match_predicate"] = json!(pred);
        }
        if let Some(ref anchor) = semantic_anchor {
            props["semantic_anchor"] = json!(anchor);
        }

        graph
            .upsert_nodes(vec![tdw_core::GraphNode {
                id: question_id.clone(),
                kind: EntityKind::OpenQuestion,
                label: question.to_string(),
                aliases: Vec::new(),
                props,
                valid_from: Some(as_of_ts),
                valid_to: None,
            }])
            .await
            .map_err(|e| execution(e.to_string()))
    })?;

    // Write to the in-memory store.
    let entry = QuestionEntry {
        question_id: question_id.clone(),
        principal_id: principal_id.to_string(),
        question: question.to_string(),
        match_entity_id: match_entity_id.clone(),
        match_tag: match_tag.clone(),
        match_predicate: match_predicate.clone(),
        semantic_anchor,
        status: QuestionStatus::Open,
        created_as_of: as_of.clone(),
        last_checked_as_of: as_of.clone(),
        matches_fired: 0,
        resolved_by: None,
        resolution_note: None,
    };
    {
        let mut guard = store
            .lock()
            .map_err(|e| execution(format!("question store mutex poisoned: {e}")))?;
        if guard.count_for_principal(principal_id) >= MAX_QUESTIONS_PER_PRINCIPAL {
            return Err(execution(format!(
                "principal {principal_id:?} has reached the maximum of \
                 {MAX_QUESTIONS_PER_PRINCIPAL} questions; resolve or dismiss old ones first"
            )));
        }
        guard.upsert(entry);
        save_store_to_env(&guard);
        drop(guard);
    }

    Ok(structured(json!({
        "question_id": question_id,
        "question": question,
        "status": "open",
        "as_of": as_of,
        "match_entity_id": match_entity_id,
        "match_tag": match_tag,
        "match_predicate": match_predicate,
        "note": "Question parked — matching engine will alert you on the configured cadence \
                 when a candidate fact is found. Use tdw.kg.resolve to record the answer \
                 or tdw.kg.dismiss to mark it as checked-absent.",
    })))
}

// ── tdw.kg.resolve ────────────────────────────────────────────────────────────

fn resolve(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<OpenQuestionStore>>,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let principal_id = require_user_id(runtime)?;

    let question_id = require_str(arguments, "question_id")?;
    let resolved_by = require_str(arguments, "resolved_by")?;
    let note = optional_str(arguments, "note");
    if let Some(n) = note {
        validate_text_field(n, "note", MAX_NOTE_CHARS)?;
    }
    let as_of =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;

    let as_of_ts = tdw_tags::date_to_timestamp(&as_of);

    // Update the OpenQuestion node in the graph.
    let question_node = block_on(async {
        graph
            .node(question_id)
            .await
            .map_err(|e| execution(e.to_string()))
    })?
    .ok_or_else(|| execution(format!("question {question_id:?} not found")))?;

    // Ownership check: only the question author can resolve it.
    let owner = question_node
        .props
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if owner != principal_id {
        return Err(execution(format!(
            "question {question_id:?} does not belong to this principal"
        )));
    }

    let mut props = question_node.props.clone();
    props["status"] = json!("resolved");
    props["resolved_by"] = json!(resolved_by);
    if let Some(n) = note {
        props["resolution_note"] = json!(n);
    }
    props["resolved_as_of"] = json!(&as_of_ts);

    block_on(async {
        graph
            .upsert_nodes(vec![tdw_core::GraphNode {
                id: question_id.to_string(),
                kind: EntityKind::OpenQuestion,
                label: question_node.label.clone(),
                aliases: question_node.aliases.clone(),
                props,
                valid_from: question_node.valid_from.clone(),
                valid_to: None,
            }])
            .await
            .map_err(|e| execution(e.to_string()))
    })?;

    // Update the in-memory store.
    {
        let mut guard = store
            .lock()
            .map_err(|e| execution(format!("question store mutex poisoned: {e}")))?;
        if let Some(entry) = guard.entries.get_mut(question_id) {
            entry.status = QuestionStatus::Resolved;
            entry.resolved_by = Some(resolved_by.to_string());
            entry.resolution_note = note.map(ToString::to_string);
        }
        save_store_to_env(&guard);
        drop(guard);
    }

    Ok(structured(json!({
        "question_id": question_id,
        "status": "resolved",
        "resolved_by": resolved_by,
        "note": note,
        "resolved_as_of": as_of,
    })))
}

// ── tdw.kg.dismiss ────────────────────────────────────────────────────────────

fn dismiss(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<OpenQuestionStore>>,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = require_graph(runtime)?;
    let principal_id = require_user_id(runtime)?;

    let question_id = require_str(arguments, "question_id")?;
    let note = require_str(arguments, "note")?;
    validate_text_field(note, "note", MAX_NOTE_CHARS)?;
    if note.trim().is_empty() {
        return Err(execution(
            "dismissal note must not be empty — explain why this is definitively absent"
                .to_string(),
        ));
    }

    let as_of =
        optional_str(arguments, "as_of").map_or_else(|| now.to_string(), ToString::to_string);
    validate_date(&as_of)?;
    let as_of_ts = tdw_tags::date_to_timestamp(&as_of);

    // Fetch the question node.
    let question_node = block_on(async {
        graph
            .node(question_id)
            .await
            .map_err(|e| execution(e.to_string()))
    })?
    .ok_or_else(|| execution(format!("question {question_id:?} not found")))?;

    let owner = question_node
        .props
        .get("user_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if owner != principal_id {
        return Err(execution(format!(
            "question {question_id:?} does not belong to this principal"
        )));
    }

    // Capture match_entity_id before mutating props.
    let match_entity_id = question_node
        .props
        .get("match_entity_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    let mut props = question_node.props.clone();
    props["status"] = json!("dismissed");
    props["resolution_note"] = json!(note);
    props["dismissal_as_of"] = json!(&as_of_ts);

    block_on(async {
        graph
            .upsert_nodes(vec![tdw_core::GraphNode {
                id: question_id.to_string(),
                kind: EntityKind::OpenQuestion,
                label: question_node.label.clone(),
                aliases: question_node.aliases.clone(),
                props,
                valid_from: question_node.valid_from.clone(),
                valid_to: None,
            }])
            .await
            .map_err(|e| execution(e.to_string()))?;

        // ── NEGATIVE KNOWLEDGE: write checked_absent edge ──────────────────
        // When a target entity is set, record a `checked_absent` edge from the
        // OpenQuestion node to the target.  This is the queryable negative-
        // knowledge assertion: future asks/extraction can see "already verified
        // absent at <as_of>" before re-deriving the same false conclusion.
        if let Some(ref target_id) = match_entity_id {
            graph
                .upsert_edges(vec![tdw_core::GraphEdge {
                    from: question_id.to_string(),
                    to: target_id.clone(),
                    rel: "checked_absent".to_string(),
                    props: json!({
                        "reason": note,
                        "dismissed_as_of": as_of_ts,
                    }),
                    provenance: tdw_core::Provenance::Agent {
                        agent_id: principal_id.to_string(),
                        gated: false,
                    },
                    valid_from: Some(as_of_ts.clone()),
                    valid_to: None,
                }])
                .await
                .map_err(|e| execution(e.to_string()))?;
        }
        Ok(())
    })?;

    // Update the in-memory store.
    let negative_knowledge_written = match_entity_id.is_some();
    {
        let mut guard = store
            .lock()
            .map_err(|e| execution(format!("question store mutex poisoned: {e}")))?;
        if let Some(entry) = guard.entries.get_mut(question_id) {
            entry.status = QuestionStatus::Dismissed;
            entry.resolution_note = Some(note.to_string());
        }
        save_store_to_env(&guard);
        drop(guard);
    }

    Ok(structured(json!({
        "question_id": question_id,
        "status": "dismissed",
        "negative_knowledge_written": negative_knowledge_written,
        "checked_absent_target": match_entity_id,
        "dismissal_note": note,
        "dismissed_as_of": as_of,
        "note": if negative_knowledge_written {
            "Dismissal recorded and checked_absent edge written — \
             the negative-knowledge assertion is queryable via tdw.kg.traverse."
        } else {
            "Dismissal recorded. No checked_absent edge written (no match_entity_id on this question)."
        },
    })))
}

// ── tdw.kg.questions ─────────────────────────────────────────────────────────

fn questions(
    runtime: &KnowledgeRuntime,
    store: &Arc<Mutex<OpenQuestionStore>>,
    arguments: &Map<String, Value>,
) -> Result<ToolExecution, ToolFailure> {
    let principal_id = require_user_id(runtime)?;

    let status_filter = optional_str(arguments, "status_filter").unwrap_or("open");
    if !["open", "resolved", "dismissed", "all"].contains(&status_filter) {
        return Err(execution(format!(
            "status_filter must be one of open|resolved|dismissed|all, got {status_filter:?}"
        )));
    }

    let entries = {
        let guard = store
            .lock()
            .map_err(|_| execution("question store mutex poisoned".to_string()))?;
        guard
            .by_principal(principal_id)
            .into_iter()
            .filter(|e| match status_filter {
                "open" => matches!(e.status, QuestionStatus::Open),
                "resolved" => matches!(e.status, QuestionStatus::Resolved),
                "dismissed" => matches!(e.status, QuestionStatus::Dismissed),
                _ => true, // "all"
            })
            .map(|e| {
                json!({
                    "question_id": e.question_id,
                    "question": e.question,
                    "status": e.status.as_str(),
                    "match_entity_id": e.match_entity_id,
                    "match_tag": e.match_tag,
                    "match_predicate": e.match_predicate,
                    "semantic_anchor": e.semantic_anchor,
                    "created_as_of": e.created_as_of,
                    "last_checked_as_of": e.last_checked_as_of,
                    "matches_fired": e.matches_fired,
                    "resolved_by": e.resolved_by,
                    "resolution_note": e.resolution_note,
                })
            })
            .collect::<Vec<_>>()
    };

    Ok(structured(json!({
        "principal_id": principal_id,
        "status_filter": status_filter,
        "questions": entries,
        "count": entries.len(),
    })))
}

// ── Cron task: matching engine ────────────────────────────────────────────────

/// Run one matching-engine tick.
///
/// Called by the cron task in `tdw-backend` with an injected `now_ms`.
///
/// For each open question:
/// 1. Compute the deterministic delta for the configured match criteria since
///    `last_checked_as_of`.
/// 2. If a candidate match is found: fire a `[tdw] QUESTION-MATCH` alert,
///    advance `last_checked_as_of`, increment `matches_fired`.
/// 3. If no match: advance `last_checked_as_of` (dedup — no re-fire).
/// 4. If zero open questions: log a note and return 0.
///
/// Returns the number of candidate-match alerts fired in this tick.
pub async fn tick_question_check(
    store: &Arc<Mutex<OpenQuestionStore>>,
    graph: &Arc<dyn GraphEngine>,
    now_ms: i64,
    freshness: &Arc<Mutex<QuestionsFreshness>>,
) -> usize {
    let now_as_of = ms_to_date(now_ms);

    // Snapshot open questions without holding the mutex across awaits.
    let questions: Vec<QuestionEntry> = store.lock().map_or_else(
        |poisoned| poisoned.into_inner().all_open(),
        |g| g.all_open(),
    );

    if questions.is_empty() {
        lock_loudly(freshness, "freshness cell (no-questions path)", |f| {
            f.open_count = 0;
            f.last_check_ms = now_ms;
            f.note = "no open questions — matching engine runs but fires nothing".to_string();
        });
        eprintln!(
            "[tdw] open questions: zero open questions — cron tick is a no-op \
             (park a question via tdw.kg.ask)"
        );
        return 0;
    }

    let mut total_fired = 0usize;

    for q in questions {
        // Skip if the window is zero-length.
        if q.last_checked_as_of >= now_as_of {
            continue;
        }

        let candidate = match check_question(graph, &q, &now_as_of).await {
            Ok(c) => c,
            Err(err) => {
                eprintln!(
                    "[tdw] ERROR: question matching failed \
                     question_id={} question={:?}: {err}",
                    q.question_id, q.question,
                );
                None
            }
        };

        if let Some(ref match_info) = candidate {
            eprintln!(
                "[tdw] QUESTION-MATCH: question={:?} match={:?} why={:?}",
                q.question, match_info.matching_edge_id, match_info.why,
            );

            lock_loudly(store, "question store (record_match)", |guard| {
                guard.record_match(&q.question_id, &now_as_of);
                save_store_to_env(guard);
            });
            total_fired += 1;
        } else {
            // No match: advance last_checked_as_of for dedup.
            lock_loudly(store, "question store (advance_checked)", |guard| {
                guard.advance_checked(&q.question_id, &now_as_of);
            });
        }
    }

    // Update freshness cell.
    let (open_count, cumulative) = lock_loudly(store, "question store (freshness)", |g| {
        (g.open_count(), g.total_matches_fired)
    });
    lock_loudly(freshness, "freshness cell (end of tick)", |f| {
        f.open_count = open_count;
        f.last_check_ms = now_ms;
        f.total_matches_fired = cumulative;
        f.note = if open_count == 0 {
            "no open questions".to_string()
        } else {
            String::new()
        };
    });

    total_fired
}

/// Candidate match info produced by `check_question`.
struct MatchCandidate {
    /// A representative edge id (from:rel:to) for the alert.
    matching_edge_id: String,
    /// Human-readable explanation.
    why: String,
}

/// Check one open question against the graph delta since `last_checked_as_of`.
///
/// Returns `Ok(Some(candidate))` when a deterministic match fires,
/// `Ok(None)` when nothing matches, or `Err(message)` on engine failure.
async fn check_question(
    graph: &Arc<dyn GraphEngine>,
    q: &QuestionEntry,
    now_as_of: &str,
) -> Result<Option<MatchCandidate>, String> {
    let from_ts = date_to_timestamp(&q.last_checked_as_of);
    let to_ts = date_to_timestamp(now_as_of);

    // ── Primary match: new edges near match_entity_id ─────────────────────
    if let Some(ref entity_id) = q.match_entity_id {
        let filter = TraversalFilter {
            direction: Direction::Both,
            max_hops: 2,
            rels: None,
            kinds: None,
            as_of: None,
        };
        let subgraph = graph
            .expand(std::slice::from_ref(entity_id), &filter)
            .await
            .map_err(|e| e.to_string())?;

        let pred_filter: Option<&str> = q.match_predicate.as_deref();

        for edge in &subgraph.edges {
            // Skip if predicate filter set and doesn't match.
            if pred_filter.is_some_and(|pred| edge.rel != pred) {
                continue;
            }
            // New edge: active at to_ts but not at from_ts.
            if !active_at(
                edge.valid_from.as_deref(),
                edge.valid_to.as_deref(),
                &from_ts,
            ) && active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), &to_ts)
            {
                let candidate_id = format!("{}:{}:{}", edge.from, edge.rel, edge.to);
                let pred_note =
                    pred_filter.map_or_else(String::new, |p| format!(" with predicate {p:?}"));
                let why = format!(
                    "new edge{pred_note} near entity {:?} appeared between {} and {}",
                    entity_id, q.last_checked_as_of, now_as_of,
                );
                return Ok(Some(MatchCandidate {
                    matching_edge_id: candidate_id,
                    why,
                }));
            }
        }
    } else if let Some(ref pred) = q.match_predicate {
        // ── Tertiary match (no entity): scan any edge with the predicate ──
        // Bounded page scan for edges with this relation.
        let edges = graph
            .edges(Some(pred.as_str()), 0, MATCH_ENGINE_EDGE_CAP)
            .await
            .map_err(|e| e.to_string())?;
        for edge in &edges {
            if !active_at(
                edge.valid_from.as_deref(),
                edge.valid_to.as_deref(),
                &from_ts,
            ) && active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), &to_ts)
            {
                let candidate_id = format!("{}:{}:{}", edge.from, edge.rel, edge.to);
                let why = format!(
                    "new edge with predicate {:?} appeared between {} and {}",
                    pred, q.last_checked_as_of, now_as_of,
                );
                return Ok(Some(MatchCandidate {
                    matching_edge_id: candidate_id,
                    why,
                }));
            }
        }
    }

    // ── Secondary match: target entity gained match_tag ──────────────────
    // Tags are first-class graph citizens (knowledge-system A5): an assignment
    // is a `tagged` edge from the entity node to the tag node, with the
    // [assigned_at, expires_at) window mapped onto the edge's
    // [valid_from, valid_to) timestamps (see GraphTagEngine). A "tag gain" is
    // therefore exactly a `tagged` edge to this tag that is active at `to_ts`
    // but was not active at `from_ts` — the same new-edge predicate the primary
    // and tertiary branches use, scanned over the same GraphEngine seam.
    if let Some(ref tag_id) = q.match_tag {
        // Page over `tagged` edges using the bounded scan seam. When a target
        // entity is set we only care about that entity gaining the tag; with no
        // target, ANY entity gaining the tag fires (the standing "did anyone get
        // tagged X?" question).
        let edges = graph
            .edges(Some("tagged"), 0, MATCH_ENGINE_EDGE_CAP)
            .await
            .map_err(|e| e.to_string())?;
        for edge in &edges {
            // Only `tagged` edges pointing AT this tag node.
            if edge.to != *tag_id {
                continue;
            }
            // When the question pins an entity, require the gain to be on it.
            if let Some(ref entity_id) = q.match_entity_id
                && edge.from != *entity_id
            {
                continue;
            }
            // New assignment: active at to_ts but not at from_ts.
            if !active_at(
                edge.valid_from.as_deref(),
                edge.valid_to.as_deref(),
                &from_ts,
            ) && active_at(edge.valid_from.as_deref(), edge.valid_to.as_deref(), &to_ts)
            {
                let candidate_id = format!("{}:{}:{}", edge.from, edge.rel, edge.to);
                let scope_note = q.match_entity_id.as_deref().map_or_else(
                    || " (any entity)".to_string(),
                    |e| format!(" on entity {e:?}"),
                );
                let why = format!(
                    "entity gained tag {tag_id:?}{scope_note} between {} and {}",
                    q.last_checked_as_of, now_as_of,
                );
                return Ok(Some(MatchCandidate {
                    matching_edge_id: candidate_id,
                    why,
                }));
            }
        }
    }

    // No deterministic match found.
    Ok(None)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Acquire `mutex`, recovering from poisoning.
///
/// # Why continuation is safe for these stores
///
/// The two mutexes this guards — [`OpenQuestionStore`] and `QuestionsFreshness`
/// — hold only independent, per-record bookkeeping: a `BTreeMap` of
/// [`QuestionEntry`] keyed by id plus a couple of scalar counters/cursors.
/// Every mutation here (`record_match`, `advance_checked`, `upsert`, the
/// freshness-cell writes) touches one record/field at a time and leaves no
/// cross-record invariant that a mid-write panic could tear: the worst a
/// poisoned tick can leave behind is a stale `last_checked_as_of` cursor or an
/// un-incremented counter, both of which self-heal on the next tick (a stale
/// cursor merely re-scans an already-seen window; a missed count is cosmetic).
/// There is no allocation/length pairing, no parallel arrays, and no partial
/// transaction that could observe a half-applied state.  Recovering and
/// continuing therefore cannot propagate corruption — only at most replay
/// idempotent work — so we log loudly at error level and proceed rather than
/// abort the whole cron loop (which would silently stop matching every standing
/// question for an unbounded time).
fn lock_loudly<T, R>(mutex: &Mutex<T>, context: &str, f: impl FnOnce(&mut T) -> R) -> R {
    match mutex.lock() {
        Ok(mut guard) => f(&mut guard),
        Err(poisoned) => {
            eprintln!(
                "[tdw] ERROR: mutex poisoned — recovering ({context}). \
                 A previous cron tick panicked; state MAY BE INCONSISTENT. \
                 Continuing is safe for this store: it holds only independent \
                 per-question bookkeeping (no torn cross-record invariant), so \
                 the worst-case residue is a stale cursor that self-heals on the \
                 next tick. See lock_loudly docs for the full justification."
            );
            let mut guard = poisoned.into_inner();
            f(&mut guard)
        }
    }
}

fn ms_to_date(ms: i64) -> String {
    let secs = ms.max(0) / 1_000;
    let days = secs / 86_400;
    let julian = days + 2_440_588;
    let adj = julian + 32_044;
    let cent = (4 * adj + 3) / 146_097;
    let rem = adj - (146_097 * cent) / 4;
    let dyear = (4 * rem + 3) / 1_461;
    let dmonth = rem - (1_461 * dyear) / 4;
    let midx = (5 * dmonth + 2) / 153;
    let day = dmonth - (153 * midx + 2) / 5 + 1;
    let month = midx + 3 - 12 * (midx / 10);
    let year = 100 * cent + dyear - 4_800 + midx / 10;
    format!("{year:04}-{month:02}-{day:02}")
}

fn date_to_timestamp(date: &str) -> String {
    tdw_tags::date_to_timestamp(date)
}

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

fn validate_graph_id(value: &str, field: &str) -> Result<(), ToolFailure> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && !matches!(c, ':' | '.' | '_' | '-'))
    {
        return Err(execution(format!(
            "invalid {field} {value:?}: must match [A-Za-z0-9:._-]+"
        )));
    }
    Ok(())
}

fn require_graph(
    runtime: &KnowledgeRuntime,
) -> Result<&std::sync::Arc<dyn GraphEngine>, ToolFailure> {
    runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))
}

fn require_user_id(runtime: &KnowledgeRuntime) -> Result<&str, ToolFailure> {
    runtime
        .bound_user_id()
        .ok_or_else(|| execution("no user identity bound to this question surface".to_string()))
}

fn require_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str, ToolFailure> {
    optional_str(arguments, name)
        .ok_or_else(|| execution(format!("missing required argument: {name}")))
}

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

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

    // ── OpenQuestionStore ──────────────────────────────────────────────────────

    fn make_entry(principal: &str, question: &str) -> QuestionEntry {
        let question_id = format!(
            "openquestion:{:016x}",
            fnv1a64(format!("{principal}:{question}").as_bytes())
        );
        QuestionEntry {
            question_id,
            principal_id: principal.to_string(),
            question: question.to_string(),
            match_entity_id: None,
            match_tag: None,
            match_predicate: None,
            semantic_anchor: None,
            status: QuestionStatus::Open,
            created_as_of: "2026-06-01".to_string(),
            last_checked_as_of: "2026-06-01".to_string(),
            matches_fired: 0,
            resolved_by: None,
            resolution_note: None,
        }
    }

    #[test]
    fn store_upsert_and_count() {
        let mut store = OpenQuestionStore::new();
        let entry = make_entry("u1", "What is AAPL revenue?");
        store.upsert(entry);
        assert_eq!(store.total_count(), 1);
        assert_eq!(store.open_count(), 1);
        assert_eq!(store.count_for_principal("u1"), 1);
    }

    #[test]
    fn store_remove_returns_true_when_found() {
        let mut store = OpenQuestionStore::new();
        let entry = make_entry("u1", "What is AAPL revenue?");
        let qid = entry.question_id.clone();
        store.upsert(entry);
        assert!(store.remove(&qid));
        assert_eq!(store.total_count(), 0);
    }

    #[test]
    fn store_record_match_increments_counts() {
        let mut store = OpenQuestionStore::new();
        let entry = make_entry("u1", "What is AAPL revenue?");
        let qid = entry.question_id.clone();
        store.upsert(entry);
        store.record_match(&qid, "2026-06-12");
        let updated = store.entries.get(&qid).expect("entry");
        assert_eq!(updated.last_checked_as_of, "2026-06-12");
        assert_eq!(updated.matches_fired, 1);
        assert_eq!(store.total_matches_fired, 1);
    }

    #[test]
    fn store_all_open_excludes_resolved_and_dismissed() {
        let mut store = OpenQuestionStore::new();
        let mut e1 = make_entry("u1", "Q1");
        e1.status = QuestionStatus::Resolved;
        let e2 = make_entry("u1", "Q2"); // stays Open
        let mut e3 = make_entry("u1", "Q3");
        e3.status = QuestionStatus::Dismissed;
        store.upsert(e1);
        store.upsert(e2);
        store.upsert(e3);
        assert_eq!(store.all_open().len(), 1);
        assert_eq!(store.open_count(), 1);
    }

    #[test]
    fn store_by_principal_sorted_by_creation() {
        let mut store = OpenQuestionStore::new();
        let mut e1 = make_entry("u1", "Q1");
        e1.created_as_of = "2026-01-01".to_string();
        let mut e2 = make_entry("u1", "Q2");
        e2.created_as_of = "2026-06-01".to_string();
        store.upsert(e2);
        store.upsert(e1);
        let by_u1 = store.by_principal("u1");
        assert_eq!(by_u1.len(), 2);
        assert_eq!(by_u1[0].created_as_of, "2026-01-01");
    }

    #[test]
    fn ms_to_date_epoch() {
        assert_eq!(ms_to_date(0), "1970-01-01");
    }

    #[test]
    fn ms_to_date_known_ts() {
        // 2025-06-12 00:00:00 UTC
        assert_eq!(ms_to_date(1_749_686_400_000), "2025-06-12");
    }

    #[test]
    fn validate_text_field_rejects_overlong() {
        let overlong = "x".repeat(MAX_QUESTION_CHARS + 1);
        assert!(validate_text_field(&overlong, "question", MAX_QUESTION_CHARS).is_err());
    }

    #[test]
    fn validate_text_field_rejects_control_chars() {
        assert!(validate_text_field("hello\x1b[31m", "question", MAX_QUESTION_CHARS).is_err());
    }

    #[test]
    fn validate_date_accepts_yyyy_mm_dd() {
        assert!(validate_date("2026-06-12").is_ok());
        assert!(validate_date("bad-date").is_err());
    }

    #[test]
    fn validate_graph_id_accepts_valid_ids() {
        assert!(validate_graph_id("instrument:AAPL", "entity_id").is_ok());
        assert!(validate_graph_id("", "entity_id").is_err());
        assert!(validate_graph_id("bad id", "entity_id").is_err());
    }

    #[test]
    fn question_status_as_str_round_trips() {
        assert_eq!(QuestionStatus::Open.as_str(), "open");
        assert_eq!(QuestionStatus::Resolved.as_str(), "resolved");
        assert_eq!(QuestionStatus::Dismissed.as_str(), "dismissed");
    }

    #[test]
    fn fnv1a64_deterministic() {
        let h1 = fnv1a64(b"test-question");
        assert_eq!(h1, fnv1a64(b"test-question"));
        assert_ne!(h1, fnv1a64(b"other-question"));
    }

    // ── Matching engine cron ───────────────────────────────────────────────────

    /// Gate: `tick_question_check` with zero open questions fires nothing and
    /// updates freshness.
    #[tokio::test]
    async fn tick_with_no_questions_fires_nothing() {
        use std::sync::Arc;
        use tdw_storage_graph::InMemoryGraphEngine;
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));
        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired = tick_question_check(&store, &graph, now_ms, &freshness).await;
        assert_eq!(fired, 0);
        let (open_count, last_check_ms) = {
            let f = freshness.lock().expect("freshness");
            (f.open_count, f.last_check_ms)
        };
        assert_eq!(open_count, 0);
        assert_eq!(last_check_ms, now_ms);
    }

    /// Gate: matching engine fires one alert when a new edge appears near the
    /// watched entity between `last_checked_as_of` and `now_as_of`.
    #[tokio::test]
    async fn tick_with_matching_new_edge_fires_one_alert() {
        use std::sync::Arc;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;

        let graph_engine = InMemoryGraphEngine::default();
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_engine);

        // Seed two entity nodes.
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "instrument:AAPL_ETF".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple ETF".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");

        // Add an edge that is NEW relative to 2025-05-01 but active by 2025-06-12.
        graph
            .upsert_edges(vec![GraphEdge {
                from: "instrument:AAPL".to_string(),
                to: "instrument:AAPL_ETF".to_string(),
                rel: "related_to".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert edge");

        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));

        // Park a question whose last_checked_as_of is before the edge.
        let mut entry = make_entry("analyst", "Is there a new AAPL relationship?");
        entry.match_entity_id = Some("instrument:AAPL".to_string());
        entry.last_checked_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        // now_ms = 2025-06-12
        let now_ms = 1_749_686_400_000_i64;
        let fired = tick_question_check(&store, &graph, now_ms, &freshness).await;

        // Exactly one alert should have fired.
        assert_eq!(fired, 1);

        // Freshness bookkeeping updated.
        let (total_matches_fired, last_check_ms) = {
            let f = freshness.lock().expect("freshness");
            (f.total_matches_fired, f.last_check_ms)
        };
        assert_eq!(total_matches_fired, 1);
        assert_eq!(last_check_ms, now_ms);
    }

    /// Gate: a second tick with the same data does NOT re-fire because
    /// `last_checked_as_of` was advanced on the first tick.
    #[tokio::test]
    async fn tick_reruns_do_not_re_fire() {
        use std::sync::Arc;
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;

        let graph_engine = InMemoryGraphEngine::default();
        let graph: Arc<dyn GraphEngine> = Arc::new(graph_engine);

        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "instrument:AAPL_ETF".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple ETF".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");

        graph
            .upsert_edges(vec![GraphEdge {
                from: "instrument:AAPL".to_string(),
                to: "instrument:AAPL_ETF".to_string(),
                rel: "related_to".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert edge");

        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));

        let mut entry = make_entry("analyst", "Is there a new AAPL relationship?");
        entry.match_entity_id = Some("instrument:AAPL".to_string());
        entry.last_checked_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired1 = tick_question_check(&store, &graph, now_ms, &freshness).await;
        // Second tick at the same timestamp — last_checked_as_of is now 2025-06-12,
        // equal to the edge's valid_from window end, so nothing is new.
        let fired2 = tick_question_check(&store, &graph, now_ms, &freshness).await;

        assert_eq!(fired1, 1);
        assert_eq!(fired2, 0, "re-run must not re-fire the same match");
    }

    // ── Negative knowledge (dismiss path) ─────────────────────────────────────

    /// Gate: a dismissed entry is excluded from `all_open()` and `open_count()`.
    #[test]
    fn dismissed_entry_excluded_from_open() {
        let mut store = OpenQuestionStore::new();
        let mut e = make_entry("u1", "Is X absent?");
        e.status = QuestionStatus::Dismissed;
        e.resolution_note = Some("Verified absent — no such entity in KG.".to_string());
        store.upsert(e);
        assert_eq!(store.open_count(), 0);
        assert_eq!(store.all_open().len(), 0);
        assert_eq!(
            store.total_count(),
            1,
            "dismissed still counts toward total"
        );
    }

    /// Gate: `store.record_match` does not touch dismissed entries (they are
    /// excluded from `all_open()` before the engine ever sees them).
    #[test]
    fn record_match_on_open_entry_advances_cursor() {
        let mut store = OpenQuestionStore::new();
        let entry = make_entry("u1", "Will AAPL beat earnings?");
        let qid = entry.question_id.clone();
        store.upsert(entry);
        store.record_match(&qid, "2026-06-12");
        let updated = store.entries.get(&qid).expect("entry");
        assert_eq!(updated.last_checked_as_of, "2026-06-12");
        assert_eq!(updated.matches_fired, 1);
    }

    // ── Secondary match: match_tag (K-X8 open-question Gemini HIGH fix) ────────

    /// Seed an entity, a tag node, and a `tagged` edge (the A5 tag-assignment
    /// edge shape) whose `valid_from` falls inside `[from, to)`.
    #[cfg(test)]
    async fn seed_tag_assignment(
        graph: &std::sync::Arc<dyn GraphEngine>,
        entity_id: &str,
        tag_id: &str,
        valid_from: &str,
    ) {
        use tdw_core::{GraphEdge, GraphNode, Provenance};
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: entity_id.to_string(),
                    kind: EntityKind::Instrument,
                    label: entity_id.to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: tag_id.to_string(),
                    kind: EntityKind::Tag,
                    label: tag_id.to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: entity_id.to_string(),
                to: tag_id.to_string(),
                rel: "tagged".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some(valid_from.to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert tagged edge");
    }

    /// Gate (HIGH fix): a tag-only question (no entity, no predicate) FIRES when
    /// some entity gains the configured tag inside the check window — the path
    /// that was documented-but-dead before this fix.
    #[tokio::test]
    async fn tick_fires_when_entity_gains_match_tag() {
        use std::sync::Arc;
        use tdw_storage_graph::InMemoryGraphEngine;

        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // Tag assigned 2025-06-01 — new relative to last_checked 2025-05-01,
        // active by now 2025-06-12.
        seed_tag_assignment(
            &graph,
            "instrument:AAPL",
            "sector:tech",
            "2025-06-01T00:00:00Z",
        )
        .await;

        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));

        let mut entry = make_entry("analyst", "Did anything get tagged tech?");
        entry.match_tag = Some("sector:tech".to_string());
        entry.last_checked_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired = tick_question_check(&store, &graph, now_ms, &freshness).await;
        assert_eq!(fired, 1, "match_tag must fire on a tag gain");
    }

    /// Gate: the SAME tag-only question does NOT fire when the tag is absent
    /// (no `tagged` edge to it) — proving the branch is truthful, not a
    /// fire-always stub.
    #[tokio::test]
    async fn tick_does_not_fire_when_match_tag_absent() {
        use std::sync::Arc;
        use tdw_core::{GraphNode, Provenance};
        use tdw_storage_graph::InMemoryGraphEngine;

        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // Seed the entity and an UNRELATED tag assignment, but never assign the
        // tag the question watches for.
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: "instrument:AAPL".to_string(),
                    kind: EntityKind::Instrument,
                    label: "Apple".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "sector:energy".to_string(),
                    kind: EntityKind::Tag,
                    label: "Energy".to_string(),
                    aliases: vec![],
                    props: Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: "instrument:AAPL".to_string(),
                to: "sector:energy".to_string(),
                rel: "tagged".to_string(),
                props: Value::Null,
                provenance: Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some("2025-06-01T00:00:00Z".to_string()),
                valid_to: None,
            }])
            .await
            .expect("upsert tagged edge");

        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));

        let mut entry = make_entry("analyst", "Did anything get tagged tech?");
        entry.match_tag = Some("sector:tech".to_string()); // absent in the graph
        entry.last_checked_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired = tick_question_check(&store, &graph, now_ms, &freshness).await;
        assert_eq!(fired, 0, "match_tag must NOT fire when the tag is absent");
    }

    /// Gate: with BOTH an entity and a tag pinned, the gain must be ON that
    /// entity — a different entity gaining the same tag does not fire.
    #[tokio::test]
    async fn tick_match_tag_scoped_to_entity() {
        use std::sync::Arc;
        use tdw_storage_graph::InMemoryGraphEngine;

        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // A DIFFERENT entity gains the tag.
        seed_tag_assignment(
            &graph,
            "instrument:MSFT",
            "sector:tech",
            "2025-06-01T00:00:00Z",
        )
        .await;

        let store = Arc::new(Mutex::new(OpenQuestionStore::new()));
        let freshness = Arc::new(Mutex::new(QuestionsFreshness::default()));

        let mut entry = make_entry("analyst", "Did AAPL get tagged tech?");
        entry.match_entity_id = Some("instrument:AAPL".to_string());
        entry.match_tag = Some("sector:tech".to_string());
        entry.last_checked_as_of = "2025-05-01".to_string();
        {
            let mut s = store.lock().expect("store");
            s.upsert(entry);
        }

        let now_ms = 1_749_686_400_000_i64; // 2025-06-12
        let fired = tick_question_check(&store, &graph, now_ms, &freshness).await;
        assert_eq!(
            fired, 0,
            "tag gain on a different entity must not fire an entity-pinned question"
        );
    }
}
