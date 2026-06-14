//! Route resolution bounded by the catalog (the FinX Partner design §1.3 step 1, §1.4).
//!
//! This is the **anti-over-engineering line**: route resolution is LLM
//! tool-selection *bounded by [`tdw_endpoint_catalog::catalog`]* plus the
//! `tdw.kg.*` knowledge verb set — NOT a free-form planner. The model picks from
//! the 268 catalog routes + the knowledge verbs, and an invalid pick is rejected
//! by [`tdw_endpoint_catalog::is_valid_route`] (data routes) or the known-verb
//! set (knowledge routes) **before any I/O**. There is no graph executor and no
//! agent loop here — just "classify intent, choose route(s), guard them".

use serde::{Deserialize, Serialize};
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};

/// The knowledge verbs the resolver may select alongside catalog data routes.
///
/// These are the read-side `tdw.kg.*` tools Partner Core composes for the
/// memory-context step (the FinX Partner design §1.3 step 2). They are not catalog routes
/// (so `is_valid_route` does not apply); the resolver guards them against this
/// fixed set instead.
pub const KNOWLEDGE_VERBS: &[&str] = &["tdw.kg.search", "tdw.kg.answer", "tdw.kg.why"];

/// Whether `route` is a knowledge verb the resolver may select.
#[must_use]
pub fn is_knowledge_verb(route: &str) -> bool {
    KNOWLEDGE_VERBS.contains(&route)
}

/// Whether `route` is a selectable target: a valid catalog data route OR a
/// known knowledge verb. Anything else is rejected before dispatch.
#[must_use]
pub fn is_selectable(route: &str) -> bool {
    is_knowledge_verb(route) || tdw_endpoint_catalog::is_valid_route(route)
}

/// The resolved targets for a turn: the guarded data routes and knowledge verbs
/// the turn will execute, in selection order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRoutes {
    /// Catalog data routes (each one passed [`tdw_endpoint_catalog::is_valid_route`]
    /// AND [`tdw_endpoint_catalog::lookup`]).
    pub data: Vec<String>,
    /// Knowledge verbs (`tdw.kg.*`) selected for the memory-context step.
    pub knowledge: Vec<String>,
}

impl ResolvedRoutes {
    /// Whether nothing resolved (a pure knowledge-free, data-free chat turn).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.data.is_empty() && self.knowledge.is_empty()
    }
}

/// Resolve `utterance` into a guarded set of routes using `model` for selection.
///
/// The model is shown the candidate vocabulary and asked to echo the routes it
/// would use, one per line. Every line is then guarded:
/// - a data route is kept only if [`tdw_endpoint_catalog::lookup`] finds it
///   (which also implies `is_valid_route`);
/// - a knowledge verb is kept only if it is in [`KNOWLEDGE_VERBS`];
/// - anything else (a hallucinated or malformed route) is dropped.
///
/// So an invalid pick can never reach the dispatcher. On a model error the
/// resolver returns an empty set (the turn answers directly without data),
/// never a panic.
#[must_use]
pub fn resolve_routes(utterance: &str, model: &dyn LanguageModel) -> ResolvedRoutes {
    let request = build_select_request(utterance);
    let Ok(response) = model.complete(request) else {
        return ResolvedRoutes::default();
    };
    guard_selection(&response.message.content)
}

/// Build the bounded tool-selection prompt. The system message lists the
/// knowledge verbs explicitly and instructs the model to pick only from the
/// catalog grammar; the guard enforces it regardless of what the model returns.
fn build_select_request(utterance: &str) -> ChatRequest {
    let system = format!(
        "You are a route selector for a financial data warehouse. Given the user's question, \
         list the data routes and knowledge verbs needed to answer it, ONE PER LINE, and nothing \
         else. Data routes are slash-namespaced catalog routes (e.g. equity/price/historical). \
         Knowledge verbs are exactly: {verbs}. If no data or knowledge lookup is needed, output \
         nothing.",
        verbs = KNOWLEDGE_VERBS.join(", "),
    );
    ChatRequest {
        messages: vec![
            ChatMessage {
                role: MessageRole::System,
                content: system,
            },
            ChatMessage {
                role: MessageRole::User,
                content: utterance.to_string(),
            },
        ],
        max_output_tokens: 256,
    }
}

/// Guard a newline-delimited model selection against the catalog + verb set.
fn guard_selection(selection: &str) -> ResolvedRoutes {
    let mut resolved = ResolvedRoutes::default();
    for raw in selection.lines() {
        let candidate = raw.trim().trim_start_matches(['-', '*', ' ']).trim();
        if candidate.is_empty() {
            continue;
        }
        if is_knowledge_verb(candidate) {
            if !resolved.knowledge.iter().any(|v| v == candidate) {
                resolved.knowledge.push(candidate.to_string());
            }
        } else if tdw_endpoint_catalog::lookup(candidate).is_some() {
            // lookup() implies is_valid_route(): an invalid route never reaches
            // the dispatcher.
            if !resolved.data.iter().any(|r| r == candidate) {
                resolved.data.push(candidate.to_string());
            }
        }
        // Anything else (hallucinated / malformed) is silently dropped — the
        // guard is the safety net, not the model.
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_llm::{ChatResponse, Result as LlmResult, StreamingLanguageModel, Usage};

    /// A scripted model that returns a fixed selection string verbatim — the
    /// offline golden-query stub for the routing eval.
    struct ScriptedModel(&'static str);

    impl LanguageModel for ScriptedModel {
        fn model_id(&self) -> &'static str {
            "scripted-resolver"
        }

        fn complete(&self, request: ChatRequest) -> LlmResult<ChatResponse> {
            let _ = tdw_llm::last_user_message(&request)?;
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content: self.0.to_string(),
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    impl StreamingLanguageModel for ScriptedModel {}

    #[test]
    fn keeps_only_valid_catalog_routes_and_known_verbs() {
        // The model proposes a real route, a knowledge verb, and a hallucinated
        // route; the guard keeps the first two and drops the third.
        let model = ScriptedModel(
            "equity/price/historical\ntdw.kg.answer\nnot/a/real/route/that/exists/anywhere",
        );
        let resolved = resolve_routes("How did AAPL do and what do we know?", &model);
        assert!(
            resolved
                .data
                .contains(&"equity/price/historical".to_string())
        );
        assert!(resolved.knowledge.contains(&"tdw.kg.answer".to_string()));
        assert!(
            !resolved.data.iter().any(|r| r.contains("not/a/real")),
            "hallucinated route is guarded out: {resolved:?}"
        );
    }

    #[test]
    fn invalid_route_is_never_selectable() {
        // The grammar guard: an uppercase / malformed route is rejected even if
        // the model emits it.
        assert!(!is_selectable("Equity/Price"));
        assert!(!is_selectable("equity//price"));
        assert!(!is_selectable("tdw.kg.unknownverb"));
        // A real catalog route and a real verb are selectable.
        assert!(is_selectable("equity/price/historical"));
        assert!(is_selectable("tdw.kg.search"));
    }

    #[test]
    fn empty_selection_resolves_to_nothing() {
        let model = ScriptedModel("");
        let resolved = resolve_routes("What is a P/E ratio?", &model);
        assert!(resolved.is_empty());
    }

    #[test]
    fn dedupes_repeated_picks() {
        let model = ScriptedModel("equity/price/historical\nequity/price/historical");
        let resolved = resolve_routes("AAPL?", &model);
        assert_eq!(resolved.data.len(), 1);
    }
}
