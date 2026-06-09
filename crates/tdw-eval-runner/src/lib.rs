#![forbid(unsafe_code)]

//! The evaluation runner: execute each [`EvalCase`] through a [`LanguageModel`] and
//! score it deterministically.
//!
//! Phase C1 replaces the prior coverage-only metric with real per-case execution. The
//! runner owns an `Arc<dyn LanguageModel>` (default: an offline deterministic stub) and,
//! for every case, builds a [`ChatRequest`] from the agent/skill context plus the case
//! prompt, calls [`LanguageModel::complete`], and applies a single honest scoring rule
//! (see [`score_case`]). Aggregated `case_count`/`passed`/`pass_rate` metrics drive the
//! run `status`. The feedback-to-skill-quality mutation is a later phase; this phase only
//! produces real metrics.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_agent::{AgentCard, ContentRef, EvalCase, EvalMetric, EvalRunRequest};
use tdw_agent_store::{AgentStore, StoredEvalRun};
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};

/// Output-token budget requested per case. Generous enough that the deterministic stub
/// (and real clients) never truncate the canned/echoed answer used for scoring.
const MAX_OUTPUT_TOKENS: u32 = 1024;

/// Minimum `pass_rate` for a `success` status: at least half the cases must pass.
/// Below this the run is `failed`. Aligned with the feedback disable threshold
/// (`tdw-backend` `FEEDBACK_DISABLE_BELOW = 0.5`), so a `success` run is exactly
/// one whose skills are not disabled. (The previous `0.0` made any single passing
/// case — out of however many — a "success", which the audit flagged as hollow.)
const PASS_RATE_SUCCESS_THRESHOLD: f64 = 0.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRunOutcome {
    pub run_id: String,
    pub status: String,
    pub metrics: Vec<EvalMetric>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalRunError {
    InvalidRequest,
    StoreRejected,
}

/// Executes evaluation cases through a [`LanguageModel`].
///
/// Holds the model behind an `Arc` so the same runner can be cloned/shared and so callers
/// inject the offline stub by default (no network in CI). Construct via [`EvalRunner::new`].
pub struct EvalRunner {
    model: Arc<dyn LanguageModel>,
}

impl EvalRunner {
    /// Build a runner that executes cases through `model`.
    #[must_use]
    pub fn new(model: Arc<dyn LanguageModel>) -> Self {
        Self { model }
    }

    /// Execute every case in `request` through the model, score each, persist the run, and
    /// return the aggregated outcome.
    ///
    /// Infallible by design: a model error on a case is recorded as a failed case rather
    /// than aborting the run, so partial results are still scored and stored.
    pub fn run(&self, request: EvalRunRequest, store: &mut AgentStore) -> EvalRunOutcome {
        let card = store.agent(&request.agent_id).cloned();
        let system_prompt = card.as_ref().map(agent_system_prompt);

        let passed = request
            .cases
            .iter()
            .filter(|case| self.run_case(case, system_prompt.as_deref()))
            .count();
        let case_count = request.cases.len();

        // Case/pass counts are reported as f64 metric values; realistic counts are far
        // within f64's exact-integer range.
        #[allow(clippy::cast_precision_loss)]
        let case_count_f64 = case_count as f64;
        #[allow(clippy::cast_precision_loss)]
        let passed_f64 = passed as f64;
        let pass_rate = if case_count > 0 {
            passed_f64 / case_count_f64
        } else {
            0.0
        };

        let metrics = vec![
            EvalMetric {
                metric_name: "case_count".to_string(),
                metric_value: case_count_f64,
            },
            EvalMetric {
                metric_name: "passed".to_string(),
                metric_value: passed_f64,
            },
            EvalMetric {
                metric_name: "pass_rate".to_string(),
                metric_value: pass_rate,
            },
        ];
        let status = if pass_rate >= PASS_RATE_SUCCESS_THRESHOLD {
            "success"
        } else {
            "failed"
        }
        .to_string();
        let outcome = EvalRunOutcome {
            run_id: request.run_id.clone(),
            status: status.clone(),
            metrics: metrics.clone(),
        };
        store.record_eval_run(StoredEvalRun {
            request,
            metrics,
            status,
            // The runner records the run; feedback (and the skill backlink) is attached by
            // the backend layer in a later, gated pass.
            updated_skills: Vec::new(),
        });
        outcome
    }

    /// Execute one case and return whether it passed.
    ///
    /// A model error counts as a failure (returns `false`) — see [`Self::run`].
    fn run_case(&self, case: &EvalCase, system_prompt: Option<&str>) -> bool {
        let request = build_chat_request(case, system_prompt);
        match self.model.complete(request) {
            Ok(response) => score_case(case, &response.message.content),
            Err(_) => false,
        }
    }

    /// Validate the request, run it, and confirm the run was persisted.
    ///
    /// # Errors
    ///
    /// Returns [`EvalRunError::InvalidRequest`] if the request is malformed, or
    /// [`EvalRunError::StoreRejected`] if the run did not persist.
    pub fn try_run(
        &self,
        request: EvalRunRequest,
        store: &mut AgentStore,
    ) -> Result<EvalRunOutcome, EvalRunError> {
        validate_request(&request)?;
        let outcome = self.run(request, store);
        if store.eval_run(&outcome.run_id).is_none() {
            return Err(EvalRunError::StoreRejected);
        }
        Ok(outcome)
    }
}

/// Build the system prompt that gives the model the agent's grounding context: the agent's
/// title/description, each skill's title/description and declared input/output JSON
/// Schemas, and — crucially for scoring — the URIs of the agent's own `content_refs` (the
/// reference material the agent is allowed to ground answers in). Deterministic for a given
/// card.
fn agent_system_prompt(card: &AgentCard) -> String {
    let mut lines = Vec::new();
    let title = card
        .meta
        .base
        .title
        .as_deref()
        .unwrap_or(&card.meta.base.name);
    lines.push(format!("You are the agent \"{title}\"."));
    if let Some(description) = card.meta.base.description.as_deref() {
        lines.push(description.to_string());
    }
    for skill in &card.skills {
        let skill_title = skill
            .meta
            .base
            .title
            .as_deref()
            .unwrap_or(&skill.meta.base.name);
        lines.push(format!("Skill: {skill_title}"));
        if let Some(description) = skill.meta.base.description.as_deref() {
            lines.push(description.to_string());
        }
        lines.push(format!("Input schema: {}", skill.input_schema));
        lines.push(format!("Output schema: {}", skill.output_schema));
    }
    if !card.content_refs.is_empty() {
        lines.push("Grounding references:".to_string());
        for content_ref in &card.content_refs {
            lines.push(format!("- {}", content_ref.uri));
        }
    }
    lines.join("\n")
}

/// Build the [`ChatRequest`] for one case: an optional agent/skill system message (the
/// grounding context, including the agent's reference URIs) followed by the case prompt as
/// the user message. The expected reference URIs are *not* injected — the scorer checks
/// whether the model's answer actually surfaces them from the grounding context.
fn build_chat_request(case: &EvalCase, system_prompt: Option<&str>) -> ChatRequest {
    let mut messages = Vec::new();
    if let Some(system) = system_prompt {
        messages.push(ChatMessage {
            role: MessageRole::System,
            content: system.to_string(),
        });
    }
    messages.push(ChatMessage {
        role: MessageRole::User,
        content: case.prompt.clone(),
    });
    ChatRequest {
        messages,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    }
}

/// Score one case against the model's response text.
///
/// The rule is deliberately simple, deterministic, and honest — it is a containment check,
/// **not** an LLM judge:
///
/// 1. The response must be non-empty (after trimming).
/// 2. For every [`ContentRef`] in `expected_refs`, the ref's `uri` must appear verbatim in
///    the response. The runner supplies the agent's own reference URIs as grounding context
///    (never the expected refs themselves), so a model that faithfully grounds its answer
///    in that context surfaces the expected URIs and passes; a model that omits a required
///    reference (or whose grounding context lacks it) fails the case.
///
/// A case with no `expected_refs` passes on a non-empty response alone.
#[must_use]
pub fn score_case(case: &EvalCase, response: &str) -> bool {
    if response.trim().is_empty() {
        return false;
    }
    case.expected_refs
        .iter()
        .all(|content_ref: &ContentRef| response.contains(&content_ref.uri))
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_request(request: &EvalRunRequest) -> Result<(), EvalRunError> {
    if request.run_id.trim().is_empty()
        || request.agent_id.trim().is_empty()
        || request.dataset_id.trim().is_empty()
        || request.cases.is_empty()
        || request
            .cases
            .iter()
            .any(|case| case.case_id.trim().is_empty() || case.prompt.trim().is_empty())
    {
        return Err(EvalRunError::InvalidRequest);
    }
    Ok(())
}

/// The deterministic, offline default [`LanguageModel`] used by the daemon in CI.
///
/// Models a faithful, fully-grounded agent by echoing back every message's content (the
/// system grounding context plus the user prompt) as the assistant reply, with no network
/// access. Callers swap in a real client (Anthropic / OpenAI-compatible) when a live model
/// is configured.
///
/// Because the runner places the agent's grounding reference URIs in the system message, a
/// case whose `expected_refs` are present in that grounding context passes the
/// [`score_case`] containment check, while a case whose expected ref is absent fails —
/// making both outcomes deterministic. The request is still validated, so an empty
/// message surfaces an error (recorded as a failed case).
#[derive(Clone, Copy, Debug, Default)]
pub struct StubLanguageModel;

impl LanguageModel for StubLanguageModel {
    fn model_id(&self) -> &'static str {
        "stub-echo"
    }

    fn complete(&self, request: ChatRequest) -> tdw_llm::Result<tdw_llm::ChatResponse> {
        // Validate (non-empty messages/content, positive token budget) before echoing, so
        // the stub honors the same contract as the real clients.
        let _ = tdw_llm::last_user_message(&request)?;
        let content = request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(tdw_llm::ChatResponse {
            model_id: self.model_id().to_string(),
            message: ChatMessage {
                role: MessageRole::Assistant,
                content,
            },
            usage: tdw_llm::Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_agent::{ContentKind, ContentRef, EvalCase, EvalRunRequest, sample_agent_card};
    use tdw_agent_store::AgentStore;

    fn runner() -> EvalRunner {
        EvalRunner::new(Arc::new(StubLanguageModel))
    }

    fn expected_ref(uri: &str) -> ContentRef {
        ContentRef {
            uri: uri.to_string(),
            kind: ContentKind::Prompt,
            checksum: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn runs_eval_and_persists_per_case_metrics() {
        let mut store = AgentStore::new();
        store.upsert_agent(sample_agent_card());
        // Case 1 passes: the echo stub returns the prompt+ref, which contains the ref URI.
        // Case 2 fails: its expected ref is never present in the prompt, so the echoed
        // response cannot contain it.
        let outcome = runner().run(
            EvalRunRequest {
                run_id: "eval-1".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![
                    EvalCase {
                        case_id: "case-pass".to_string(),
                        prompt: "Summarize AAPL".to_string(),
                        expected_refs: vec![expected_ref("tdw://docs/research-template")],
                    },
                    EvalCase {
                        case_id: "case-fail".to_string(),
                        prompt: "Summarize MSFT".to_string(),
                        expected_refs: vec![expected_ref("tdw://docs/never-mentioned")],
                    },
                ],
            },
            &mut store,
        );

        assert_eq!(outcome.status, "success");
        let metric = |name: &str| {
            outcome
                .metrics
                .iter()
                .find(|metric| metric.metric_name == name)
                .map(|metric| metric.metric_value)
        };
        assert_eq!(metric("case_count"), Some(2.0));
        assert_eq!(metric("passed"), Some(1.0));
        assert_eq!(metric("pass_rate"), Some(0.5));
        assert_eq!(
            store.eval_run("eval-1").map(|run| run.metrics.len()),
            Some(3)
        );
    }

    #[test]
    fn all_cases_failing_yields_failed_status() {
        let mut store = AgentStore::new();
        store.upsert_agent(sample_agent_card());
        let outcome = runner().run(
            EvalRunRequest {
                run_id: "eval-fail".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: vec![expected_ref("tdw://docs/absent")],
                }],
            },
            &mut store,
        );

        assert_eq!(outcome.status, "failed");
        let pass_rate = outcome
            .metrics
            .iter()
            .find(|metric| metric.metric_name == "pass_rate")
            .map(|metric| metric.metric_value);
        assert_eq!(pass_rate, Some(0.0));
    }

    #[test]
    fn case_without_expected_refs_passes_on_non_empty_response() {
        let case = EvalCase {
            case_id: "case-1".to_string(),
            prompt: "Summarize AAPL".to_string(),
            expected_refs: Vec::new(),
        };
        assert!(score_case(&case, "a non-empty note"));
        assert!(!score_case(&case, "   "));
    }

    #[test]
    fn checked_run_rejects_empty_eval_cases() {
        let mut store = AgentStore::new();
        store.upsert_agent(sample_agent_card());

        assert_eq!(
            runner().try_run(
                EvalRunRequest {
                    run_id: "eval-1".to_string(),
                    agent_id: "market-researcher".to_string(),
                    dataset_id: "golden-market-notes".to_string(),
                    cases: Vec::new(),
                },
                &mut store,
            ),
            Err(EvalRunError::InvalidRequest)
        );
    }

    #[test]
    fn validate_request_rejects_each_malformed_field() {
        let base = || EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: Vec::new(),
            }],
        };
        let case = |case_id: &str, prompt: &str| EvalCase {
            case_id: case_id.to_string(),
            prompt: prompt.to_string(),
            expected_refs: Vec::new(),
        };

        assert_eq!(
            validate_request(&EvalRunRequest {
                run_id: " ".to_string(),
                ..base()
            }),
            Err(EvalRunError::InvalidRequest)
        );
        assert_eq!(
            validate_request(&EvalRunRequest {
                agent_id: String::new(),
                ..base()
            }),
            Err(EvalRunError::InvalidRequest)
        );
        assert_eq!(
            validate_request(&EvalRunRequest {
                dataset_id: " ".to_string(),
                ..base()
            }),
            Err(EvalRunError::InvalidRequest)
        );
        assert_eq!(
            validate_request(&EvalRunRequest {
                cases: vec![case(" ", "p")],
                ..base()
            }),
            Err(EvalRunError::InvalidRequest)
        );
        assert_eq!(
            validate_request(&EvalRunRequest {
                cases: vec![case("c", "  ")],
                ..base()
            }),
            Err(EvalRunError::InvalidRequest)
        );
        assert!(validate_request(&base()).is_ok());
    }
}
