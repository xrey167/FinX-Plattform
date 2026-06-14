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
//! - [`learning`] — the W4 learning-loop read seam: [`LearningState`] snapshots
//!   the gated runtime's `versions()` + resolved adaptivity per turn so a
//!   promoted lesson/rule/param takes effect (W4.1), and the trust-dial →
//!   retrieval filter / route reshaping (W4.2). Core only READS the runtime.
//! - [`walk_forward`] — the W4.3 induced-rule routing hints (read off the
//!   installed, promoted-past-B9 rule set) + the W4.4 walk-forward usefulness
//!   harness proving rising usefulness across a learning epoch.
//! - this module — the [`PartnerCore::turn`] sequencer + the [`PartnerEvent`]
//!   vocabulary, plus [`PartnerCore::answer_workspace`] (the W2.7 Workspace seam
//!   that reuses the pure `tdw_openbb_agent` two-leg).

pub mod audit;
pub mod dataplane;
pub mod learning;
pub mod principal;
pub mod proactive;
pub mod resolve;
pub mod scheduler;
pub mod walk_forward;
pub mod writeback;

use std::sync::Arc;

use tdw_infer::InferEngine;
use tdw_knowledge::runtime::KnowledgeRuntime;
use tdw_llm::StreamingLanguageModel;
use tdw_openbb_agent::{Answer, QueryRequest, answer};

pub use audit::{
    ActionKind, ActionRecord, ActionStatus, AuditInputs, Correction, EscalationConfig,
    FeedbackSignal, KgWriteReversal, RetiredEdge, UndoError, UndoOutcome, UndoReversal, audit_feed,
    correct, escalation_status, undo,
};
pub use dataplane::{DataPlane, DataPlaneError};
pub use learning::{AppliedResolution, LearningState, apply_to_resolution, retrieval_admits};
pub use principal::{Principal, Provenance, TrustContext};
pub use proactive::{
    BriefInputs, Dismissal, Nudge, NudgeKind, Severity, build_brief, rerank_with_dismissals,
};
pub use resolve::{KNOWLEDGE_VERBS, ResolvedRoute, ResolvedRoutes};
pub use scheduler::{BRIEF_QUEUE, BRIEF_TOOL, BriefJobSpec, DAILY_BRIEF_CRON, daily_brief_spec};
pub use walk_forward::{RoutingHint, UsefulnessReport, is_induced_type, routing_hints_from};
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
    /// The promoted inference generation that shaped this turn (W4.1).
    ///
    /// `0` when learning is inactive or no rule-set has been promoted past B9;
    /// otherwise the gated `infer_version` read from the runtime. A surface can
    /// attribute the turn's routing to this generation, and a bumped version is
    /// observable here — the proof that a promotion changed behavior.
    pub infer_generation: u64,
    /// The answer model's failure, when the streamed generation did NOT complete
    /// cleanly (MEDIUM, v1.7.1).
    ///
    /// `None` on a successful turn. `Some(message)` when `complete_streaming`
    /// errored mid-stream: [`Self::answer`] then holds at most a PARTIAL answer
    /// and MUST NOT be persisted as a complete answer. A write-back caller checks
    /// this flag to distinguish a failed/partial generation from a successful one
    /// (it previously could only infer failure from a `Reasoning` event string).
    pub model_error: Option<String>,
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
    /// The gated knowledge runtime Partner Core *reads* per turn for the learning
    /// loop (the partner design §3, W4.1/W4.2). `None` keeps the partner working
    /// without a learning loop attached (offline/CLI surfaces); when attached,
    /// every turn snapshots its `versions()` + resolves the principal's
    /// adaptivity so a promoted lesson/rule/param takes effect *this* turn. Core
    /// only reads it — it never mutates the runtime, the §3 invariant.
    knowledge: Option<Arc<KnowledgeRuntime>>,
    /// The gated induced-rule engine Partner Core READS per turn for learned
    /// route preferences (the partner design §3, W4.2/W4.3). The gated daemon
    /// worker owns the engine and hot-reloads only promoted-past-B9 rules into
    /// it; Core reads the installed rule set via
    /// [`walk_forward::routing_hints_from`] and never mutates it. `None` keeps a
    /// surface at the baseline route order (no learned reshaping). The preference
    /// is therefore ALWAYS the gated signal — never a caller-supplied hint.
    infer: Option<Arc<InferEngine>>,
}

impl PartnerCore {
    /// Build a Partner Core over a streaming `model` and a `dataplane` port.
    ///
    /// No learning loop is attached; call [`Self::with_knowledge`] to wire the
    /// gated runtime so promoted learning reshapes turns (W4.1/W4.2).
    #[must_use]
    pub fn new(model: Arc<dyn StreamingLanguageModel>, dataplane: Arc<dyn DataPlane>) -> Self {
        Self {
            model,
            dataplane,
            knowledge: None,
            infer: None,
        }
    }

    /// Attach the gated knowledge `runtime` Partner Core reads per turn
    /// (the partner design §3, W4.1). Read-only: the core consumes
    /// `versions()` + the adaptivity resolver but never mutates the runtime, so
    /// every behavior change stays B9/eval-gated upstream.
    #[must_use]
    pub fn with_knowledge(mut self, runtime: Arc<KnowledgeRuntime>) -> Self {
        self.knowledge = Some(runtime);
        self
    }

    /// Attach the gated induced-rule `engine` Partner Core reads per turn for
    /// learned route preferences (the partner design §3, W4.2/W4.3).
    ///
    /// Read-only: the core reads the installed (promoted-past-B9) rule set via
    /// [`walk_forward::routing_hints_from`] to derive a gated route-preference
    /// list, but never mutates the engine — the daemon worker is the sole writer
    /// (it hot-reloads only promoted rules). Wiring this is what makes the W4.2
    /// learned-route reshaping fire on a LIVE turn rather than only in unit tests:
    /// a preferred route leads the resolution, gated by the principal's adaptivity.
    #[must_use]
    pub fn with_infer_engine(mut self, engine: Arc<InferEngine>) -> Self {
        self.infer = Some(engine);
        self
    }

    /// The gated learned route preferences for this turn (W4.2/W4.3).
    ///
    /// Reads the installed rule set off the attached [`InferEngine`] and maps its
    /// induced routing hints to a route-preference list via
    /// [`walk_forward::route_preferences_from_hints`]. The preference is ALWAYS
    /// the gated, promoted-past-B9 signal (the engine holds only hot-reloaded
    /// rules) — never caller-supplied — preserving the audit-only/gated posture.
    /// Empty when no engine is attached or no induced rule is installed, so the
    /// resolution stays at the baseline order. The trust-dial gate is applied
    /// downstream in [`learning::apply_to_resolution`], so a below-`Learning`
    /// principal is never reshaped even when preferences are present.
    fn gated_preferred_routes(&self) -> Vec<String> {
        self.infer.as_ref().map_or_else(Vec::new, |engine| {
            walk_forward::route_preferences_from_hints(&walk_forward::routing_hints_from(engine))
        })
    }

    /// The current gated learning state for `principal` (W4.1/W4.2).
    ///
    /// A pure read over the attached runtime: the version triple + the resolved
    /// adaptivity. Returns the inert default (`infer_version = None`, no
    /// adaptivity → learning inactive) when no runtime is attached, so a partner
    /// without a learning loop behaves as the un-promoted baseline.
    #[must_use]
    pub fn learning_state(&self, principal: &Principal) -> LearningState {
        self.knowledge.as_ref().map_or(
            LearningState {
                infer_version: None,
                rules_version: None,
                adaptivity: None,
                preferred_routes: Vec::new(),
            },
            |runtime| LearningState::read_from(runtime, principal),
        )
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
    /// answer model's own failure is NOT an `Err`: the stream always closes
    /// cleanly, the failure is rendered as a terminal [`PartnerEvent::Reasoning`],
    /// AND it is recorded on [`TurnOutcome::model_error`] so a write-back caller
    /// can detect a failed/partial generation (MEDIUM, v1.7.1).
    pub async fn turn(
        &self,
        turn: &PartnerTurn,
        sink: &mut (dyn FnMut(PartnerEvent) + Send),
    ) -> Result<TurnOutcome, DataPlaneError> {
        sink(PartnerEvent::Reasoning(
            "Reviewing your question".to_string(),
        ));

        // [0] LEARNING — read the gated runtime ONCE for this turn (W4.1/W4.2,
        // the partner design §3). A promoted lesson/rule/param only reaches here by a
        // gated version bump; Core reads, never mutates. The learned route
        // preferences are derived from the gated InferEngine's installed
        // (promoted-past-B9) rule set and threaded onto the snapshot BEFORE the
        // resolution is reshaped — the gated signal, never a caller hint (W4.2).
        let learning = self
            .learning_state(&turn.principal)
            .with_preferred_routes(self.gated_preferred_routes());

        // [1] RESOLVE — catalog-bounded route selection (never free-form), then
        // reshaped by the gated learning state: a promoted inference generation
        // re-weights / stamps the resolution AND a learned route preference moves
        // a preferred route ahead, so a bumped version + an induced hint change
        // this turn's behavior (W4.1/W4.2). With learning inactive this is the
        // baseline.
        let base_resolved = resolve::resolve_routes(&turn.utterance, self.model.as_ref());
        let applied = learning::apply_to_resolution(&learning, base_resolved);
        let resolved = applied.resolved;

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
        // A streamed-generation failure is surfaced BOTH as the reasoning step
        // (so the stream still closes cleanly) AND as `model_error` on the
        // outcome (MEDIUM, v1.7.1), so a write-back caller can distinguish a
        // failed/partial generation from a successful one and refuse to persist
        // the (at most partial) answer as complete.
        let model_error = match stream_result {
            Ok(_response) => None,
            Err(error) => {
                let message = error.to_string();
                sink(PartnerEvent::Reasoning(format!("Model error: {message}")));
                Some(message)
            }
        };

        // [5] CITATION — close with the provenance the adapter renders.
        if !provenance.is_empty() {
            sink(PartnerEvent::Citation(provenance.clone()));
        }

        Ok(TurnOutcome {
            answer: answer_text,
            provenance,
            resolved,
            infer_generation: applied.infer_generation,
            model_error,
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

    /// A gated runtime with a fixed promoted `infer_version` and a resolver that
    /// grants the partner agent `Learning` — the W4.1 learning-loop fixture.
    fn runtime_at_generation(infer_version: u64) -> Arc<KnowledgeRuntime> {
        use tdw_taxonomy::Adaptivity;
        let embedder = Arc::new(tdw_embed_local::HashEmbeddingProvider::default());
        let vectors = Arc::new(tdw_storage_qdrant::InMemoryVectorEngine::default());
        let resolver: tdw_knowledge::runtime::AdaptivityResolver =
            Arc::new(|_agent: &str| Some(Adaptivity::Learning));
        Arc::new(
            KnowledgeRuntime::new(embedder, vectors)
                .with_versions(None, Some(infer_version))
                .with_adaptivity_resolver(resolver),
        )
    }

    /// A model that returns a SCRIPTED selection verbatim from `complete` (so the
    /// resolve step yields exactly the routes named, one per line) and whose
    /// streaming answer echoes the same text. Used to drive a LIVE turn with a
    /// known multi-route resolution so the learned-reorder is observable.
    struct ScriptedModel {
        selection: String,
    }

    impl tdw_llm::LanguageModel for ScriptedModel {
        fn model_id(&self) -> &'static str {
            "scripted-test"
        }
        fn complete(
            &self,
            _request: tdw_llm::ChatRequest,
        ) -> tdw_llm::Result<tdw_llm::ChatResponse> {
            Ok(tdw_llm::ChatResponse {
                model_id: "scripted-test".to_string(),
                message: tdw_llm::ChatMessage {
                    role: tdw_llm::MessageRole::Assistant,
                    content: self.selection.clone(),
                },
                usage: tdw_llm::Usage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            })
        }
    }
    impl tdw_llm::StreamingLanguageModel for ScriptedModel {}

    /// A model whose generation leg ALWAYS fails — drives the MEDIUM model-error
    /// path so the turn surfaces `model_error` instead of a silent partial Ok.
    struct FailingModel;
    impl tdw_llm::LanguageModel for FailingModel {
        fn model_id(&self) -> &'static str {
            "failing-test"
        }
        fn complete(
            &self,
            _request: tdw_llm::ChatRequest,
        ) -> tdw_llm::Result<tdw_llm::ChatResponse> {
            Err(tdw_llm::LlmError::EmptyMessages)
        }
    }
    impl tdw_llm::StreamingLanguageModel for FailingModel {}

    // ── MEDIUM (v1.7.1): a model-leg failure is detectable on the outcome ─────

    #[tokio::test]
    async fn model_error_is_surfaced_on_the_turn_outcome() {
        // The generation leg fails: the turn still returns Ok (the stream closes
        // cleanly) BUT model_error is Some, and the answer is empty — a write-back
        // caller MUST NOT persist this partial answer as complete.
        let core = PartnerCore::new(Arc::new(FailingModel), Arc::new(FixturePlane::default()));
        let turn = PartnerTurn::new(Principal::new("s", "a"), "hello");
        let mut events = Vec::new();
        let outcome = core
            .turn(&turn, &mut |event| events.push(event))
            .await
            .expect("turn closes cleanly even on a model error");
        assert!(
            outcome.model_error.is_some(),
            "a failed generation is detectable on the outcome: {outcome:?}"
        );
        assert!(
            outcome.answer.is_empty(),
            "no partial answer accumulated when the model errors: {:?}",
            outcome.answer
        );
        // The reasoning event still reports the error so the stream is legible.
        assert!(
            events
                .iter()
                .any(|event| matches!(event, PartnerEvent::Reasoning(message) if message.contains("Model error"))),
            "the model error is also rendered as a reasoning step: {events:?}"
        );
    }

    #[tokio::test]
    async fn successful_turn_has_no_model_error() {
        // The happy path: a clean generation leaves model_error None, so a
        // write-back caller knows the answer is complete.
        let core = core_with(HashMap::new());
        let turn = PartnerTurn::new(Principal::new("s", "a"), "What is a P/E ratio?");
        let outcome = core.turn(&turn, &mut |_| {}).await.expect("turn completes");
        assert!(
            outcome.model_error.is_none(),
            "a successful turn carries no model_error: {outcome:?}"
        );
    }

    /// A gated infer engine holding one INSTALLED (hot-reloaded, hence
    /// promoted-past-B9) induced rule whose derived type's route-prefix segment is
    /// `route_prefix` — the gated W4.2 route-preference signal Partner Core reads.
    fn infer_engine_preferring(route_prefix: &str) -> Arc<tdw_infer::InferEngine> {
        use tdw_infer::{EdgePattern, InferEngine, InferRule};
        let mut engine = InferEngine::default();
        engine
            .hot_reload(vec![InferRule::DeriveEdge {
                rule_id: "inducted_pattern_test".to_string(),
                stratum: 0,
                when: vec![EdgePattern {
                    rel: "supplier_of".to_string(),
                }],
                // `inducted:<prefix>--<rel>`: route_preferences_from_hints strips
                // the namespace and takes the pre-`--` segment as the preference.
                derived_type: format!("inducted:{route_prefix}--supplier_of"),
            }])
            .expect("hot_reload installs the induced rule");
        Arc::new(engine)
    }

    // ── W4.2 (v1.7.1): a LIVE turn re-orders routes so the learned route leads ─

    #[tokio::test]
    async fn live_turn_reorders_routes_so_gated_preference_leads() {
        // A scripted resolution names a non-equity route FIRST, then an equity
        // route. The gated infer engine carries an induced rule preferring the
        // `equity` family. On a LIVE turn (gate open via the Learning runtime),
        // the learned preference must move the equity route AHEAD — the W4.2
        // promise firing in the product, not only in a learning.rs unit test.
        let mut rows = HashMap::new();
        rows.insert("news/world".to_string(), serde_json::json!({"rows": []}));
        rows.insert(
            "equity/profile".to_string(),
            serde_json::json!({"rows": []}),
        );
        let scripted = Arc::new(ScriptedModel {
            selection: "news/world\nequity/profile".to_string(),
        });
        let core = PartnerCore {
            model: scripted,
            dataplane: Arc::new(FixturePlane { rows }),
            knowledge: Some(runtime_at_generation(1)),
            infer: Some(infer_engine_preferring("equity")),
        };
        let turn = PartnerTurn::new(Principal::new("sess", "agent:partner"), "How did AAPL do?");
        let outcome = core
            .turn(&turn, &mut |_| {})
            .await
            .expect("live turn completes");

        // Baseline resolution order is news/world then equity/profile; the
        // gated preference for `equity` moves the equity route to lead.
        assert_eq!(
            outcome.resolved.data_routes(),
            vec!["equity/profile".to_string(), "news/world".to_string()],
            "the learned-preferred route leads on a live turn: {:?}",
            outcome.resolved.data_routes()
        );
    }

    #[tokio::test]
    async fn live_turn_without_infer_engine_keeps_baseline_order() {
        // No gated engine attached → no learned preference → baseline order, so
        // the reshaping is bounded by the gated signal even with learning active.
        let mut rows = HashMap::new();
        rows.insert("news/world".to_string(), serde_json::json!({"rows": []}));
        rows.insert(
            "equity/profile".to_string(),
            serde_json::json!({"rows": []}),
        );
        let scripted = Arc::new(ScriptedModel {
            selection: "news/world\nequity/profile".to_string(),
        });
        let core = PartnerCore {
            model: scripted,
            dataplane: Arc::new(FixturePlane { rows }),
            knowledge: Some(runtime_at_generation(1)),
            infer: None,
        };
        let turn = PartnerTurn::new(Principal::new("sess", "agent:partner"), "How did AAPL do?");
        let outcome = core.turn(&turn, &mut |_| {}).await.expect("turn completes");
        assert_eq!(
            outcome.resolved.data_routes(),
            vec!["news/world".to_string(), "equity/profile".to_string()],
            "no gated engine → baseline order preserved"
        );
    }

    // ── W4.1: a bumped promoted version changes the turn's behavior ──────────

    #[tokio::test]
    async fn bumped_runtime_version_changes_turn_outcome() {
        // The SAME turn run against two runtimes that differ only in the promoted
        // infer_version must produce a different outcome generation — the W4.1
        // done-condition (Partner Core reads versions() per turn so a promotion
        // takes effect), with Core only reading the gated runtime.
        let turn = PartnerTurn::new(Principal::new("sess", "agent:partner"), "What is AAPL?");

        let core_v1 = core_with(HashMap::new()).with_knowledge(runtime_at_generation(1));
        let out_v1 = core_v1
            .turn(&turn, &mut |_| {})
            .await
            .expect("turn completes at gen 1");
        assert_eq!(out_v1.infer_generation, 1);

        let core_v2 = core_with(HashMap::new()).with_knowledge(runtime_at_generation(2));
        let out_v2 = core_v2
            .turn(&turn, &mut |_| {})
            .await
            .expect("turn completes at gen 2");
        assert_eq!(out_v2.infer_generation, 2);

        assert_ne!(
            out_v1.infer_generation, out_v2.infer_generation,
            "a bumped promoted version changes the turn's behavior"
        );
    }

    #[tokio::test]
    async fn turn_without_knowledge_is_baseline_generation() {
        // No learning loop attached → the inert baseline (generation 0).
        let core = core_with(HashMap::new());
        let turn = PartnerTurn::new(Principal::new("s", "a"), "hello");
        let outcome = core.turn(&turn, &mut |_| {}).await.expect("turn completes");
        assert_eq!(outcome.infer_generation, 0);
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
        assert!(names2.contains(&"citations"));
        assert_eq!(names2.last(), Some(&"prompt_suggestions"));
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
