//! LLM extraction-as-proposals (knowledge-system K-M1).
//!
//! ## The differentiator
//!
//! Every competitor (Zep, Mem0, Cognee) writes LLM-extracted facts **directly**
//! into their memory substrate.  TDW routes them through the **B9 PROPOSAL
//! GATE** instead: extracted entities and relations become `Validated` proposals
//! in the [`ProposalQueue`], from where the eval-gated promotion + K-L4
//! auto-materialize sweep lands them exactly as agent-authored proposals are
//! landed.  Until a proposal is `Ready` and materialized it is **invisible** to
//! reads — the deterministic lexical/rule pipeline is never blocked or replaced.
//!
//! ## Stub gate (ER3 pattern, PR #345)
//!
//! [`is_production_grade()`](tdw_llm::LanguageModel::is_production_grade) must
//! return `true` for extraction to run.  A stub/echo model (the default when no
//! API key is configured) returns `false` and the stage is absent — extraction
//! never runs on a stub model.  Keyless installs therefore execute the full
//! deterministic pipeline unchanged.
//!
//! ## Cost bounds (required when enabled)
//!
//! When extraction is enabled the operator **must** configure:
//! - `max_tokens_per_doc` — per-document token cap; documents skipped when
//!   budget would be exceeded get a loud `[tdw.kg.extraction]` status note.
//! - `daily_token_budget` — daily total; exhaustion stops the stage loudly
//!   but **never** blocks the deterministic ingestion path (extraction is
//!   additive).
//!
//! ## Provenance
//!
//! Proposals carry `agent_id = "extraction:<model_id>@<doc_id>"` (validated
//! against the B9 agent-id grammar before every submission).  The
//! `extraction_confidence` prop (K-R6 seam) is encoded in the Annotation note
//! so the aggregation layer can parse it.
//!
//! ## Safety
//!
//! LLM output is **untrusted input** into graph labels.  Every extracted
//! string is passed through [`validate_extracted_label`] before use — control
//! characters and oversized strings are rejected outright (B5/B8 posture).
//! The B9 validators run again at enqueue time (and again at materialize time),
//! providing three independent validation layers.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::GraphEngine;
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

use crate::{KnowledgeError, Result, proposals::ProposalQueue};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum byte length of an extracted entity label or predicate
/// (injection-safety cap matching the B5/B8 posture).
pub const MAX_LABEL_LEN: usize = 512;

/// Maximum number of entity annotations the extraction stage enqueues per
/// document.  Bounds queue pressure from a single document; the B9 per-agent
/// cap provides a second layer.
pub const MAX_ENTITIES_PER_DOC: usize = 64;

/// Maximum number of relation annotations the extraction stage enqueues per
/// document.
pub const MAX_RELATIONS_PER_DOC: usize = 128;

// ---------------------------------------------------------------------------
// Cost-bound configuration
// ---------------------------------------------------------------------------

/// Cost-bound configuration for the LLM extraction stage.
///
/// Both fields are **required** when extraction is enabled — there is no
/// unbounded default.  The config validator rejects an enabled extraction
/// section that omits either bound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionBudget {
    /// Maximum output tokens requested per document (`max_output_tokens` in the
    /// `ChatRequest`).  A document whose token cap would be 0 is skipped with a
    /// loud note; its deterministic indexing path continues unchanged.
    pub max_tokens_per_doc: u32,
    /// Daily token budget across all extractions (input + output combined).
    /// Exhaustion stops the stage loudly; ingestion continues.
    pub daily_token_budget: u64,
}

// ---------------------------------------------------------------------------
// Extraction payload (LLM output)
// ---------------------------------------------------------------------------

/// One extracted entity from the LLM pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// The entity label as returned by the LLM (validated, trimmed).
    pub label: String,
    /// Optional entity type hint (e.g. `"person"`, `"organization"`).
    #[serde(default)]
    pub kind_hint: Option<String>,
    /// Extraction confidence in `[0.0, 1.0]` — stamped in the Annotation note
    /// so the K-R6 aggregation layer can consume it.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

const fn default_confidence() -> f64 {
    0.5
}

/// One extracted relation from the LLM pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractedRelation {
    /// The subject entity label.
    pub subject: String,
    /// The relation type (e.g. `"ceo_of"`, `"mentions"`).
    pub predicate: String,
    /// The object entity label.
    pub object: String,
    /// Extraction confidence in `[0.0, 1.0]`.
    #[serde(default = "default_confidence")]
    pub confidence: f64,
}

/// The full extraction payload returned by the LLM (parsed from JSON).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtractionPayload {
    /// Entities extracted from the document text.
    #[serde(default)]
    pub entities: Vec<ExtractedEntity>,
    /// Relations extracted from the document text.
    #[serde(default)]
    pub relations: Vec<ExtractedRelation>,
}

// ---------------------------------------------------------------------------
// Extraction result
// ---------------------------------------------------------------------------

/// Result of one extraction run against a single document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionResult {
    /// How many entity proposals were enqueued.
    pub entities_enqueued: usize,
    /// How many relation proposals were enqueued.
    pub relations_enqueued: usize,
    /// How many proposed items were skipped (label validation, budget, or B9
    /// rejection — e.g. duplicate).
    pub skipped: usize,
    /// Tokens consumed by this extraction run (input + output).
    pub tokens_consumed: u64,
    /// `true` when the daily budget was exhausted during this run.
    pub budget_exhausted: bool,
}

// ---------------------------------------------------------------------------
// Freshness status (surfaces in tdw.kg.status)
// ---------------------------------------------------------------------------

/// Observability status for the LLM extraction stage (K-M1).
///
/// Written by [`ExtractionStage`] after each document; read by
/// `KnowledgeRuntime::status()` to populate `KgStatus::extraction_freshness`.
/// `None` status field means the stage is absent (default-OFF, keyless install).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ExtractionFreshness {
    /// Stage is OFF: either `enabled = false` in config or no production-grade
    /// model is wired.  The deterministic pipeline runs unchanged.
    Disabled {
        /// Human-readable reason, e.g. `"enabled = false"` or
        /// `"stub model — is_production_grade() = false"`.
        reason: String,
    },
    /// Stage is enabled with a production-grade model but no document has been
    /// processed yet since daemon start.
    Pending,
    /// Last batch completed.
    Ok {
        /// Documents processed in the last batch.
        docs_processed: usize,
        /// Entity proposals enqueued in the last batch.
        entities_enqueued: usize,
        /// Relation proposals enqueued in the last batch.
        relations_enqueued: usize,
        /// Tokens consumed in the last batch.
        tokens_consumed: u64,
        /// Remaining daily budget after the last batch.
        budget_remaining: u64,
    },
    /// Daily budget exhausted; stage stopped early.
    BudgetExhausted {
        /// Tokens consumed at exhaustion point.
        tokens_consumed: u64,
    },
    /// LLM call or parse failure.
    Error {
        /// Error description.
        error: String,
    },
}

impl Default for ExtractionFreshness {
    fn default() -> Self {
        Self::Disabled {
            reason: "not configured".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// ExtractionStage
// ---------------------------------------------------------------------------

/// The optional LLM extraction stage (K-M1).
///
/// Holds a production-grade [`LanguageModel`], the shared [`ProposalQueue`],
/// and the graph + tag engine handles needed by the B9 admission validators.
///
/// ## Stub gate
///
/// [`is_active`](Self::is_active) returns `false` when
/// `model.is_production_grade()` is `false` — the stage is then a structural
/// no-op.  Call sites check `is_active()` before any LLM work.
pub struct ExtractionStage {
    model: Arc<dyn LanguageModel>,
    proposals: Arc<tokio::sync::Mutex<ProposalQueue>>,
    graph: Arc<dyn GraphEngine>,
    tags: Arc<dyn TagEngine>,
    budget: ExtractionBudget,
    /// Tokens consumed so far in the current day (reset via
    /// [`reset_daily_budget`](Self::reset_daily_budget)).
    tokens_used_today: u64,
    /// Shared freshness cell (K-M1) read by `tdw.kg.status`. Attached by the
    /// composition root via [`with_freshness_cell`](Self::with_freshness_cell)
    /// so production reports REAL extraction state; `None` in unit fixtures that
    /// don't assert on status. Updated after every [`extract_and_propose`] run.
    freshness_cell: Option<Arc<std::sync::Mutex<crate::ExtractionFreshness>>>,
    /// Whether the "model not production-grade, extraction inactive" warning has
    /// already been emitted. Gates [`maybe_log_inactive_warning`] so the warning
    /// fires exactly once, not on every ingest tick (MEDIUM: log spamming).
    logged_inactive_warning: bool,
}

impl ExtractionStage {
    /// Construct an extraction stage.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] when:
    /// - The model id is invalid (control characters, empty).
    /// - `budget.max_tokens_per_doc == 0`.
    /// - `budget.daily_token_budget == 0`.
    pub fn new(
        model: Arc<dyn LanguageModel>,
        proposals: Arc<tokio::sync::Mutex<ProposalQueue>>,
        graph: Arc<dyn GraphEngine>,
        tags: Arc<dyn TagEngine>,
        budget: ExtractionBudget,
    ) -> Result<Self> {
        // Validate model id (control chars, empty).
        tdw_llm::validate_model_id(model.model_id())
            .map_err(|error| KnowledgeError::Storage(error.to_string()))?;
        if budget.max_tokens_per_doc == 0 {
            return Err(KnowledgeError::Storage(
                "extraction budget: max_tokens_per_doc must be > 0".to_string(),
            ));
        }
        if budget.daily_token_budget == 0 {
            return Err(KnowledgeError::Storage(
                "extraction budget: daily_token_budget must be > 0".to_string(),
            ));
        }
        Ok(Self {
            model,
            proposals,
            graph,
            tags,
            budget,
            tokens_used_today: 0,
            freshness_cell: None,
            logged_inactive_warning: false,
        })
    }

    /// Attach the shared freshness cell (K-M1 honesty gate).
    ///
    /// The composition root (`tdw-backend`) builds one
    /// `Arc<std::sync::Mutex<ExtractionFreshness>>`, attaches it to the
    /// [`KnowledgeRuntime`] (read side: `tdw.kg.status`) AND to this stage (write
    /// side) so status reports the REAL extraction state rather than a stale
    /// default.  Without this, `tdw.kg.status` would LIE about extraction
    /// progress.
    #[must_use]
    pub fn with_freshness_cell(
        mut self,
        cell: Arc<std::sync::Mutex<crate::ExtractionFreshness>>,
    ) -> Self {
        self.freshness_cell = Some(cell);
        self
    }

    /// Write `state` into the shared freshness cell, if attached.
    ///
    /// Recovers from a poisoned mutex (a panic in another holder must not wedge
    /// the extraction stage — status honesty is best-effort and never blocks
    /// ingestion).
    fn publish_freshness(&self, state: crate::ExtractionFreshness) {
        if let Some(cell) = &self.freshness_cell {
            match cell.lock() {
                Ok(mut guard) => *guard = state,
                Err(poisoned) => *poisoned.into_inner() = state,
            }
        }
    }

    /// Log the "model not production-grade, extraction inactive" warning exactly
    /// once (MEDIUM: log spamming when model inactive).
    ///
    /// Called from the stub-gate path on every ingest; the `logged_inactive_warning`
    /// latch ensures only the first call emits the warning.
    fn maybe_log_inactive_warning(&mut self) {
        if !self.logged_inactive_warning {
            self.logged_inactive_warning = true;
            eprintln!(
                "[tdw.kg.extraction] WARN model {model_id} not production-grade — \
                 extraction stage inactive; deterministic ingestion path runs \
                 unchanged (this warning is logged once)",
                model_id = self.model.model_id(),
            );
        }
    }

    /// Whether the stage is currently operational: model is production-grade
    /// AND the daily budget is not yet exhausted.
    ///
    /// This is the structural stub gate (ER3 pattern): `false` → no LLM call is
    /// made, no proposal is submitted.  The deterministic pipeline is unaffected.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.model.is_production_grade() && self.tokens_used_today < self.budget.daily_token_budget
    }

    /// The daily token budget configuration.
    #[must_use]
    pub const fn budget(&self) -> &ExtractionBudget {
        &self.budget
    }

    /// Tokens consumed so far today.
    #[must_use]
    pub const fn tokens_used_today(&self) -> u64 {
        self.tokens_used_today
    }

    /// Reset the daily token counter (called by the composition root at midnight
    /// or on daemon restart — injected-`now` pattern, never reads a clock here).
    pub const fn reset_daily_budget(&mut self) {
        self.tokens_used_today = 0;
    }

    /// Run extraction on one document, submit proposals through the B9 gate.
    ///
    /// Returns:
    /// - `Ok(None)` when the model is not production-grade (stub gate — silent,
    ///   not an error; the deterministic path has already run).
    /// - `Ok(Some(result))` on success (including budget-exhausted early stop).
    /// - `Err` only on a hard engine failure from the proposal queue (not on
    ///   individual proposal validation failures, which are counted as `skipped`).
    ///
    /// The provenance `agent_id` is `extraction:<model_id>@<doc_id>` so the
    /// why-chain and audit trail clearly identify the extraction origin.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] on hard queue engine failure.
    pub async fn extract_and_propose(
        &mut self,
        doc_id: &str,
        text: &str,
        now: &str,
    ) -> Result<Option<ExtractionResult>> {
        // Stub gate: is_production_grade() false → silent absent (no LLM call).
        // Log the inactive warning exactly once and publish the Disabled state so
        // status reports the gap honestly rather than a stale default.
        if !self.model.is_production_grade() {
            self.maybe_log_inactive_warning();
            self.publish_freshness(crate::ExtractionFreshness::Disabled {
                reason: "stub model — is_production_grade() = false".to_string(),
            });
            return Ok(None);
        }
        // Budget gate: daily total exhausted → loud stop, set flag, publish state.
        if self.tokens_used_today >= self.budget.daily_token_budget {
            eprintln!(
                "[tdw.kg.extraction] WARN daily token budget exhausted \
                 ({used}/{cap}); extraction stage stopped for today — \
                 deterministic ingestion unaffected",
                used = self.tokens_used_today,
                cap = self.budget.daily_token_budget,
            );
            self.publish_freshness(crate::ExtractionFreshness::BudgetExhausted {
                tokens_consumed: self.tokens_used_today,
            });
            return Ok(Some(ExtractionResult {
                budget_exhausted: true,
                ..ExtractionResult::default()
            }));
        }

        // Build the extraction prompt.
        let prompt = build_extraction_prompt(text, doc_id);
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: prompt,
            }],
            max_output_tokens: self.budget.max_tokens_per_doc,
        };

        let response = self.run_model_call(request).await?;

        let tokens_this_call =
            u64::from(response.usage.input_tokens) + u64::from(response.usage.output_tokens);
        self.tokens_used_today = self.tokens_used_today.saturating_add(tokens_this_call);

        // Parse structured JSON response; malformed → skip this doc (additive).
        let payload = match parse_extraction_response(&response.message.content) {
            Ok(p) => p,
            Err(reason) => {
                eprintln!(
                    "[tdw.kg.extraction] doc {doc_id}: failed to parse LLM response \
                     ({reason}); skipping extraction — deterministic path unaffected"
                );
                self.publish_freshness(crate::ExtractionFreshness::Error {
                    error: format!("doc {doc_id}: parse failure: {reason}"),
                });
                return Ok(Some(ExtractionResult {
                    tokens_consumed: tokens_this_call,
                    ..ExtractionResult::default()
                }));
            }
        };

        // Build the provenance agent_id: extraction:<model_id>@<doc_id>.
        // The agent-id grammar is [A-Za-z0-9:._-]+; sanitize model_id and doc_id.
        let agent_id = build_agent_id(self.model.model_id(), doc_id);

        let mut result = ExtractionResult {
            tokens_consumed: tokens_this_call,
            ..ExtractionResult::default()
        };

        {
            let ctx = SubmitCtx {
                agent_id: &agent_id,
                doc_id,
                now,
                graph: &self.graph,
                tags: &self.tags,
            };

            let mut queue = self.proposals.lock().await;
            process_entities(&mut queue, &ctx, payload.entities, &mut result).await;
            process_relations(&mut queue, &ctx, payload.relations, &mut result).await;
        }

        self.publish_post_run_freshness(doc_id, tokens_this_call, &mut result);

        Ok(Some(result))
    }

    /// Run the (synchronous, blocking) model call off the async path.
    ///
    /// RUNTIME HAZARD FIX (Gemini HIGH #1): `LanguageModel::complete` is a
    /// synchronous, network-blocking call. Running it directly on the async path
    /// would block a tokio runtime worker thread. This offloads it to the
    /// blocking pool via `spawn_blocking` (the model is `Arc<dyn LanguageModel>`:
    /// `Send + Sync`, so the clone moves cleanly into the closure). On any
    /// failure the freshness cell is updated to `Error` before returning.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Storage`] if the model errors or the blocking
    /// task panics/cancels.
    async fn run_model_call(&self, request: ChatRequest) -> Result<tdw_llm::ChatResponse> {
        let model = Arc::clone(&self.model);
        match tokio::task::spawn_blocking(move || model.complete(request)).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => {
                let error = error.to_string();
                self.publish_freshness(crate::ExtractionFreshness::Error {
                    error: error.clone(),
                });
                Err(KnowledgeError::Storage(error))
            }
            Err(join_error) => {
                // The blocking task panicked or was cancelled — surface as a hard
                // engine failure (the deterministic path has already run).
                let error = format!("extraction model task failed: {join_error}");
                self.publish_freshness(crate::ExtractionFreshness::Error {
                    error: error.clone(),
                });
                Err(KnowledgeError::Storage(error))
            }
        }
    }

    /// Budget accounting AFTER processing one document.
    ///
    /// If this run pushed `tokens_used_today` at/over the daily budget, the cap
    /// is now enforced LOUDLY (Gemini HIGH #2): the exhaustion flag is set on the
    /// result so callers observe it without a second call, and `BudgetExhausted`
    /// is published. Otherwise a normal `Ok` with remaining budget is published.
    fn publish_post_run_freshness(
        &self,
        doc_id: &str,
        tokens_this_call: u64,
        result: &mut ExtractionResult,
    ) {
        if self.tokens_used_today >= self.budget.daily_token_budget {
            result.budget_exhausted = true;
            eprintln!(
                "[tdw.kg.extraction] WARN daily token budget reached after processing \
                 doc {doc_id} ({used}/{cap}) — extraction stage will stop until reset",
                used = self.tokens_used_today,
                cap = self.budget.daily_token_budget,
            );
            self.publish_freshness(crate::ExtractionFreshness::BudgetExhausted {
                tokens_consumed: self.tokens_used_today,
            });
        } else {
            self.publish_freshness(crate::ExtractionFreshness::Ok {
                docs_processed: 1,
                entities_enqueued: result.entities_enqueued,
                relations_enqueued: result.relations_enqueued,
                tokens_consumed: tokens_this_call,
                budget_remaining: self
                    .budget
                    .daily_token_budget
                    .saturating_sub(self.tokens_used_today),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Shared submission context threaded through the entity/relation loops.
struct SubmitCtx<'a> {
    agent_id: &'a str,
    doc_id: &'a str,
    now: &'a str,
    graph: &'a Arc<dyn GraphEngine>,
    tags: &'a Arc<dyn TagEngine>,
}

/// Submit one Annotation proposal; increment the appropriate counter.
///
/// Validation failures are counted as `skipped` (they are expected: a
/// proposal for a document node that the deterministic path has not yet
/// written will fail the B9 endpoint existence check).  Hard engine errors
/// are also counted as skipped (additive contract — extraction never fails
/// ingestion).
async fn submit_annotation(
    queue: &mut ProposalQueue,
    ctx: &SubmitCtx<'_>,
    note: String,
    enqueued: &mut usize,
    skipped: &mut usize,
) {
    use crate::proposals::ProposalKind;
    let entity_id = format!("document:{}", ctx.doc_id);
    match queue
        .submit(
            ctx.agent_id,
            Adaptivity::Learning,
            ProposalKind::Annotation { entity_id, note },
            ctx.graph,
            ctx.tags,
            ctx.now,
        )
        .await
    {
        Ok(_) => *enqueued += 1,
        Err(error) => {
            eprintln!(
                "[tdw.kg.extraction] agent {}: annotation proposal skipped: {error}",
                ctx.agent_id
            );
            *skipped += 1;
        }
    }
}

/// Process entity proposals from the extraction payload.
async fn process_entities(
    queue: &mut ProposalQueue,
    ctx: &SubmitCtx<'_>,
    entities: Vec<ExtractedEntity>,
    result: &mut ExtractionResult,
) {
    for entity in entities.into_iter().take(MAX_ENTITIES_PER_DOC) {
        match validate_extracted_label(&entity.label) {
            Ok(safe_label) => {
                let note = build_entity_note(
                    &safe_label,
                    entity.kind_hint.as_deref(),
                    entity.confidence,
                    ctx.doc_id,
                );
                submit_annotation(
                    queue,
                    ctx,
                    note,
                    &mut result.entities_enqueued,
                    &mut result.skipped,
                )
                .await;
            }
            Err(_) => result.skipped += 1,
        }
    }
}

/// Process relation proposals from the extraction payload.
async fn process_relations(
    queue: &mut ProposalQueue,
    ctx: &SubmitCtx<'_>,
    relations: Vec<ExtractedRelation>,
    result: &mut ExtractionResult,
) {
    for relation in relations.into_iter().take(MAX_RELATIONS_PER_DOC) {
        match (
            validate_extracted_label(&relation.subject),
            validate_extracted_label(&relation.predicate),
            validate_extracted_label(&relation.object),
        ) {
            (Ok(subject), Ok(predicate), Ok(object)) => {
                let note = build_relation_note(
                    &subject,
                    &predicate,
                    &object,
                    relation.confidence,
                    ctx.doc_id,
                );
                submit_annotation(
                    queue,
                    ctx,
                    note,
                    &mut result.relations_enqueued,
                    &mut result.skipped,
                )
                .await;
            }
            _ => result.skipped += 1,
        }
    }
}

/// Build the extraction prompt sent to the LLM.
///
/// The prompt requests a structured JSON object with `entities` and `relations`
/// arrays.  The format is minimal and stable — a small, injection-safe schema
/// that the response parser can validate.
fn build_extraction_prompt(text: &str, doc_id: &str) -> String {
    // Strip control characters from the document text before embedding in the
    // prompt (injection safety — the text is untrusted external content).
    let safe_text: String = text
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    format!(
        "Extract entities and relations from the following document text.\n\
         Document id: {doc_id}\n\
         ---\n\
         {safe_text}\n\
         ---\n\
         Respond with ONLY a JSON object of the form:\n\
         {{\"entities\":[{{\"label\":\"...\",\"kind_hint\":\"...\",\"confidence\":0.9}},...],\
         \"relations\":[{{\"subject\":\"...\",\"predicate\":\"...\",\"object\":\"...\",\
         \"confidence\":0.8}},...]}}\n\
         Rules:\n\
         - label/subject/predicate/object: plain ASCII or unicode text, no control chars, \
           max {MAX_LABEL_LEN} bytes.\n\
         - confidence: float in [0.0, 1.0].\n\
         - Omit arrays when empty.\n\
         - No markdown, no explanation — raw JSON only."
    )
}

/// Parse the LLM's JSON response into an [`ExtractionPayload`].
///
/// A lenient parse: extra fields are ignored; missing arrays default to empty.
/// Returns `Err(reason)` when the response is not valid JSON or lacks the
/// expected top-level object shape.
fn parse_extraction_response(response: &str) -> std::result::Result<ExtractionPayload, String> {
    // Find the first `{` and last `}` to strip any surrounding whitespace or
    // markdown fences that a chatty model might add.
    let trimmed = response.trim();
    let start = trimmed
        .find('{')
        .ok_or_else(|| "no JSON object found in response".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "no closing brace found in response".to_string())?;
    if end < start {
        return Err("malformed JSON: closing brace before opening brace".to_string());
    }
    let json_slice = &trimmed[start..=end];
    serde_json::from_str::<ExtractionPayload>(json_slice).map_err(|error| error.to_string())
}

/// Build the `agent_id` for extraction proposals.
///
/// Format: `extraction:<sanitized_model_id>.<sanitized_doc_id>`.
/// The B9 grammar is `[A-Za-z0-9:._-]+`; both components are sanitized and
/// separated by `.` (which is in the allowed set).  The result is truncated to
/// `proposals::MAX_AGENT_ID_LEN` so the B9 validator never rejects it.
fn build_agent_id(model_id: &str, doc_id: &str) -> String {
    use crate::proposals::MAX_AGENT_ID_LEN;
    // Sanitize: keep only [A-Za-z0-9._-], replace anything else with `-`.
    // Note: `:` is allowed in agent ids but we don't use it in these components
    // to keep the separator (`:` prefix on "extraction") unambiguous.
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    // Separator is `.` — in grammar, unambiguous, not `:` (reserved for
    // the prefix) and not `@` (not in grammar).
    let raw = format!("extraction:{}.{}", sanitize(model_id), sanitize(doc_id));
    // Truncate to MAX_AGENT_ID_LEN (byte-safe: all chars are ASCII after sanitize).
    raw[..raw.len().min(MAX_AGENT_ID_LEN)].to_string()
}

/// Quote and escape a field value for the space/`=`-delimited note format.
///
/// The note format is `key=value key=value ...`; an unquoted value containing a
/// space, `=`, or quote would corrupt the parse (and entity text from an LLM is
/// untrusted).  This wraps the value in double quotes and backslash-escapes any
/// embedded backslash or double-quote so the K-R6 aggregation parser can recover
/// the exact field boundaries.  (Control characters are already rejected by
/// [`validate_extracted_label`] upstream.)
fn quote_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Clamp an LLM-provided confidence into `[0.0, 1.0]`.
///
/// LLM output is untrusted: a model may emit `1.7`, `-0.3`, or `NaN`. NaN maps
/// to the neutral default; finite values are clamped to the unit interval before
/// they are stamped into the note (and later consumed by K-R6 aggregation).
const fn clamp_confidence(confidence: f64) -> f64 {
    if confidence.is_nan() {
        default_confidence()
    } else {
        confidence.clamp(0.0, 1.0)
    }
}

/// Build the Annotation note for an extracted entity.
///
/// Format is machine-parseable by K-R6 aggregation; fields are quoted/escaped so
/// commas, spaces, `=`, or quotes in entity text cannot corrupt the note:
/// `km1:entity label="<label>" kind="<kind|none>" confidence=<conf> src="<doc_id>"`
fn build_entity_note(
    label: &str,
    kind_hint: Option<&str>,
    confidence: f64,
    doc_id: &str,
) -> String {
    format!(
        "km1:entity label={label} kind={kind} confidence={confidence:.4} src={src}",
        label = quote_field(label),
        kind = quote_field(kind_hint.unwrap_or("none")),
        confidence = clamp_confidence(confidence),
        src = quote_field(doc_id),
    )
}

/// Build the Annotation note for an extracted relation.
///
/// Format (quoted/escaped fields):
/// `km1:relation subject="<s>" predicate="<p>" object="<o>" confidence=<c> src="<doc_id>"`
fn build_relation_note(
    subject: &str,
    predicate: &str,
    object: &str,
    confidence: f64,
    doc_id: &str,
) -> String {
    format!(
        "km1:relation subject={subject} predicate={predicate} object={object} \
         confidence={confidence:.4} src={src}",
        subject = quote_field(subject),
        predicate = quote_field(predicate),
        object = quote_field(object),
        confidence = clamp_confidence(confidence),
        src = quote_field(doc_id),
    )
}

/// Validate a label extracted by the LLM.
///
/// Rejects:
/// - Empty or whitespace-only strings.
/// - Strings longer than [`MAX_LABEL_LEN`] bytes.
/// - Strings containing control characters (injection-safety, B5/B8 posture).
///
/// Returns the trimmed label on success.
///
/// # Errors
///
/// Returns a [`KnowledgeError::Storage`] with a description when validation fails.
pub fn validate_extracted_label(label: &str) -> Result<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err(KnowledgeError::Storage(
            "extracted label must not be empty".to_string(),
        ));
    }
    if trimmed.len() > MAX_LABEL_LEN {
        return Err(KnowledgeError::Storage(format!(
            "extracted label is too long ({} bytes, max {MAX_LABEL_LEN})",
            trimmed.len()
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(KnowledgeError::Storage(
            "extracted label must not contain control characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tdw_llm::{ChatMessage as LlmChatMessage, ChatRequest, ChatResponse, MessageRole, Usage};
    use tdw_storage_graph::{GraphTagEngine, InMemoryGraphEngine};

    use super::*;
    use crate::proposals::{ProposalKind, ProposalQueue};

    // ── Fake production-grade model (is_production_grade = true) ─────────────

    struct FakeProductionModel {
        response: String,
        input_tokens: u32,
        output_tokens: u32,
    }

    impl tdw_llm::LanguageModel for FakeProductionModel {
        fn model_id(&self) -> &'static str {
            "fake-prod-v1"
        }

        fn is_production_grade(&self) -> bool {
            true
        }

        fn complete(&self, _request: ChatRequest) -> tdw_llm::Result<ChatResponse> {
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: LlmChatMessage {
                    role: MessageRole::Assistant,
                    content: self.response.clone(),
                },
                usage: Usage {
                    input_tokens: self.input_tokens,
                    output_tokens: self.output_tokens,
                },
            })
        }
    }

    // ── Stub model (is_production_grade = false, the default) ─────────────

    struct StubModel;

    impl tdw_llm::LanguageModel for StubModel {
        fn model_id(&self) -> &'static str {
            "stub-echo"
        }

        // default is_production_grade() = false
        fn complete(&self, request: ChatRequest) -> tdw_llm::Result<ChatResponse> {
            let content = request
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: LlmChatMessage {
                    role: MessageRole::Assistant,
                    content,
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    fn make_engines() -> (Arc<dyn tdw_core::GraphEngine>, Arc<dyn tdw_tags::TagEngine>) {
        let graph: Arc<dyn tdw_core::GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let tags: Arc<dyn tdw_tags::TagEngine> =
            Arc::new(GraphTagEngine::new(InMemoryGraphEngine::default()));
        (graph, tags)
    }

    fn make_queue() -> Arc<tokio::sync::Mutex<ProposalQueue>> {
        Arc::new(tokio::sync::Mutex::new(ProposalQueue::default()))
    }

    fn budget(tokens_per_doc: u32, daily: u64) -> ExtractionBudget {
        ExtractionBudget {
            max_tokens_per_doc: tokens_per_doc,
            daily_token_budget: daily,
        }
    }

    // ── Gate: stub model → extraction never runs ──────────────────────────

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    async fn stub_model_never_extracts() {
        let (graph, tags) = make_engines();
        let queue = make_queue();
        let mut stage = ExtractionStage::new(
            Arc::new(StubModel),
            Arc::clone(&queue),
            graph,
            tags,
            budget(512, 100_000),
        )
        .expect("stage constructs");

        assert!(!stage.is_active(), "stub model must not be active");

        let result = stage
            .extract_and_propose("doc-1", "Apple is a company.", "2026-06-12")
            .await
            .expect("no error from stub gate");

        assert!(result.is_none(), "stub gate must return None");
        let q = queue.lock().await;
        assert!(
            q.list(None, None).proposals.is_empty(),
            "stub model must enqueue nothing"
        );
    }

    // ── Golden fixture: fake prod model → proposals enqueued ─────────────

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    async fn production_model_enqueues_entity_proposals() {
        let golden_json = serde_json::json!({
            "entities": [
                {"label": "Apple Inc", "kind_hint": "organization", "confidence": 0.95},
                {"label": "Tim Cook", "kind_hint": "person", "confidence": 0.88}
            ],
            "relations": []
        })
        .to_string();

        let (graph, tags) = make_engines();
        let queue = make_queue();

        // The B9 Annotation validator requires the entity_id node to exist.
        // The deterministic indexer writes document:<id> before extraction runs;
        // here we pre-seed the node to replicate that contract.
        {
            use tdw_core::GraphNode;
            use tdw_taxonomy::EntityKind;
            graph
                .upsert_nodes(vec![GraphNode {
                    id: "document:doc-golden".to_string(),
                    kind: EntityKind::Document,
                    label: "Golden fixture doc".to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .expect("seed document node");
        }

        let mut stage = ExtractionStage::new(
            Arc::new(FakeProductionModel {
                response: golden_json,
                input_tokens: 50,
                output_tokens: 30,
            }),
            Arc::clone(&queue),
            Arc::clone(&graph),
            tags,
            budget(512, 100_000),
        )
        .expect("stage constructs");

        assert!(stage.is_active(), "prod model must be active");

        let result = stage
            .extract_and_propose(
                "doc-golden",
                "Apple Inc was founded by Tim Cook.",
                "2026-06-12",
            )
            .await
            .expect("no error");

        let result = result.expect("prod model returns Some");
        assert_eq!(result.entities_enqueued, 2, "two entities enqueued");
        assert_eq!(result.relations_enqueued, 0);
        assert!(!result.budget_exhausted);
        assert_eq!(result.tokens_consumed, 80); // 50 + 30

        // Proposals must be in the queue, NOT directly written to the graph.
        let q = queue.lock().await;
        let page = q.list(None, None);
        assert_eq!(page.proposals.len(), 2, "two proposals in queue");
        // All must be Annotation kind with the extraction provenance id.
        for proposal in &page.proposals {
            assert!(
                matches!(proposal.kind, ProposalKind::Annotation { .. }),
                "extracted proposals must be Annotations"
            );
            assert!(
                proposal.agent_id.starts_with("extraction:"),
                "agent_id must carry extraction provenance: {}",
                proposal.agent_id
            );
        }
    }

    // ── Budget exhaustion stops stage, ingestion continues ────────────────

    #[tokio::test]
    async fn budget_exhaustion_stops_stage_loudly() {
        let golden_json = r#"{"entities":[{"label":"X","confidence":0.9}],"relations":[]}"#;
        let (graph, tags) = make_engines();
        let queue = make_queue();

        // Seed document node.
        {
            use tdw_core::GraphNode;
            use tdw_taxonomy::EntityKind;
            graph
                .upsert_nodes(vec![GraphNode {
                    id: "document:doc-budget".to_string(),
                    kind: EntityKind::Document,
                    label: "Budget doc".to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .expect("seed");
        }

        // Very tight budget: 1 token total.
        let mut stage = ExtractionStage::new(
            Arc::new(FakeProductionModel {
                response: golden_json.to_string(),
                input_tokens: 50,
                output_tokens: 30,
            }),
            Arc::clone(&queue),
            graph,
            tags,
            budget(512, 1), // 1-token daily budget
        )
        .expect("stage constructs");

        // First call: tokens_used=0 < budget=1 → runs, consumes 80 tokens. The
        // run crosses the cap, so the exhaustion flag must be set LOUDLY on this
        // result (cost cap enforced — Gemini HIGH #2), not silently deferred.
        let r1 = stage
            .extract_and_propose("doc-budget", "X is notable.", "2026-06-12")
            .await
            .expect("no error");
        let r1 = r1.expect("returns Some");
        assert!(
            r1.budget_exhausted,
            "first call crosses the cap → exhaustion flag set on this result"
        );
        assert!(!stage.is_active(), "stage inactive after crossing the cap");

        // Second call: tokens_used=80 >= budget=1 → budget exhausted, early return.
        let r2 = stage
            .extract_and_propose("doc-budget", "X again.", "2026-06-12")
            .await
            .expect("no error");
        let r2 = r2.expect("returns Some");
        assert!(r2.budget_exhausted, "second call must be budget-exhausted");
        assert!(!stage.is_active(), "stage inactive after exhaustion");
    }

    // ── Keyless install: extraction off → deterministic path unchanged ────

    #[tokio::test]
    async fn keyless_install_deterministic_path_unaffected() {
        // A keyless install has no ExtractionStage at all (None in indexer).
        // This test verifies that the deterministic indexer path works identically
        // with extraction off — no proposals, no errors.
        use crate::indexer::KnowledgeIndexer;
        use crate::{KnowledgeDocument, KnowledgeIndex};
        use tdw_kg::{Entity, EntityKind};

        let mut indexer = KnowledgeIndexer::new(KnowledgeIndex::default());
        let doc = KnowledgeDocument {
            id: "keyless-doc".to_string(),
            body: "Deterministic body text.".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: Vec::new(),
            },
            tags: Vec::new(),
            source: None,
            plane: None,
            as_of: None,
            mentions: Vec::new(),
        };
        let outcome = indexer
            .index_at(doc, "2026-06-12")
            .await
            .expect("keyless indexing succeeds");
        assert_eq!(outcome, crate::indexer::IndexOutcome::Indexed);
    }

    // ── Injection safety: malicious label rejected ─────────────────────────

    #[test]
    fn injection_malicious_label_rejected() {
        // Control character in label.
        assert!(
            validate_extracted_label("malicious\x00label").is_err(),
            "null byte must be rejected"
        );
        assert!(
            validate_extracted_label("escape\x1b[31mred").is_err(),
            "ANSI escape must be rejected"
        );
        // Oversized label.
        let long: String = "a".repeat(MAX_LABEL_LEN + 1);
        assert!(
            validate_extracted_label(&long).is_err(),
            "over-length label must be rejected"
        );
        // Valid label passes.
        assert!(validate_extracted_label("Apple Inc").is_ok());
        assert!(validate_extracted_label("  trimmed  ").is_ok());
    }

    // ── parse_extraction_response: handles whitespace, markdown fences ────

    #[test]
    fn parse_extraction_response_strips_markdown_fences() {
        let response = "```json\n{\"entities\":[{\"label\":\"Tesla\",\"confidence\":0.9}],\
                        \"relations\":[]}\n```";
        let payload = parse_extraction_response(response).expect("parses");
        assert_eq!(payload.entities.len(), 1);
        assert_eq!(payload.entities[0].label, "Tesla");
    }

    #[test]
    fn parse_extraction_response_empty_arrays_default() {
        let response = r#"{"entities":[],"relations":[]}"#;
        let payload = parse_extraction_response(response).expect("parses");
        assert!(payload.entities.is_empty());
        assert!(payload.relations.is_empty());
    }

    #[test]
    fn parse_extraction_response_missing_arrays_default() {
        let payload = parse_extraction_response("{}").expect("parses empty object");
        assert!(payload.entities.is_empty());
        assert!(payload.relations.is_empty());
    }

    // ── Note formatting: fields quoted/escaped, confidence clamped ─────────

    #[test]
    fn entity_note_quotes_fields_and_clamps_confidence() {
        // A label with a space, `=`, and a double-quote — all of which would
        // corrupt the unquoted space/`=`-delimited note format.
        let note = build_entity_note(
            r#"Acme = "Corp", Inc"#,
            Some("organization"),
            1.7, // out of range → must clamp to 1.0
            "doc with space",
        );
        // Field is wrapped in quotes with the embedded quote backslash-escaped.
        assert!(
            note.contains(r#"label="Acme = \"Corp\", Inc""#),
            "label must be quoted+escaped: {note}"
        );
        // Confidence clamped to 1.0000 (not 1.7000).
        assert!(
            note.contains("confidence=1.0000"),
            "confidence must clamp to 1.0: {note}"
        );
        // src quoted so the trailing space in doc id can't be mis-parsed.
        assert!(
            note.contains(r#"src="doc with space""#),
            "src must be quoted: {note}"
        );
    }

    #[test]
    fn relation_note_clamps_negative_and_nan_confidence() {
        let neg = build_relation_note("a", "rel", "b", -0.5, "d");
        assert!(
            neg.contains("confidence=0.0000"),
            "negative clamps to 0: {neg}"
        );
        let nan = build_relation_note("a", "rel", "b", f64::NAN, "d");
        // NaN maps to the neutral default (0.5).
        assert!(
            nan.contains("confidence=0.5000"),
            "NaN maps to default: {nan}"
        );
    }

    #[test]
    fn quote_field_escapes_backslash_and_quote() {
        assert_eq!(quote_field("plain"), r#""plain""#);
        assert_eq!(quote_field(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_field(r"a\b"), r#""a\\b""#);
    }

    #[test]
    fn clamp_confidence_bounds_and_nan() {
        assert!((clamp_confidence(0.42) - 0.42).abs() < f64::EPSILON);
        assert!((clamp_confidence(2.0) - 1.0).abs() < f64::EPSILON);
        assert!((clamp_confidence(-1.0) - 0.0).abs() < f64::EPSILON);
        assert!((clamp_confidence(f64::NAN) - default_confidence()).abs() < f64::EPSILON);
    }

    // ── Freshness cell: production path publishes REAL state ───────────────

    #[tokio::test]
    async fn freshness_cell_updated_after_run() {
        let golden_json = serde_json::json!({
            "entities": [{"label": "Nvidia", "kind_hint": "organization", "confidence": 0.9}],
            "relations": []
        })
        .to_string();

        let (graph, tags) = make_engines();
        let queue = make_queue();
        {
            use tdw_core::GraphNode;
            use tdw_taxonomy::EntityKind;
            graph
                .upsert_nodes(vec![GraphNode {
                    id: "document:doc-fresh".to_string(),
                    kind: EntityKind::Document,
                    label: "Freshness doc".to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .expect("seed");
        }

        let cell = Arc::new(std::sync::Mutex::new(ExtractionFreshness::Pending));
        let mut stage = ExtractionStage::new(
            Arc::new(FakeProductionModel {
                response: golden_json,
                input_tokens: 10,
                output_tokens: 5,
            }),
            Arc::clone(&queue),
            graph,
            tags,
            budget(512, 100_000),
        )
        .expect("stage constructs")
        .with_freshness_cell(Arc::clone(&cell));

        stage
            .extract_and_propose("doc-fresh", "Nvidia makes GPUs.", "2026-06-12")
            .await
            .expect("no error");

        // The cell must now report REAL Ok state, not a stale Pending — status
        // would otherwise LIE about extraction progress (Gemini HIGH #3).
        let freshness = cell.lock().expect("not poisoned").clone();
        match freshness {
            ExtractionFreshness::Ok {
                docs_processed,
                entities_enqueued,
                tokens_consumed,
                ..
            } => {
                assert_eq!(docs_processed, 1);
                assert_eq!(entities_enqueued, 1);
                assert_eq!(tokens_consumed, 15);
            }
            other => panic!("expected Ok freshness, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn freshness_cell_reports_disabled_for_stub_model() {
        let (graph, tags) = make_engines();
        let queue = make_queue();
        let cell = Arc::new(std::sync::Mutex::new(ExtractionFreshness::Pending));
        let mut stage = ExtractionStage::new(
            Arc::new(StubModel),
            Arc::clone(&queue),
            graph,
            tags,
            budget(512, 100_000),
        )
        .expect("stage constructs")
        .with_freshness_cell(Arc::clone(&cell));

        let result = stage
            .extract_and_propose("doc-stub", "text", "2026-06-12")
            .await
            .expect("no error");
        assert!(result.is_none(), "stub gate returns None");
        // Stub model → status must honestly report Disabled, not stale Pending.
        let freshness = cell.lock().expect("not poisoned").clone();
        assert!(
            matches!(freshness, ExtractionFreshness::Disabled { .. }),
            "stub model must publish Disabled freshness, got {freshness:?}"
        );
    }

    // ── build_agent_id: sanitized, bounded ────────────────────────────────

    #[test]
    fn build_agent_id_sanitizes_and_bounds() {
        use crate::proposals::MAX_AGENT_ID_LEN;
        let id = build_agent_id("claude-3-opus", "doc-123");
        assert!(
            id.starts_with("extraction:"),
            "agent_id must start with extraction:"
        );
        assert!(
            id.len() <= MAX_AGENT_ID_LEN,
            "agent_id must be bounded: {id}"
        );
        // No semicolons (B9 provenance separator).
        assert!(!id.contains(';'), "no semicolons in agent_id");
        // No `@` — not in the B9 grammar; separator is `.`.
        assert!(!id.contains('@'), "no @ in agent_id — not in B9 grammar");
        // All chars strictly in [A-Za-z0-9:._-].
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-');
        assert!(
            id.chars().all(allowed),
            "agent_id chars must be in grammar: {id}"
        );
    }

    // ── ExtractionStage::new rejects zero budgets ─────────────────────────

    #[test]
    fn stage_rejects_zero_token_budgets() {
        let (graph, tags) = make_engines();
        let queue = make_queue();
        // zero max_tokens_per_doc
        assert!(
            ExtractionStage::new(
                Arc::new(StubModel),
                Arc::clone(&queue),
                Arc::clone(&graph),
                Arc::clone(&tags),
                budget(0, 100_000),
            )
            .is_err(),
            "zero max_tokens_per_doc must fail"
        );
        // zero daily_token_budget
        assert!(
            ExtractionStage::new(
                Arc::new(StubModel),
                Arc::clone(&queue),
                Arc::clone(&graph),
                Arc::clone(&tags),
                budget(512, 0),
            )
            .is_err(),
            "zero daily_token_budget must fail"
        );
    }

    // ── Why-chain: proposal provenance carries extraction marker ──────────

    #[tokio::test]
    #[allow(clippy::significant_drop_tightening)]
    async fn why_chain_shows_extraction_provenance() {
        let golden_json = serde_json::json!({
            "entities": [{"label": "Microsoft", "kind_hint": "organization", "confidence": 0.92}],
            "relations": []
        })
        .to_string();

        let (graph, tags) = make_engines();
        let queue = make_queue();

        {
            use tdw_core::GraphNode;
            use tdw_taxonomy::EntityKind;
            graph
                .upsert_nodes(vec![GraphNode {
                    id: "document:why-doc".to_string(),
                    kind: EntityKind::Document,
                    label: "Why-chain doc".to_string(),
                    aliases: Vec::new(),
                    props: serde_json::Value::Null,
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .expect("seed");
        }

        let mut stage = ExtractionStage::new(
            Arc::new(FakeProductionModel {
                response: golden_json,
                input_tokens: 40,
                output_tokens: 20,
            }),
            Arc::clone(&queue),
            graph,
            tags,
            budget(512, 100_000),
        )
        .expect("stage constructs");

        stage
            .extract_and_propose("why-doc", "Microsoft dominates cloud.", "2026-06-12")
            .await
            .expect("no error");

        let q = queue.lock().await;
        let page = q.list(None, None);
        assert!(!page.proposals.is_empty());
        let proposal = &page.proposals[0];
        // Provenance chain: agent_id starts with "extraction:" → identifiable
        // in why-chain as LLM-extracted.
        assert!(
            proposal.agent_id.starts_with("extraction:"),
            "why-chain must show extraction provenance"
        );
        // History must record the proposal creation.
        assert!(
            !proposal.history.is_empty(),
            "proposal must have audit history"
        );
        // The proposal is Validated (went through B9 automated validators).
        assert_eq!(
            proposal.status,
            tdw_taxonomy::ValidationStatus::Validated,
            "extracted proposal must be Validated, not raw"
        );
        // The proposal is NOT materialized (eval gate has not run yet).
        assert!(
            !proposal.materialized,
            "extracted proposal must not be auto-materialized"
        );
    }
}
