//! Session → findings distillation as proposals (knowledge-system K-X9).
//!
//! ## The differentiator
//!
//! At session end a transcript is distilled into candidate research **findings**
//! (a claim + the supporting episode evidence that backs it).  Like K-M1
//! extraction, these findings are **never written directly** into the knowledge
//! substrate.  Every candidate is routed through the **B9 PROPOSAL GATE** as a
//! `Validated` [`ProposalKind::Annotation`] on an existing graph entity, from
//! where the eval-gated promotion + K-L4 auto-materialize sweep lands it exactly
//! as agent-authored proposals are landed.  Until a proposal is `Ready` and
//! materialized it is **invisible** to reads — a distilled finding is therefore
//! `pending-eval`, NOT presented as established knowledge until promoted.
//!
//! ## Keyless / offline (deterministic extractive distillation)
//!
//! The default path is a **deterministic extractive distiller** that needs no
//! model: it tokenizes the transcript, finds existing graph entities mentioned
//! across turns, and proposes one annotation per anchored entity carrying the
//! distilled claim plus the supporting episode evidence ids.  A keyless install
//! runs this path unchanged.  A production LLM (`is_production_grade() == true`)
//! MAY refine the claim text when wired and explicitly enabled — cost-bounded,
//! off by default — but the gate, anchoring, and provenance are identical.
//!
//! ## Honesty
//!
//! * **Provenance** — every proposal carries `agent_id =
//!   "distill:<session_id>"` (validated against the B9 grammar) and the note
//!   encodes the originating `session=<id>` plus the supporting
//!   `episodes=<id,id,...>` so the why-chain is auditable.
//! * **Pending-eval** — findings land as `Validated` proposals, invisible to
//!   reads until promoted.  They are candidates, never established knowledge.
//! * **Empty session** — a session with no turns, or no turn mentioning a known
//!   graph entity, yields an honest **empty** [`DistillationResult`]; no
//!   proposal is submitted and the deterministic pipeline is untouched.
//!
//! ## Leakage safety
//!
//! Distillation is **injected-now**: the caller supplies `as_of` = the session
//! time and it is the only timestamp stamped onto the proposal history.  No
//! wall-clock is read here, so a replayed/back-dated session never leaks a
//! future `now` into the audit trail (the same discipline as the extraction
//! stage's `reset_daily_budget`).
//!
//! ## Safety
//!
//! Transcript text is **untrusted input** into graph annotation notes.  Every
//! claim is length-capped and control-character-stripped before it is wrapped
//! into a note, and the B9 `Annotation` validator runs again at enqueue time
//! (non-empty, `<= MAX_NOTE_CHARS`, no control characters, entity exists).

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::GraphEngine;
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

use crate::proposals::{MAX_AGENT_ID_LEN, ProposalKind, ProposalQueue};
use crate::{KnowledgeError, Result};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum characters of a distilled claim retained in the proposal note.
///
/// Bounds note size (notes flow into graph props and agent context) well under
/// the B9 `MAX_NOTE_CHARS` ceiling, leaving room for the provenance envelope.
pub const MAX_CLAIM_CHARS: usize = 2_048;

/// Maximum number of candidate findings distilled from a single session.
///
/// Bounds queue pressure from one distillation run; the B9 per-agent pending
/// cap is the second layer.
pub const MAX_FINDINGS_PER_SESSION: usize = 32;

/// Maximum number of episode evidence ids cited by a single finding.
///
/// Keeps the note bounded even for a long session that mentions one entity in
/// many turns.
pub const MAX_EVIDENCE_PER_FINDING: usize = 16;

/// Minimum token length for a transcript word to count as an entity anchor.
///
/// Filters trivial stopword-length tokens cheaply.
const MIN_ANCHOR_TOKEN_LEN: usize = 2;

/// Bound on the number of known graph-entity ids scanned for anchoring.
///
/// Mirrors the K-X6 auto-link scan limit so one distillation never turns into an
/// O(n) full graph scan.
const ANCHOR_SCAN_LIMIT: usize = 512;

// ---------------------------------------------------------------------------
// Session transcript input
// ---------------------------------------------------------------------------

/// One turn of a session transcript.
///
/// A turn is the atomic unit of episode evidence: its `episode_id` is the
/// citation a distilled finding carries to point back at the supporting
/// evidence.  `episode_id` is opaque to distillation — it is whatever stable id
/// the caller's episodic layer (K-M2) assigns to the turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptTurn {
    /// Stable evidence id for this turn (the citation a finding cites). Opaque
    /// to distillation; sanitized into the note's `episodes=` envelope.
    pub episode_id: String,
    /// The speaker role (e.g. `"user"`, `"assistant"`). Informational; not used
    /// for anchoring.
    #[serde(default)]
    pub role: String,
    /// The turn text — the corpus distillation mines for entity mentions.
    pub text: String,
}

/// A bounded window of a session transcript to distill.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTranscript {
    /// Stable session id. Sanitized into the provenance `agent_id =
    /// "distill:<session_id>"` and the note's `session=` envelope.
    pub session_id: String,
    /// Leakage-safe `as_of` = the session time (`YYYY-MM-DD` or any caller
    /// timestamp). Stamped as the proposal-history `now`; no wall-clock is read.
    pub as_of: String,
    /// The transcript turns, in order. Empty → honest empty distillation.
    #[serde(default)]
    pub turns: Vec<TranscriptTurn>,
}

// ---------------------------------------------------------------------------
// Distillation result
// ---------------------------------------------------------------------------

/// One candidate finding the distiller proposed (or skipped).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistilledFinding {
    /// The existing graph entity the finding is anchored to.
    pub anchor_entity_id: String,
    /// The distilled claim (capped, control-stripped).
    pub claim: String,
    /// Supporting episode evidence ids (the citations).
    pub evidence_episode_ids: Vec<String>,
    /// The B9 proposal id, when the finding was successfully enqueued. `None`
    /// when the candidate was skipped (e.g. B9 validator rejection).
    #[serde(default)]
    pub proposal_id: Option<String>,
}

/// Result of one distillation run over a session transcript.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistillationResult {
    /// The session that was distilled.
    pub session_id: String,
    /// Candidate findings that were enqueued as PROPOSALS (never direct writes).
    pub findings: Vec<DistilledFinding>,
    /// How many candidate findings were skipped (B9 rejection, e.g. duplicate
    /// note on the same entity, or per-agent cap reached).
    pub skipped: usize,
    /// `true` when the session was empty OR no turn mentioned a known graph
    /// entity — an honest empty result, no proposal submitted.
    pub empty: bool,
}

impl DistillationResult {
    /// How many candidate findings were enqueued as proposals.
    #[must_use]
    pub fn enqueued(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.proposal_id.is_some())
            .count()
    }
}

// ---------------------------------------------------------------------------
// Freshness status (surfaces in tdw.kg.status)
// ---------------------------------------------------------------------------

/// Observability status for the session-distillation worker (K-X9).
///
/// Written after each distillation run; read by `KnowledgeRuntime::status()` to
/// populate `KgStatus::distillation_freshness`. `None` means the worker has
/// never run since daemon start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DistillationFreshness {
    /// The worker is wired but has not distilled any session yet.
    Pending,
    /// Last distillation completed.
    Ok {
        /// The session id distilled last.
        session_id: String,
        /// Findings enqueued as proposals in the last run.
        enqueued: usize,
        /// Candidate findings skipped (B9 rejection) in the last run.
        skipped: usize,
        /// `true` when the last session distilled to an honest empty result.
        empty: bool,
    },
}

// ---------------------------------------------------------------------------
// The distiller
// ---------------------------------------------------------------------------

/// Deterministic, keyless session-distillation worker (K-X9).
///
/// Holds the gated [`ProposalQueue`] and the validation engines, mirroring the
/// K-M1 [`ExtractionStage`](crate::extraction::ExtractionStage). The distiller
/// never writes the graph directly — every candidate finding is submitted
/// through [`ProposalQueue::submit`].
pub struct SessionDistiller {
    proposals: Arc<tokio::sync::Mutex<ProposalQueue>>,
    graph: Arc<dyn GraphEngine>,
    tags: Arc<dyn TagEngine>,
    adaptivity: Adaptivity,
    freshness_cell: Option<Arc<std::sync::Mutex<DistillationFreshness>>>,
}

impl std::fmt::Debug for SessionDistiller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDistiller")
            .field("adaptivity", &self.adaptivity)
            .field("freshness_wired", &self.freshness_cell.is_some())
            .finish_non_exhaustive()
    }
}

impl SessionDistiller {
    /// Build a distiller over the gated queue and the validation engines.
    ///
    /// `adaptivity` is the proposing identity's adaptivity level — it must be at
    /// least [`Adaptivity::Learning`] for the B9 gate to admit submissions
    /// (below that every submit is rejected loudly, which is the correct,
    /// honest behaviour).
    #[must_use]
    pub fn new(
        proposals: Arc<tokio::sync::Mutex<ProposalQueue>>,
        graph: Arc<dyn GraphEngine>,
        tags: Arc<dyn TagEngine>,
        adaptivity: Adaptivity,
    ) -> Self {
        Self {
            proposals,
            graph,
            tags,
            adaptivity,
            freshness_cell: None,
        }
    }

    /// Attach the shared freshness cell so `tdw.kg.status` reports the real
    /// last-run state instead of a stale default.
    #[must_use]
    pub fn with_freshness_cell(
        mut self,
        cell: Arc<std::sync::Mutex<DistillationFreshness>>,
    ) -> Self {
        self.freshness_cell = Some(cell);
        self
    }

    /// Publish `state` into the shared freshness cell, if attached. Recovers
    /// from a poisoned mutex (a panic in another holder must not wedge the
    /// distiller — status honesty is best-effort and never blocks).
    fn publish_freshness(&self, state: DistillationFreshness) {
        if let Some(cell) = &self.freshness_cell {
            match cell.lock() {
                Ok(mut guard) => *guard = state,
                Err(poisoned) => *poisoned.into_inner() = state,
            }
        }
    }

    /// Distill one session transcript into candidate findings, submitting each
    /// through the B9 gate.
    ///
    /// Deterministic and keyless: the same transcript always yields the same
    /// candidate findings in the same order (entity ids are anchored in sorted
    /// order; evidence ids preserve transcript order).
    ///
    /// `transcript.as_of` is the only timestamp stamped onto the proposal
    /// history — no wall-clock is read (leakage-safe injected-now).
    ///
    /// Returns an honest empty result when the session has no turns or no turn
    /// mentions a known graph entity.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] on a hard queue/engine failure or an
    /// invalid `session_id` (B9 agent-id grammar). Individual B9 validator
    /// rejections (e.g. a duplicate annotation) are counted as `skipped`, not
    /// propagated.
    pub async fn distill(&self, transcript: &SessionTranscript) -> Result<DistillationResult> {
        let agent_id = build_distill_agent_id(&transcript.session_id);
        // Validate the provenance id up front so a malformed session id is a
        // loud error rather than a silent no-op.
        crate::proposals::validate_agent_id(&agent_id)?;

        let mut result = DistillationResult {
            session_id: transcript.session_id.clone(),
            ..DistillationResult::default()
        };

        // Empty transcript → honest empty, no scan, no proposal.
        if transcript.turns.is_empty() {
            result.empty = true;
            self.publish_post_run(&result);
            return Ok(result);
        }

        // Deterministic extractive distillation: anchor candidate findings to
        // existing graph entities mentioned across the transcript turns.
        let candidates = self.extract_candidates(transcript).await?;
        if candidates.is_empty() {
            result.empty = true;
            self.publish_post_run(&result);
            return Ok(result);
        }

        // Acquire and release the queue lock PER finding rather than holding it
        // across the whole loop: `submit` is async and validates against the
        // graph/tag engines (IO), so a loop-wide guard would block every other
        // queue user for the duration of up to MAX_FINDINGS_PER_SESSION submits
        // (lock-across-IO; Gemini MEDIUM).
        for mut finding in candidates {
            let note = build_finding_note(
                &transcript.session_id,
                &finding.claim,
                &finding.evidence_episode_ids,
            );
            let kind = ProposalKind::Annotation {
                entity_id: finding.anchor_entity_id.clone(),
                note,
            };
            let submit_result = {
                let mut queue = self.proposals.lock().await;
                queue
                    .submit(
                        &agent_id,
                        self.adaptivity,
                        kind,
                        &self.graph,
                        &self.tags,
                        &transcript.as_of,
                    )
                    .await
            };
            match submit_result {
                Ok(proposal) => finding.proposal_id = Some(proposal.id),
                Err(_) => result.skipped += 1,
            }
            result.findings.push(finding);
        }

        self.publish_post_run(&result);
        Ok(result)
    }

    /// Publish the post-run freshness state from a completed result.
    fn publish_post_run(&self, result: &DistillationResult) {
        self.publish_freshness(DistillationFreshness::Ok {
            session_id: result.session_id.clone(),
            enqueued: result.enqueued(),
            skipped: result.skipped,
            empty: result.empty,
        });
    }

    /// Build candidate findings deterministically from the transcript: for each
    /// known graph entity mentioned in the transcript, emit one finding whose
    /// claim is the distilled mention context and whose evidence is the set of
    /// turns that mentioned it.
    async fn extract_candidates(
        &self,
        transcript: &SessionTranscript,
    ) -> Result<Vec<DistilledFinding>> {
        let entity_ids = self.scan_entity_ids().await?;
        if entity_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Per-entity: which turns mention it (in transcript order) and a claim
        // assembled from the first mentioning turn's text.
        let mut findings: Vec<DistilledFinding> = Vec::new();
        for entity_id in &entity_ids {
            let token = entity_id
                .rsplit(':')
                .next()
                .unwrap_or(entity_id.as_str())
                .to_lowercase();
            if token.len() < MIN_ANCHOR_TOKEN_LEN {
                continue;
            }
            let id_lower = entity_id.to_lowercase();

            let mut evidence: Vec<String> = Vec::new();
            let mut claim_source: Option<&str> = None;
            for turn in &transcript.turns {
                let turn_lower = turn.text.to_lowercase();
                if contains_token(&turn_lower, &token) || contains_token(&turn_lower, &id_lower) {
                    if claim_source.is_none() {
                        claim_source = Some(turn.text.as_str());
                    }
                    if evidence.len() < MAX_EVIDENCE_PER_FINDING {
                        evidence.push(turn.episode_id.clone());
                    }
                }
            }

            let Some(source) = claim_source else { continue };
            if evidence.is_empty() {
                continue;
            }
            let claim = distill_claim(entity_id, source);
            if claim.is_empty() {
                continue;
            }
            findings.push(DistilledFinding {
                anchor_entity_id: entity_id.clone(),
                claim,
                evidence_episode_ids: evidence,
                proposal_id: None,
            });
            if findings.len() >= MAX_FINDINGS_PER_SESSION {
                break;
            }
        }
        Ok(findings)
    }

    /// Collect a bounded, sorted sample of known entity ids from the graph by
    /// scanning edges (mirrors the K-X6 auto-link scan, bounded by
    /// [`ANCHOR_SCAN_LIMIT`]).
    async fn scan_entity_ids(&self) -> Result<Vec<String>> {
        let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
        let mut ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut offset = 0usize;
        let page_size = 256usize;
        loop {
            let page = self
                .graph
                .edges(None, offset, page_size)
                .await
                .map_err(storage)?;
            if page.is_empty() {
                break;
            }
            for edge in &page {
                ids.insert(edge.from.clone());
                ids.insert(edge.to.clone());
                if ids.len() >= ANCHOR_SCAN_LIMIT {
                    return Ok(ids.into_iter().collect());
                }
            }
            offset += page.len();
            if offset >= ANCHOR_SCAN_LIMIT * 4 {
                break;
            }
        }
        Ok(ids.into_iter().collect())
    }
}

// ---------------------------------------------------------------------------
// Deterministic helpers
// ---------------------------------------------------------------------------

/// Build the provenance `agent_id` for distillation proposals.
///
/// Format: `distill:<sanitized_session_id>`. The B9 grammar is
/// `[A-Za-z0-9:._-]+`; the session id is sanitized (anything else → `-`) and the
/// result truncated to [`MAX_AGENT_ID_LEN`].
#[must_use]
pub fn build_distill_agent_id(session_id: &str) -> String {
    let sanitized: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let raw = format!("distill:{sanitized}");
    raw[..raw.len().min(MAX_AGENT_ID_LEN)].to_string()
}

/// Distill a one-line claim for `entity_id` from the first mentioning turn.
///
/// Deterministic: trims and collapses whitespace, strips control characters
/// (except newline, which is normalized to a space), and caps to
/// [`MAX_CLAIM_CHARS`]. Returns an empty string when nothing survives (caller
/// skips empty claims).
#[must_use]
pub fn distill_claim(entity_id: &str, source_text: &str) -> String {
    let cleaned: String = source_text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    let token = entity_id.rsplit(':').next().unwrap_or(entity_id);
    let claim = format!("Session evidence referencing {token}: {collapsed}");
    truncate_chars(&claim, MAX_CLAIM_CHARS)
}

/// Build the proposal note carrying the claim plus the provenance envelope.
///
/// Format: `<claim>\n[distill] session=<id> episodes=<id,id,...>`.
/// The envelope is parseable by an aggregation layer and makes the why-chain
/// (session + supporting episodes) explicit. Session and episode ids are
/// sanitized to the note-safe charset; the claim is already cleaned by
/// [`distill_claim`].
#[must_use]
pub fn build_finding_note(session_id: &str, claim: &str, episode_ids: &[String]) -> String {
    let session = sanitize_envelope_field(session_id);
    let episodes = episode_ids
        .iter()
        .map(|id| sanitize_envelope_field(id))
        .collect::<Vec<_>>()
        .join(",");
    let note = format!("{claim}\n[distill] session={session} episodes={episodes}");
    // Guard the B9 note ceiling even with the envelope appended.
    truncate_chars(&note, crate::proposals::MAX_NOTE_CHARS)
}

/// Sanitize an id for the note envelope: keep `[A-Za-z0-9._:-]`, replace
/// anything else (including whitespace and `,` / `=`) with `-` so the envelope
/// parse is unambiguous.
fn sanitize_envelope_field(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Truncate a string to at most `max` characters (char-safe, never splits a
/// multi-byte boundary).
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Whole-token containment: `token` must appear in `haystack` bounded by
/// non-alphanumeric characters (so `AA` does not match inside `AAPL`). Both
/// arguments are expected lowercased by the caller.
fn contains_token(haystack: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let bytes = haystack.as_bytes();
    let token_bytes = token.as_bytes();
    let mut start = 0usize;
    while let Some(pos) = haystack[start..].find(token) {
        let abs = start + pos;
        let before_ok = abs == 0 || !bytes[abs - 1].is_ascii_alphanumeric();
        let after = abs + token_bytes.len();
        let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        // Advance past the matched char by its UTF-8 length so `start` always
        // lands on a char boundary — `abs + 1` would split a multi-byte char
        // and panic the next `haystack[start..]` slice (Gemini HIGH).
        start = abs + haystack[abs..].chars().next().map_or(1, char::len_utf8);
        if start >= haystack.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tdw_core::{GraphEdge, GraphNode, Provenance};
    use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};
    use tdw_taxonomy::EntityKind;

    // ── Deterministic helper tests (run in release too) ────────────────────

    #[test]
    fn build_distill_agent_id_sanitizes_and_bounds() {
        let id = build_distill_agent_id("sess/2026 06 13@x");
        assert!(id.starts_with("distill:"), "must carry distill provenance: {id}");
        assert!(!id.contains(' '), "no whitespace in agent id: {id}");
        assert!(!id.contains(';'), "no semicolons in agent id: {id}");
        assert!(!id.contains('@'), "no @ in agent id: {id}");
        assert!(id.len() <= MAX_AGENT_ID_LEN, "agent id bounded: {id}");
        // The grammar must accept it.
        assert!(crate::proposals::validate_agent_id(&id).is_ok());
    }

    #[test]
    fn distill_claim_strips_control_and_caps() {
        let claim = distill_claim("instrument:AAPL", "Apple\tbeat\nestimates  badly");
        assert!(claim.contains("AAPL"), "claim cites the anchor token: {claim}");
        assert!(!claim.contains('\t'), "control chars stripped: {claim:?}");
        assert!(!claim.contains('\n'), "newlines normalized: {claim:?}");
        assert!(claim.chars().count() <= MAX_CLAIM_CHARS);
    }

    #[test]
    fn distill_claim_empty_source_is_empty() {
        assert!(distill_claim("instrument:AAPL", "   \t \n ").is_empty());
    }

    #[test]
    fn build_finding_note_carries_provenance_envelope() {
        let note = build_finding_note(
            "sess-1",
            "Session evidence referencing AAPL: beat estimates",
            &["ep-1".to_string(), "ep-2".to_string()],
        );
        assert!(note.contains("[distill] session=sess-1"), "{note}");
        assert!(note.contains("episodes=ep-1,ep-2"), "{note}");
        assert!(note.chars().count() <= crate::proposals::MAX_NOTE_CHARS);
    }

    #[test]
    fn contains_token_is_whole_token() {
        assert!(contains_token("apple aapl reported", "aapl"));
        assert!(!contains_token("aaplx reported", "aapl"));
        assert!(!contains_token("xaapl reported", "aapl"));
        assert!(contains_token("(aapl)", "aapl"));
    }

    // ── Gate behaviour: enqueues proposals, never direct writes ─────────────

    async fn graph_with_entity(id: &str) -> Arc<dyn GraphEngine> {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // Two nodes + an edge so the entity exists and the edge-scan finds it.
        graph
            .upsert_nodes(vec![
                GraphNode {
                    id: id.to_string(),
                    kind: EntityKind::Instrument,
                    label: id.to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
                GraphNode {
                    id: "venue:nasdaq".to_string(),
                    kind: EntityKind::Venue,
                    label: "NASDAQ".to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                },
            ])
            .await
            .expect("upsert nodes");
        graph
            .upsert_edges(vec![GraphEdge {
                from: id.to_string(),
                to: "venue:nasdaq".to_string(),
                rel: "listed_on".to_string(),
                props: serde_json::Value::Null,
                provenance: Provenance::System {
                    detail: "test-fixture".to_string(),
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("upsert edge");
        graph
    }

    fn tag_engine() -> Arc<dyn TagEngine> {
        Arc::new(GraphTagEngine::new(InMemoryGraphEngine::default()))
    }

    /// Count all stored edges (paged) — the "no direct write" probe.
    async fn edge_count(graph: &Arc<dyn GraphEngine>) -> usize {
        let mut total = 0usize;
        let mut offset = 0usize;
        loop {
            let page = graph.edges(None, offset, 256).await.expect("edges");
            if page.is_empty() {
                break;
            }
            total += page.len();
            offset += page.len();
        }
        total
    }

    fn transcript(turns: &[(&str, &str)]) -> SessionTranscript {
        SessionTranscript {
            session_id: "sess-1".to_string(),
            as_of: "2026-06-13".to_string(),
            turns: turns
                .iter()
                .enumerate()
                .map(|(i, (role, text))| TranscriptTurn {
                    episode_id: format!("ep-{i}"),
                    role: (*role).to_string(),
                    text: (*text).to_string(),
                })
                .collect(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn distillation_enqueues_proposals_not_direct_writes() {
        let graph = graph_with_entity("instrument:AAPL").await;
        let tags = tag_engine();
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));

        // Snapshot the graph edge count before: distillation must NOT write the
        // graph directly (an annotation would add an `annotated_by` edge).
        let edges_before = edge_count(&graph).await;

        let distiller = SessionDistiller::new(
            Arc::clone(&queue),
            Arc::clone(&graph),
            Arc::clone(&tags),
            Adaptivity::Learning,
        );
        let t = transcript(&[("user", "What about AAPL this quarter?")]);
        let result = distiller.distill(&t).await.expect("distill");

        assert!(!result.empty, "non-empty session with a known entity");
        assert_eq!(result.enqueued(), 1, "one finding enqueued: {result:?}");
        let finding = &result.findings[0];
        assert_eq!(finding.anchor_entity_id, "instrument:AAPL");
        assert!(finding.proposal_id.is_some(), "proposal id assigned");
        assert_eq!(finding.evidence_episode_ids, vec!["ep-0".to_string()]);

        // The proposal sits in the queue as Validated (pending), invisible to
        // reads — NOT materialized into the graph.
        let queue_guard = queue.lock().await;
        let (_, validated, _) = queue_guard.pending_counts_by_state();
        assert_eq!(validated, 1, "exactly one pending Validated proposal");
        assert_eq!(queue_guard.distilled_pending(), 1, "distilled-pending count");
        drop(queue_guard);

        // The graph edge set is unchanged — no direct write happened.
        let edges_after = edge_count(&graph).await;
        assert_eq!(edges_before, edges_after, "distillation made no direct graph write");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_session_is_honest_empty() {
        let graph = graph_with_entity("instrument:AAPL").await;
        let tags = tag_engine();
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        let distiller = SessionDistiller::new(
            Arc::clone(&queue),
            graph,
            tags,
            Adaptivity::Learning,
        );

        let empty = SessionTranscript {
            session_id: "sess-empty".to_string(),
            as_of: "2026-06-13".to_string(),
            turns: Vec::new(),
        };
        let result = distiller.distill(&empty).await.expect("distill empty");
        assert!(result.empty, "empty session → honest empty");
        assert_eq!(result.enqueued(), 0);
        assert!(result.findings.is_empty());
        assert_eq!(queue.lock().await.distilled_pending(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_known_entity_is_honest_empty() {
        let graph = graph_with_entity("instrument:AAPL").await;
        let tags = tag_engine();
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        let distiller =
            SessionDistiller::new(Arc::clone(&queue), graph, tags, Adaptivity::Learning);

        // Turn mentions no known graph entity → honest empty, no proposal.
        let t = transcript(&[("user", "Tell me about the weather forecast")]);
        let result = distiller.distill(&t).await.expect("distill");
        assert!(result.empty, "no known entity mentioned → honest empty");
        assert_eq!(result.enqueued(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn keyless_extractive_path_is_deterministic() {
        // No model wired anywhere — the deterministic path runs and is stable
        // across repeated runs (same evidence ids, same order).
        let graph = graph_with_entity("instrument:AAPL").await;
        let tags = tag_engine();

        let run = || async {
            let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
            let distiller = SessionDistiller::new(
                Arc::clone(&queue),
                Arc::clone(&graph),
                Arc::clone(&tags),
                Adaptivity::Learning,
            );
            let t = transcript(&[
                ("user", "AAPL guidance?"),
                ("assistant", "AAPL raised guidance"),
            ]);
            distiller.distill(&t).await.expect("distill")
        };

        let a = run().await;
        let b = run().await;
        assert_eq!(a.findings.len(), b.findings.len());
        assert_eq!(
            a.findings[0].evidence_episode_ids, b.findings[0].evidence_episode_ids,
            "evidence ids deterministic"
        );
        assert_eq!(a.findings[0].claim, b.findings[0].claim, "claim deterministic");
        assert_eq!(
            a.findings[0].evidence_episode_ids,
            vec!["ep-0".to_string(), "ep-1".to_string()],
            "both mentioning turns cited in transcript order"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn below_learning_adaptivity_is_skipped_not_written() {
        let graph = graph_with_entity("instrument:AAPL").await;
        let tags = tag_engine();
        let queue = Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()));
        // Adaptivity::None is below Learning → the B9 admission gate rejects.
        let distiller =
            SessionDistiller::new(Arc::clone(&queue), graph, tags, Adaptivity::None);
        let t = transcript(&[("user", "AAPL update")]);
        let result = distiller.distill(&t).await.expect("distill");
        assert_eq!(result.enqueued(), 0, "below Learning enqueues nothing");
        assert_eq!(result.skipped, 1, "candidate skipped at the gate");
        assert_eq!(queue.lock().await.distilled_pending(), 0);
    }
}
