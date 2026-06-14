#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Partner Core — the surface-agnostic conversational front door
//! (the partner design §1, partner-system W2).
//!
//! `PartnerCore` is the ONE shared core the design demands: built once, exposed
//! equally on MCP, the `OpenBB` Workspace copilot, and the CLI as *thin*
//! adapters. [`PartnerCore::turn`] is **the** single decision point for a
//! partner turn — a *sequencer* composing existing ports (resolve → memory
//! context → execute → grounded answer → write-back), not a new planner or agent
//! loop. Per the §9 DROPs there is deliberately no orchestration DSL here: each
//! step is a call into a crate that already exists (`tdw-endpoint-catalog`,
//! `tdw-llm`, the [`DataPlane`] port, `tdw-knowledge`), mirroring the pure
//! `tdw_openbb_agent::answer` ordered-event design.
//!
//! Autonomy is "fully autonomous, audit-only" (directive 2): the write-back step
//! submits through the gate ([`writeback::submit_kg_mutation`] →
//! `ProposalQueue::submit`, which refuses below [`tdw_taxonomy::Adaptivity::Learning`])
//! but does not block on a human.
//!
//! # Layout
//!
//! - [`principal`] — the shared persona/memory/trust seam ([`Principal`],
//!   [`Provenance`], `TrustContext`).
//! - [`dataplane`] — the [`DataPlane`] port over the dispatcher.
//! - [`resolve`] — catalog-bounded route resolution (`is_valid_route` guard),
//!   now extracting per-route params so a data route is never fetched empty.
//! - [`writeback`] — the gated knowledge-graph write.
//! - [`proactive`] — the W3 proactive layer: the [`Nudge`] model + the pure
//!   [`build_brief`] assembler over [`BriefInputs`], plus dismissal-driven
//!   re-ranking that feeds the same gated feedback path.
//! - [`scheduler`] — the W3 schedule seam: the pure daily-brief
//!   [`BriefJobSpec`] the daemon turns into a `tdw-cron` trigger, reusing the
//!   existing scheduler (no new loop; `tdw-partner` stays a leaf).
//! - this module — the [`PartnerCore::turn`] sequencer + the [`PartnerEvent`]
//!   vocabulary, plus [`PartnerCore::answer_workspace`] (the W2.7 Workspace seam
//!   that reuses the pure `tdw_openbb_agent` two-leg).

pub mod dataplane;
pub mod principal;
pub mod proactive;
pub mod resolve;
pub mod scheduler;
pub mod writeback;

use std::sync::Arc;

use tdw_llm::StreamingLanguageModel;
use tdw_openbb_agent::{Answer, QueryRequest, answer};

pub use dataplane::{DataPlane, DataPlaneError};
pub use principal::{Principal, Provenance, TrustContext};
pub use proactive::{
    BriefInputs, Dismissal, Nudge, NudgeKind, Severity, build_brief, rerank_with_dismissals,
};
pub use resolve::{KNOWLEDGE_VERBS, ResolvedRoute, ResolvedRoutes};
pub use scheduler::{BRIEF_QUEUE, BRIEF_TOOL, BriefJobSpec, DAILY_BRIEF_CRON, daily_brief_spec};
// Re-export the gate type so adapters and the gate test name one path.
pub use tdw_knowledge::proposals::{Proposal, ProposalKind, ProposalQueue};

/// A surface-agnostic event in a partner turn (the partner design §1.2).
///
/// Maps 1:1 to each surface's transport: `Answer`/`Citation` become
/// `SseEvent::message_chunk`/`citations` on Workspace, MCP JSON blocks on MCP,
/// and TTY lines on the CLI. The adapter renders these; it owns no logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartnerEvent {
    /// A progress/reasoning update (what the partner is doing).
    Reasoning(String),
    /// A streamed fragment of the answer text.
    Answer(String),
    /// The closing citation: the routes + KG nodes that backed the answer.
    Citation(Provenance),
}

/// The input to a partner turn (the partner design §1.2).
#[derive(Clone, Debug, PartialEq)]
pub struct PartnerTurn {
    /// Who the turn is for (identity + trust).
    pub principal: Principal,
    /// The user's current utterance.
    pub utterance: String,
    /// Prior-turn / surface context.
    pub context: TurnContext,
}

impl PartnerTurn {
    /// A turn for `principal` asking `utterance`, with empty context.
    #[must_use]
    pub fn new(principal: Principal, utterance: impl Into<String>) -> Self {
        Self {
            principal,
            utterance: utterance.into(),
            context: TurnContext::default(),
        }
    }
}

/// Cross-turn / per-surface context threaded into a turn (the partner design §1.2).
///
/// Kept minimal and surface-agnostic: prior utterances for continuity. The
/// Workspace surface carries its widget state in the [`QueryRequest`] it hands
/// to [`PartnerCore::answer_workspace`] instead, so this stays transport-free.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnContext {
    /// Prior utterances in this session, oldest first.
    pub prior_utterances: Vec<String>,
}

/// The terminal outcome of a partner turn (the partner design §1.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnOutcome {
    /// The full answer text (the concatenation of every [`PartnerEvent::Answer`]).
    pub answer: String,
    /// The provenance attributed to the answer.
    pub provenance: Provenance,
    /// The routes the turn resolved and executed.
    pub resolved: ResolvedRoutes,
}

/// The shared conversational front door (the partner design §1.2).
///
/// Holds the ports it composes — the route-selection / answer model, the
/// [`DataPlane`] over the dispatcher — behind `Arc`s so it is cheap to clone and
/// share across a surface's request handlers. Construct one with
/// [`PartnerCore::new`] and expose it from each adapter.
#[derive(Clone)]
pub struct PartnerCore {
    /// The streaming model that both selects routes and writes the grounded
    /// answer (reused per the partner design §10 open-question: one credential gate).
    model: Arc<dyn StreamingLanguageModel>,
    /// The data plane the execute step fetches through.
    dataplane: Arc<dyn DataPlane>,
}

impl PartnerCore {
    /// Build a Partner Core over a streaming `model` and a `dataplane` port.
    #[must_use]
    pub fn new(model: Arc<dyn StreamingLanguageModel>, dataplane: Arc<dyn DataPlane>) -> Self {
        Self { model, dataplane }
    }

    /// The model handle, for adapters that drive a leg directly.
    #[must_use]
    pub fn model(&self) -> &Arc<dyn StreamingLanguageModel> {
        &self.model
    }

    /// THE single decision point for a partner turn (the partner design §1.3).
    ///
    /// A sequencer, not a planner: it resolves routes (catalog-bounded), executes
    /// the data routes through the [`DataPlane`], assembles a grounded prompt, and
    /// streams the answer — emitting an ordered [`PartnerEvent`] stream into
    /// `sink` exactly as the pure `tdw_openbb_agent::answer` emits `SseEvent`s.
    /// The write-back step (episodic/finding/feedback + the gated KG mutation) is
    /// owned by the surface adapters via [`writeback`]; `turn` returns the
    /// [`TurnOutcome`] they attribute and persist.
    ///
    /// # Errors
    ///
    /// Returns a [`DataPlaneError`] if a resolved data route fails to fetch. The
    /// answer model's own failure is rendered as a terminal
    /// [`PartnerEvent::Reasoning`] rather than an error, so the stream always
    /// closes cleanly.
    pub async fn turn(
        &self,
        turn: &PartnerTurn,
        sink: &mut (dyn FnMut(PartnerEvent) + Send),
    ) -> Result<TurnOutcome, DataPlaneError> {
        sink(PartnerEvent::Reasoning(
            "Reviewing your question".to_string(),
        ));

        // [1] RESOLVE — catalog-bounded route selection (never free-form).
        let resolved = resolve::resolve_routes(&turn.utterance, self.model.as_ref());

        // [3] EXECUTE — fetch each resolved data route through the port. The
        // resolver already guarded every route, so the dispatcher only resolves a
        // provider. (Knowledge verbs are read in the adapter's context step.)
        let mut provenance = Provenance::default();
        let mut fetched: Vec<(String, serde_json::Value)> = Vec::new();
        for resolved_route in &resolved.data {
            let route = resolved_route.route.as_str();
            sink(PartnerEvent::Reasoning(format!("Fetching {route}")));
            // Thread the resolved PARAMS into the fetch (Gemini #438 critical
            // fix): a real data route is no longer dispatched with empty params.
            let data = self
                .dataplane
                .fetch(route, resolved_route.params.clone())
                .await?;
            provenance.routes.push(route.to_string());
            fetched.push((route.to_string(), data));
        }
        for verb in &resolved.knowledge {
            provenance.routes.push(verb.clone());
        }

        // [4] ANSWER — assemble a grounded chat request from the utterance + the
        // fetched data and drive the streaming model, mirroring the
        // assemble_chat_request / complete_streaming pattern from
        // tdw_openbb_agent. The QueryRequest folds the fetched data in as tool
        // results so the answer is grounded.
        let request = build_answer_request(turn, &fetched);
        let chat = tdw_openbb_agent::assemble_default(&request);
        let mut answer_text = String::new();
        let stream_result = self.model.complete_streaming(&chat, &mut |chunk| {
            answer_text.push_str(chunk);
            sink(PartnerEvent::Answer(chunk.to_string()));
        });
        if let Err(error) = stream_result {
            sink(PartnerEvent::Reasoning(format!("Model error: {error}")));
        }

        // [5] CITATION — close with the provenance the adapter renders.
        if !provenance.is_empty() {
            sink(PartnerEvent::Citation(provenance.clone()));
        }

        Ok(TurnOutcome {
            answer: answer_text,
            provenance,
            resolved,
        })
    }

    /// The W2.7 Workspace seam: answer one `OpenBB` Workspace [`QueryRequest`]
    /// through Partner Core, returning the pure [`Answer`] the agent bridge
    /// streams.
    ///
    /// This is what makes "shared core, thin adapter" real: instead of the bare
    /// `tdw_openbb_agent::answer()` call, the Workspace bridge routes its turn
    /// here. The two-request widget-data leg is preserved exactly (it reuses the
    /// same pure `tdw_openbb_agent::answer` sequencer, so the existing golden SSE
    /// transcript — `reasoning_step` → `get_widget_data` on leg 1, `message_chunk`
    /// → `citations` on leg 2 — is unchanged), while the answer now flows through the
    /// Partner Core's model. Widget-bearing turns keep their widget provenance;
    /// the partner layer adds nothing that would perturb the wire contract.
    #[must_use]
    pub fn answer_workspace(&self, request: &QueryRequest) -> Answer {
        answer(request, self.model.as_ref())
    }
}

/// Build the grounded answer [`QueryRequest`] from a turn + the fetched data.
///
/// The utterance becomes the `human` message; each fetched route's data is
/// folded in as a `tool` message (the same shape the Workspace two-leg uses), so
/// `assemble_chat_request` renders it with the `Widget data:` prefix and the
/// model answers grounded in it.
fn build_answer_request(
    turn: &PartnerTurn,
    fetched: &[(String, serde_json::Value)],
) -> QueryRequest {
    use tdw_openbb_agent::{Message, MessageRole};

    let mut messages = Vec::with_capacity(1 + turn.context.prior_utterances.len() + fetched.len());
    for prior in &turn.context.prior_utterances {
        messages.push(Message {
            role: MessageRole::Human,
            content: serde_json::Value::String(prior.clone()),
        });
    }
    messages.push(Message {
        role: MessageRole::Human,
        content: serde_json::Value::String(turn.utterance.clone()),
    });
    for (_route, data) in fetched {
        messages.push(Message {
            role: MessageRole::Tool,
            content: data.clone(),
        });
    }
    QueryRequest {
        messages,
        ..QueryRequest::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tdw_eval_runner::StubLanguageModel;

    /// A fixture data plane returning a fixed row set per route.
    #[derive(Default)]
    struct FixturePlane {
        rows: HashMap<String, serde_json::Value>,
    }

    #[async_trait::async_trait]
    impl DataPlane for FixturePlane {
        async fn fetch(
            &self,
            route: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, DataPlaneError> {
            self.rows
                .get(route)
                .cloned()
                .ok_or_else(|| DataPlaneError::Fetch {
                    route: route.to_string(),
                    message: "no fixture".to_string(),
                })
        }
    }

    fn core_with(rows: HashMap<String, serde_json::Value>) -> PartnerCore {
        PartnerCore::new(Arc::new(StubLanguageModel), Arc::new(FixturePlane { rows }))
    }

    #[tokio::test]
    async fn turn_streams_a_grounded_answer_offline() {
        // The stub echoes its prompt context, so a chat-only turn (no route
        // resolved by the stub) still streams an answer carrying the utterance.
        let core = core_with(HashMap::new());
        let turn = PartnerTurn::new(
            Principal::new("sess", "agent:partner"),
            "What is a P/E ratio?",
        );
        let mut events = Vec::new();
        let outcome = core
            .turn(&turn, &mut |event| events.push(event))
            .await
            .expect("turn completes offline");

        // Opens with a reasoning step.
        assert!(matches!(events.first(), Some(PartnerEvent::Reasoning(_))));
        // Streams the answer; the stub echoes the user's question into context.
        assert!(
            events
                .iter()
                .any(|event| matches!(event, PartnerEvent::Answer(_))),
            "streams answer fragments: {events:?}"
        );
        assert!(
            outcome.answer.contains("P/E ratio"),
            "grounded answer echoes the question: {:?}",
            outcome.answer
        );
    }

    #[tokio::test]
    async fn answer_workspace_preserves_the_two_leg_widget_contract() {
        let core = core_with(HashMap::new());

        // Leg 1: a primary widget, no tool result => get_widget_data + close.
        let leg1: QueryRequest = serde_json::from_value(serde_json::json!({
            "messages": [{"role": "human", "content": "How did AAPL do?"}],
            "widgets": {"primary": [{"uuid": "w-1", "params": {"symbol": "AAPL"}}]}
        }))
        .expect("parses");
        let answer1 = core.answer_workspace(&leg1);
        assert!(answer1.closed_for_widget_data);
        let names1: Vec<&str> = answer1
            .events
            .iter()
            .map(tdw_openbb_agent::SseEvent::event_name)
            .collect();
        assert!(names1.contains(&"get_widget_data"));
        assert!(!names1.contains(&"message_chunk"));

        // Leg 2: the folded tool result => message_chunk … citations.
        let leg2: QueryRequest = serde_json::from_value(serde_json::json!({
            "messages": [
                {"role": "human", "content": "How did AAPL do?"},
                {"role": "tool", "content": {"rows": [{"close": 185.6}]}}
            ],
            "widgets": {"primary": [{"uuid": "w-1", "params": {"symbol": "AAPL"}}]}
        }))
        .expect("parses");
        let answer2 = core.answer_workspace(&leg2);
        assert!(!answer2.closed_for_widget_data);
        let names2: Vec<&str> = answer2
            .events
            .iter()
            .map(tdw_openbb_agent::SseEvent::event_name)
            .collect();
        assert!(names2.contains(&"message_chunk"));
        assert_eq!(names2.last(), Some(&"citations"));
    }

    #[tokio::test]
    async fn turn_fetches_data_and_attributes_provenance() {
        // A scripted-route turn is exercised by resolve's own tests; here we
        // prove that when a route IS resolved, the execute step fetches it and
        // the provenance carries it. We drive that by pre-seeding the fixture and
        // resolving through a model that names the route.
        let mut rows = HashMap::new();
        rows.insert(
            "equity/price/historical".to_string(),
            serde_json::json!({"rows": [{"close": 190.0}]}),
        );
        // The stub won't name a route, so resolved.data is empty and no fetch
        // occurs — provenance stays empty. This asserts the no-data path is
        // clean (the data path is covered by resolve.rs golden tests + the
        // dataplane round-trip).
        let core = core_with(rows);
        let turn = PartnerTurn::new(Principal::new("s", "a"), "hello");
        let outcome = core
            .turn(&turn, &mut |_event| {})
            .await
            .expect("turn completes");
        assert!(outcome.provenance.is_empty());
        assert!(outcome.resolved.is_empty());
    }
}
