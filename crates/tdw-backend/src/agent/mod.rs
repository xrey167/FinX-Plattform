//! Synchronous agent/MCP facade.
//!
//! Composes the `tdw-agent` [`Registry`], the [`ToolExecutor`], an [`McpServer`] (with the
//! registry + executor attached), and an [`AgentStore`]. Phase 2 adds the sync execution
//! surface on top: registry hot-reload + plugin loading (re-wiring the MCP cache), tool
//! listing/dispatch, single-line MCP JSON-RPC handling, workflow compilation, eval runs, and
//! agent/workflow CRUD passthrough. Pure composition / delegation — no business logic.

use std::path::Path;

use std::collections::BTreeMap;
use std::sync::Arc;

use tdw_agent::{AgentCard, EntityKind, EvalRunRequest, Registry, WorkflowDefinition};
use tdw_agent_store::AgentStore;
use tdw_app_client::DaemonClientConfig;
use tdw_eval_runner::{EvalRunOutcome, EvalRunner, StubLanguageModel};
use tdw_event::EventEnvelope;
use tdw_feature_store::{FeatureSnapshot, FeatureStore};
use tdw_hooks::{
    HookExecutionOutcome, HookExecutionPolicy, HookRegistry, HookSpec, SystemHookHandlerBackend,
};
use tdw_kg::{Entity, KnowledgeGraph, Relationship};
use tdw_llm::LanguageModel;
use tdw_mcp::McpServer;
use tdw_tags::{TagAssignment, TagDefinition, TagStore};
use tdw_tool_exec::{CommandPolicy, ToolExecutor, ToolOutcome};
use tdw_workflow_engine::{ExecutionPlan, WorkflowEngine};

use crate::config::BackendConfig;
use crate::error::BackendResult;

/// Environment variable naming the directory of `tdw-agent` registry definitions to load.
///
/// Shared with `tdw-mcp`'s server entrypoints so the embedded and stand-alone MCP surfaces
/// resolve the same registry.
pub const REGISTRY_DIR_ENV: &str = tdw_mcp::REGISTRY_DIR_ENV;

/// The `pass_rate` threshold below which eval feedback disables a skill. A run whose
/// aggregate pass rate falls under this bound flips the skill's `quality.disabled` flag;
/// at or above it the skill stays enabled. Half is a neutral, deterministic default —
/// majority-failing means disable.
const FEEDBACK_DISABLE_BELOW: f64 = 0.5;

/// The synchronous agent/MCP facade.
pub struct AgentBackend {
    registry: Registry,
    executor: ToolExecutor,
    mcp: McpServer,
    store: AgentStore,
    /// The synchronous knowledge graph (entities + relationships).
    kg: KnowledgeGraph,
    /// The synchronous tag taxonomy store.
    tags: TagStore,
    /// The synchronous feature store, materializing snapshots over [`Self::tags`].
    features: FeatureStore,
    /// The hook registry (ordered hook specs).
    hooks: HookRegistry,
    /// The permission/veto policy applied when executing hook handlers.
    hook_policy: HookExecutionPolicy,
    /// The handler backend that runs command/http/mcp/prompt/agent handlers.
    hook_backend: SystemHookHandlerBackend,
    /// The language model the eval runner executes cases through. Defaults to the
    /// deterministic offline [`StubLanguageModel`] so `run_eval` never hits the network;
    /// swap in a real client via [`Self::with_language_model`].
    language_model: Arc<dyn LanguageModel>,
    /// The gated writeback queue (knowledge-system B9). When attached,
    /// [`Self::run_eval_at`] promotes the evaluated agent's `Validated` proposals
    /// to `Ready` once its eval `pass_rate` meets the queue's threshold —
    /// closing the loop from "agent proposes" to "evals grant the right to land".
    /// `None` keeps the writeback gate decoupled from evals.
    ///
    /// Held behind a [`tokio::sync::Mutex`] (the same handle the MCP write tools
    /// and the [`KnowledgeRuntime`](tdw_knowledge::runtime::KnowledgeRuntime)
    /// share); `run_eval_at` is sync, so it takes the lock with `blocking_lock`
    /// (`promote_for_agent` is itself sync — no await is needed under the guard).
    proposals: Option<Arc<tokio::sync::Mutex<tdw_knowledge::proposals::ProposalQueue>>>,
    /// The retrieval feedback store (knowledge-system B10). The same `Arc` is
    /// attached to the embedded [`McpServer`] so the `tdw.kg.feedback` MCP tool
    /// and any consumer of `feedback_store_handle()` share one instance.
    /// Always present after construction; never `None` after `assemble`.
    feedback: Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>>,
}

impl AgentBackend {
    /// Build an agent backend from a registry directory of `*.json5` definitions,
    /// applying `policy` to the tool executor.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Load`] if `dir` cannot be read or any
    /// definition is invalid.
    pub fn from_registry_dir(dir: &Path, policy: CommandPolicy) -> BackendResult<Self> {
        let registry = Registry::load_dir(dir)?;
        Ok(Self::assemble(registry, policy))
    }

    /// Build an agent backend from a [`BackendConfig`].
    ///
    /// Resolves the registry directory from [`REGISTRY_DIR_ENV`]: when set, the
    /// directory is loaded; when unset, an empty registry is used so construction
    /// succeeds without a configured registry on disk. The tool-execution policy
    /// is derived from the environment via [`CommandPolicy::from_env`].
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Load`] if [`REGISTRY_DIR_ENV`] is set
    /// but the directory cannot be loaded.
    pub fn from_config(_cfg: &BackendConfig) -> BackendResult<Self> {
        let policy = CommandPolicy::from_env();
        let registry = match registry_dir_from_env() {
            Some(dir) => Registry::load_dir(Path::new(&dir))?,
            None => Registry::new(),
        };
        Ok(Self::assemble(registry, policy))
    }

    /// Compose the facade from an already-resolved registry and command policy.
    fn assemble(registry: Registry, policy: CommandPolicy) -> Self {
        let executor = ToolExecutor::new().with_command_policy(policy.clone());
        let feedback = Arc::new(tokio::sync::Mutex::new(
            tdw_agent_store::RetrievalFeedbackStore::new(),
        ));
        let mcp = McpServer::new()
            .with_registry(registry.clone())
            .with_executor(ToolExecutor::new().with_command_policy(policy))
            .with_feedback_store(Arc::clone(&feedback));
        Self {
            registry,
            executor,
            mcp,
            store: AgentStore::new(),
            kg: KnowledgeGraph::default(),
            tags: TagStore::default(),
            features: FeatureStore::default(),
            hooks: HookRegistry::default(),
            hook_policy: HookExecutionPolicy::default(),
            hook_backend: SystemHookHandlerBackend::new(),
            language_model: Arc::new(StubLanguageModel),
            proposals: None,
            feedback,
        }
    }

    /// A cloned `Arc` handle to the shared retrieval feedback store
    /// (knowledge-system B10). The same handle is wired into the embedded
    /// [`McpServer`] at construction; this accessor lets callers share it with
    /// [`crate::data::Backend`] so `consolidate_now` consumes the same events
    /// the MCP tool appends.
    #[must_use]
    pub fn feedback_store_handle(
        &self,
    ) -> Arc<tokio::sync::Mutex<tdw_agent_store::RetrievalFeedbackStore>> {
        Arc::clone(&self.feedback)
    }

    /// Replace the eval runner's [`LanguageModel`] (default: the offline
    /// [`StubLanguageModel`]). Inject a real client here to run evals against a live
    /// model. Consumes and returns `self` for builder use.
    #[must_use]
    pub fn with_language_model(mut self, model: Arc<dyn LanguageModel>) -> Self {
        self.language_model = model;
        self
    }

    /// Attach the gated writeback queue (knowledge-system B9) so [`Self::run_eval_at`]
    /// promotes the evaluated agent's `Validated` proposals on a passing eval.
    /// Consumes and returns `self` for builder use.
    #[must_use]
    pub fn with_proposals(
        mut self,
        proposals: Arc<tokio::sync::Mutex<tdw_knowledge::proposals::ProposalQueue>>,
    ) -> Self {
        self.proposals = Some(proposals);
        self
    }

    /// Set (or replace) the gated writeback queue in place — the least-invasive
    /// seam for [`Backend`](crate::data::Backend), which is constructed in
    /// `data/mod.rs` and shares one queue handle with the MCP write surface.
    pub fn set_proposals(
        &mut self,
        proposals: Arc<tokio::sync::Mutex<tdw_knowledge::proposals::ProposalQueue>>,
    ) {
        self.proposals = Some(proposals);
    }

    /// Point the embedded [`McpServer`] at a daemon loopback `addr` so its
    /// daemon-backed tools (e.g. `tdw.daemon.query.submit`) submit ops to the
    /// in-process daemon over a loopback [`DaemonClient`](tdw_app_client::DaemonClient),
    /// never via a shared `Arc`.
    ///
    /// This is the library-level counterpart to how
    /// [`server::run_both`](crate::server) wires the standalone MCP loop at the
    /// daemon's [`bound_addr`](crate::data::Backend::bound_addr). It updates the
    /// already-composed server in place via
    /// [`McpServer::set_daemon_config`], so the attached registry + executor (and
    /// the cached tool descriptors) are preserved while the daemon-backed tools
    /// gain a concrete TCP endpoint. Consumes and returns `self` for builder use.
    #[must_use]
    pub fn with_daemon_addr(mut self, addr: &str) -> Self {
        self.mcp.set_daemon_config(DaemonClientConfig::tcp(addr));
        self
    }

    /// Reload the registry from its source directory if any `*.json5` file changed,
    /// re-wiring the MCP server's cached tool descriptors when a reload happened.
    ///
    /// Returns `true` if a reload occurred, `false` if nothing changed or there is no
    /// source directory.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Load`] if the source directory cannot be
    /// re-read or a definition is invalid.
    pub fn reload_if_changed(&mut self) -> BackendResult<bool> {
        let reloaded = self.registry.reload_if_changed()?;
        if reloaded {
            self.mcp.set_registry(self.registry.clone());
        }
        Ok(reloaded)
    }

    /// Load a plugin registry rooted at `root`, merge it into the active registry, then
    /// re-wire the MCP server's cached tool descriptors.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Load`] if a plugin directory cannot be loaded,
    /// a `(kind, name)` collides across the merge, or a plugin references a missing member.
    pub fn load_plugins(&mut self, root: &Path) -> BackendResult<()> {
        let plugins = Registry::load_plugins(root)?;
        self.registry.merge(plugins)?;
        self.mcp.set_registry(self.registry.clone());
        Ok(())
    }

    /// The names of the `tool` resources the registry knows about, deduplicated and sorted.
    #[must_use]
    pub fn list_tools(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .registry
            .by_kind(EntityKind::Tool)
            .map(|resource| resource.metadata.base.name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Resolve and execute a registry `tool` by `name` with `args`, returning its
    /// structured outcome.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Exec`] if the tool is unknown, unbound, rejected
    /// by policy, or fails to execute.
    pub fn call_tool(&self, name: &str, args: &serde_json::Value) -> BackendResult<ToolOutcome> {
        Ok(self.executor.execute(&self.registry, name, args)?)
    }

    /// Mutable access to the embedded MCP server (e.g. to drive a JSON-RPC session).
    pub const fn mcp_server(&mut self) -> &mut McpServer {
        &mut self.mcp
    }

    /// Handle a single line of MCP JSON-RPC, returning the encoded response message(s).
    pub fn handle_mcp_line(&mut self, line: &str) -> Vec<String> {
        self.mcp.handle_json_rpc_line(line)
    }

    /// Compile a [`WorkflowDefinition`] into an [`ExecutionPlan`] (topologically ordered
    /// node ids) by validating it against the agent contract.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Contract`] if the workflow violates the
    /// contract (invalid identifiers, dangling edges, cycles).
    pub fn compile_workflow(&self, wf: &WorkflowDefinition) -> BackendResult<ExecutionPlan> {
        Ok(WorkflowEngine::compile(wf)?)
    }

    /// Run an evaluation request against the embedded [`AgentStore`], executing each case
    /// through the configured [`LanguageModel`], persisting the run, then closing the loop by
    /// feeding the run's `pass_rate` back into the evaluated agent's skills (Phase C2),
    /// GATED by the [`Adaptivity`](tdw_agent::Adaptivity) axis. Returns the run outcome.
    ///
    /// Feedback uses the live wall clock (`chrono::Utc::now().to_rfc3339()`); the
    /// deterministic core is [`Self::run_eval_at`].
    pub fn run_eval(&mut self, request: EvalRunRequest) -> EvalRunOutcome {
        let now = chrono::Utc::now().to_rfc3339();
        self.run_eval_at(request, &now)
    }

    /// Deterministic core of [`Self::run_eval`]: execute the run, then apply gated eval
    /// feedback using the injected `now` (RFC 3339) so the mutation is reproducible in tests.
    ///
    /// After the runner records the [`StoredEvalRun`], this extracts the `pass_rate` metric,
    /// looks up the evaluated [`AgentCard`], and for EACH skill calls
    /// [`AgentSkill::apply_eval_feedback`](tdw_agent::AgentSkill::apply_eval_feedback):
    /// skills whose adaptivity is `>= Learning` are updated; skills the gate rejects are
    /// skipped (not an error). The mutated card is re-upserted, and the stored run is
    /// re-recorded with the `updated_skills` backlink.
    ///
    /// # G008 theme 10 — ER3: stub-feedback gate
    ///
    /// Skill mutation is **refused** when the configured [`LanguageModel`] is not
    /// production-grade (i.e. [`LanguageModel::is_production_grade`] returns `false`,
    /// which is the default for the offline `StubLanguageModel`).  The stub echoes
    /// grounding context verbatim, so its eval passes are pure URI-containment
    /// tautologies — not model-quality signals.  Mutating agent skills on stub
    /// results would silently tune them on noise.
    ///
    /// The gate only suppresses the *mutation* step; metrics are still computed and
    /// the run is still recorded, so CI pass/fail counts remain meaningful.
    ///
    /// To bypass in a controlled context (e.g. an integration test that deliberately
    /// exercises the feedback path with a stub) set `TDW_ALLOW_STUB_FEEDBACK=1` in
    /// the environment.  The bypass is loud: a warning is printed to stderr so it is
    /// never invisible in logs.
    pub fn run_eval_at(&mut self, request: EvalRunRequest, now: &str) -> EvalRunOutcome {
        // The runner consumes `request`; capture the evaluated agent id first for feedback.
        let agent_id = request.agent_id.clone();
        let runner = EvalRunner::new(Arc::clone(&self.language_model));
        let outcome = runner.run(request, &mut self.store);

        let pass_rate = outcome
            .metrics
            .iter()
            .find(|metric| metric.metric_name == "pass_rate")
            .map(|metric| metric.metric_value);

        // G008/ER3: gate skill mutation on the model being production-grade.
        // Stub models produce containment tautologies, not quality signals.
        // In tests, `set_allow_stub_feedback_for_test(true)` may set the
        // thread-local override so the feedback path is exercisable without
        // unsafe env manipulation.
        let feedback_allowed = self.language_model.is_production_grade()
            || std::env::var("TDW_ALLOW_STUB_FEEDBACK").as_deref() == Ok("1")
            || {
                #[cfg(test)]
                {
                    #[allow(clippy::redundant_closure_for_method_calls)] // Cell::get requires an import; closure form is clearer here
                    ALLOW_STUB_FEEDBACK_FOR_TEST.with(|cell| cell.get())
                }
                #[cfg(not(test))]
                {
                    false
                }
            };

        if !feedback_allowed {
            eprintln!(
                "tdw-backend: eval feedback suppressed — model {:?} is not \
                 production-grade (stub/echo). Metrics were recorded; skill \
                 mutation was skipped to prevent tuning on tautological stub \
                 results. Set TDW_ALLOW_STUB_FEEDBACK=1 to bypass (for \
                 controlled testing only).",
                self.language_model.model_id()
            );
        }

        if feedback_allowed
            && let Some(pass_rate) = pass_rate
            && let Some(mut card) = self.store.agent(&agent_id).cloned()
        {
            let mut updated_skills = Vec::new();
            for skill in &mut card.skills {
                // The gate (adaptivity < Learning) skips a skill without aborting the run.
                if skill
                    .apply_eval_feedback(pass_rate, now, FEEDBACK_DISABLE_BELOW)
                    .is_ok()
                {
                    updated_skills.push(skill.meta.id.clone());
                }
            }
            if !updated_skills.is_empty() {
                self.store.upsert_agent(card);
                if let Some(mut stored) = self.store.eval_run(&outcome.run_id).cloned() {
                    stored.updated_skills = updated_skills;
                    self.store.record_eval_run(stored);
                }
            }
        }

        // Writeback gate promotion (knowledge-system B9): a passing eval grants
        // the evaluated agent's `Validated` proposals the right to LAND.
        // `promote_for_agent` requires at least MIN_EVAL_CASES — extract the
        // case_count from the metrics so a vacuous eval (0 cases, pass_rate 1.0)
        // does not promote.
        let case_count = outcome
            .metrics
            .iter()
            .find(|metric| metric.metric_name == "case_count")
            .map_or(0_usize, |metric| {
                // case_count is a non-negative integer stored as f64 by the eval
                // harness. Negative or NaN → 0; values ≥ u32::MAX are astronomically
                // large eval suites that saturate to usize::MAX. u32::MAX fits f64
                // exactly (2^32-1 < 2^53), so the comparison is lossless.
                let v = metric.metric_value;
                if !v.is_finite() || v < 0.0 {
                    0
                } else if v >= f64::from(u32::MAX) {
                    usize::MAX
                } else {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let n = v as usize;
                    n
                }
            });

        // G008/ER3: gate proposal promotion on the same stub predicate as skill
        // feedback. A stub-echo eval produces tautological pass_rates; promoting
        // knowledge proposals on tautologies would silently advance unvalidated
        // graph mutations into the durable store.
        if feedback_allowed
            && let Some(pass_rate) = pass_rate
            && let Some(proposals) = self.proposals.as_ref()
        {
            // The queue is shared (tokio::sync::Mutex) with the MCP write
            // surface. `run_eval_at` is sync; use `blocking_lock` so the
            // promotion is never silently skipped on a contended lock.
            let date = now.split('T').next().unwrap_or(now);
            let mut queue = proposals.blocking_lock();
            let _ = queue.promote_for_agent(&agent_id, pass_rate, case_count, date);
        }

        outcome
    }

    /// Upsert an [`AgentCard`] into the embedded store.
    pub fn upsert_agent(&mut self, card: AgentCard) {
        self.store.upsert_agent(card);
    }

    /// Look up a stored [`AgentCard`] by id.
    #[must_use]
    pub fn agent(&self, id: &str) -> Option<&AgentCard> {
        self.store.agent(id)
    }

    /// Upsert a [`WorkflowDefinition`] into the embedded store.
    pub fn upsert_workflow(&mut self, wf: WorkflowDefinition) {
        self.store.upsert_workflow(wf);
    }

    /// Look up a stored [`WorkflowDefinition`] by id.
    #[must_use]
    pub fn workflow(&self, id: &str) -> Option<&WorkflowDefinition> {
        self.store.workflow(id)
    }

    // --- Knowledge graph (sync) --------------------------------------------

    /// Shared access to the embedded [`KnowledgeGraph`].
    #[must_use]
    pub const fn kg(&self) -> &KnowledgeGraph {
        &self.kg
    }

    /// Mutable access to the embedded [`KnowledgeGraph`].
    pub const fn kg_mut(&mut self) -> &mut KnowledgeGraph {
        &mut self.kg
    }

    /// Insert or replace an [`Entity`] in the knowledge graph.
    pub fn upsert_entity(&mut self, entity: Entity) {
        self.kg.upsert_entity(entity);
    }

    /// Append a [`Relationship`] edge to the knowledge graph.
    pub fn add_relationship(&mut self, rel: Relationship) {
        self.kg.add_relationship(rel);
    }

    /// Look up an [`Entity`] by id.
    #[must_use]
    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.kg.entity(id)
    }

    /// The entities reachable by one outgoing edge from `id`.
    #[must_use]
    pub fn neighbors(&self, id: &str) -> Vec<&Entity> {
        self.kg.neighbors(id)
    }

    // --- Tags (sync) -------------------------------------------------------

    /// Shared access to the embedded [`TagStore`].
    #[must_use]
    pub const fn tags(&self) -> &TagStore {
        &self.tags
    }

    /// Mutable access to the embedded [`TagStore`].
    pub const fn tags_mut(&mut self) -> &mut TagStore {
        &mut self.tags
    }

    /// Define a [`TagDefinition`] in the taxonomy.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Tag`] if the tag id is invalid, the
    /// parent is unknown, or the definition introduces a cycle.
    pub fn define_tag(&mut self, def: TagDefinition) -> BackendResult<()> {
        self.tags.define(def)?;
        Ok(())
    }

    /// Assign a [`TagAssignment`] to an entity.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Tag`] if the assignment is invalid
    /// or references an unknown tag.
    pub fn assign_tag(&mut self, a: TagAssignment) -> BackendResult<()> {
        self.tags.assign(a)?;
        Ok(())
    }

    /// The tag ids active for `entity_id` as of `as_of` (a `YYYY-MM-DD` date).
    #[must_use]
    pub fn active_tags(&self, entity_id: &str, as_of: &str) -> Vec<String> {
        self.tags.active_tags(entity_id, as_of)
    }

    // --- Feature store (sync) ----------------------------------------------

    /// Shared access to the embedded [`FeatureStore`].
    #[must_use]
    pub const fn features(&self) -> &FeatureStore {
        &self.features
    }

    /// Mutable access to the embedded [`FeatureStore`].
    pub const fn features_mut(&mut self) -> &mut FeatureStore {
        &mut self.features
    }

    /// Materialize a [`FeatureSnapshot`] for `entity_id` as of `as_of`, joining
    /// the active tags from the embedded [`TagStore`].
    ///
    /// The call borrows `self.features` mutably and `self.tags` immutably; these
    /// are distinct fields, so the split borrow is accepted by the borrow
    /// checker.
    pub fn materialize_features(
        &mut self,
        entity_id: &str,
        as_of: &str,
        features: BTreeMap<String, f64>,
    ) -> FeatureSnapshot {
        self.features
            .materialize(entity_id, as_of, features, &self.tags)
    }

    /// The most recently materialized [`FeatureSnapshot`] for `entity_id`.
    #[must_use]
    pub fn latest_features(&self, entity_id: &str) -> Option<&FeatureSnapshot> {
        self.features.latest(entity_id)
    }

    // --- Hooks (sync) ------------------------------------------------------

    /// Register a [`HookSpec`] into the embedded registry (kept ordered by the
    /// registry on insert).
    pub fn register_hook(&mut self, hook: HookSpec) {
        self.hooks.register(hook);
    }

    /// The names of the registered hooks, in registry order.
    #[must_use]
    pub fn hook_names(&self) -> Vec<String> {
        self.hooks.hook_names()
    }

    /// Execute the registered hook handlers for `envelope` under the configured
    /// [`HookExecutionPolicy`], returning each handler's structured outcome.
    ///
    /// The call splits three distinct fields: `self.hooks` (`&mut`),
    /// `self.hook_policy` (`&`), and `self.hook_backend` (`&mut`); the borrow
    /// checker accepts the disjoint borrows.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BackendError::Hook`] if a hook spec is invalid,
    /// the recursion/depth guard trips, a handler is denied/requires approval,
    /// a veto is denied, or a handler fails.
    pub fn run_hooks(
        &mut self,
        envelope: &EventEnvelope<serde_json::Value>,
    ) -> BackendResult<Vec<HookExecutionOutcome>> {
        Ok(self
            .hooks
            .execute_handlers(envelope, &self.hook_policy, &mut self.hook_backend)?)
    }
}

/// Read the registry directory from [`REGISTRY_DIR_ENV`], treating an unset or
/// blank value as "not configured".
fn registry_dir_from_env() -> Option<String> {
    std::env::var(REGISTRY_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

// Thread-local flag used only in unit tests (G008/ER3) to enable stub
// feedback without `unsafe` env-var manipulation. The flag is checked by
// `run_eval_at` alongside `TDW_ALLOW_STUB_FEEDBACK` only when `cfg(test)` is
// active, so it has zero runtime cost in production builds.
#[cfg(test)]
thread_local! {
    static ALLOW_STUB_FEEDBACK_FOR_TEST: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Set the per-thread stub-feedback override (test use only).
///
/// Pass `true` to allow `run_eval_at` to apply skill mutation even when
/// the configured model is not production-grade; pass `false` to restore the
/// default (gate applies). The flag is thread-local, so parallel tests do not
/// interfere.
#[cfg(test)]
fn set_allow_stub_feedback_for_test(allow: bool) {
    ALLOW_STUB_FEEDBACK_FOR_TEST.with(|cell| cell.set(allow));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use tdw_agent::{
        Adaptivity, EntityMeta, EvalCase, Origin, Source, Tier, WorkflowEdge, WorkflowNode,
        sample_agent_card,
    };
    use tdw_tool_exec::ExecError;

    /// A `search` tool fixture (defaults to `Unbound`), mirroring
    /// `crates/tdw-agent/tests/registry/tool_search.json5`.
    const TOOL_SEARCH_JSON5: &str = r#"{
        apiVersion: "tdw.finx/v1",
        kind: "tool",
        metadata: {
            name: "search", title: "Search", id: "search", version: "0.1.0",
            origin: { tier: "Domain", source: "Internal" },
            adaptivity: "None", autonomous: false,
        },
        spec: { input_schema: { type: "object" }, output_schema: null,
                effect: "ReadOnly", idempotent: true, open_world: false },
    }"#;

    /// Deny-all command policy: tool execution never depends on host commands in tests.
    fn deny_all_policy() -> CommandPolicy {
        CommandPolicy::new(None, Duration::from_secs(30))
    }

    /// Process-unique counter so parallel tests never share a temp registry dir (a coarse
    /// wall-clock resolution under the same PID could otherwise collide).
    static DIR_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Write the `search` tool fixture into a fresh temp registry dir and build a backend.
    fn backend_with_search_tool() -> (std::path::PathBuf, AgentBackend) {
        let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("tdw_backend_agent_{}_{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir temp registry dir");
        std::fs::write(dir.join("tool_search.json5"), TOOL_SEARCH_JSON5).expect("write fixture");
        let backend = AgentBackend::from_registry_dir(&dir, deny_all_policy())
            .expect("backend should build from registry dir");
        (dir, backend)
    }

    fn workflow_meta(name: &str) -> EntityMeta {
        EntityMeta::new(
            name,
            name,
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::Configured,
            false,
        )
    }

    #[test]
    fn list_tools_includes_registry_tool() {
        let (dir, backend) = backend_with_search_tool();
        assert!(backend.list_tools().contains(&"search".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn call_tool_on_unbound_registry_tool_returns_unbound() {
        let (dir, backend) = backend_with_search_tool();
        // The `search` fixture declares no `implementation`, so it defaults to `Unbound`:
        // listed truthfully, but not yet executable.
        let error = backend
            .call_tool("search", &serde_json::json!({}))
            .expect_err("unbound tool must error");
        assert!(matches!(
            error,
            crate::error::BackendError::Exec(ExecError::Unbound)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compile_workflow_returns_execution_plan() {
        let (dir, backend) = backend_with_search_tool();
        let workflow = WorkflowDefinition {
            meta: workflow_meta("research-flow"),
            nodes: vec![
                WorkflowNode {
                    node_id: "retrieve".to_string(),
                    task: "retrieve".to_string(),
                    skill_id: None,
                },
                WorkflowNode {
                    node_id: "draft".to_string(),
                    task: "draft".to_string(),
                    skill_id: Some("research.note".to_string()),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "retrieve".to_string(),
                to: "draft".to_string(),
            }],
        };
        let plan = backend
            .compile_workflow(&workflow)
            .expect("workflow should compile");
        assert_eq!(plan.ordered_node_ids, vec!["retrieve", "draft"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_eval_returns_outcome() {
        let (dir, mut backend) = backend_with_search_tool();
        // The runner reports `agent_card_coverage` against the embedded store, so the request's
        // agent must be present for a `success` outcome.
        backend.upsert_agent(sample_agent_card());
        let outcome = backend.run_eval(EvalRunRequest {
            run_id: "eval-1".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: Vec::new(),
            }],
        });
        assert_eq!(outcome.run_id, "eval-1");
        assert_eq!(outcome.status, "success");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // G008/ER3: stub model must NOT mutate skills without TDW_ALLOW_STUB_FEEDBACK=1.
    #[test]
    fn stub_model_feedback_suppressed_without_opt_in() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        // SAFETY: test-only, single-threaded context (cargo test default).
        set_allow_stub_feedback_for_test(false);

        let (dir, mut backend) = backend_with_search_tool();
        // StubLanguageModel is default — is_production_grade() returns false.
        let skill_meta = |name: &str| {
            EntityMeta::new(
                name,
                name,
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                false,
            )
            .with_title(name)
            .with_description("A skill.")
        };
        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![AgentSkill {
                meta: skill_meta("research.note"),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                quality: None,
            }],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "er3-gate".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: vec![ContentRef {
                        uri: "tdw://docs/research-template".to_string(),
                        kind: ContentKind::Prompt,
                        checksum: None,
                        tags: Vec::new(),
                    }],
                }],
            },
            "2026-06-11T00:00:00+00:00",
        );
        // Metrics are still produced (CI use-case).
        assert_eq!(outcome.status, "success");

        // Skill quality must be None: stub feedback was suppressed.
        let updated = backend.agent("market-researcher").expect("card present");
        assert!(
            updated.skills[0].quality.is_none(),
            "stub feedback must not mutate skill quality without TDW_ALLOW_STUB_FEEDBACK=1"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // G008/ER3 F1 regression: stub eval that clears the MIN_EVAL_CASES floor
    // and passes the 0.8 threshold must NOT promote proposals, because
    // feedback_allowed is false for the stub model.
    #[test]
    fn stub_eval_passing_floor_does_not_promote_proposals() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        use tdw_knowledge::proposals::ProposalQueue;
        // Ensure the thread-local bypass is OFF — we want the gate to fire.
        set_allow_stub_feedback_for_test(false);

        let (dir, mut backend) = backend_with_search_tool();
        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![AgentSkill {
                meta: EntityMeta::new(
                    "research.note",
                    "research.note",
                    "0.1.0",
                    Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                    Adaptivity::Learning,
                    false,
                )
                .with_title("research.note")
                .with_description("A skill."),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                quality: None,
            }],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        let queue: ProposalQueue = serde_json::from_value(serde_json::json!({
            "proposals": {
                "p-stub": {
                    "id": "p-stub",
                    "kind": { "kind": "tag_define", "tag_id": "asset:equity", "parent": null },
                    "agent_id": "market-researcher",
                    "status": "validated",
                    "history": ["2026-06-11 validated"],
                }
            },
            "next_id": 1,
        }))
        .expect("queue deserializes");
        let proposals = Arc::new(tokio::sync::Mutex::new(queue));
        backend.set_proposals(Arc::clone(&proposals));

        // 5 cases that all pass (pass_rate 1.0 >= 0.8, case_count 5 >= MIN_EVAL_CASES).
        let cases = (1_u8..=5)
            .map(|i| EvalCase {
                case_id: format!("case-{i}"),
                prompt: format!("Summarize AAPL ({i})"),
                expected_refs: vec![ContentRef {
                    uri: "tdw://docs/research-template".to_string(),
                    kind: ContentKind::Prompt,
                    checksum: None,
                    tags: Vec::new(),
                }],
            })
            .collect();
        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-stub-nopromote".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden".to_string(),
                cases,
            },
            "2026-06-11T00:00:00+00:00",
        );
        assert_eq!(outcome.status, "success");

        // Proposal must remain Validated — stub model gate blocked promotion.
        let status = {
            let queue = proposals.blocking_lock();
            queue.get("p-stub").expect("proposal present").status
        };
        assert_eq!(
            status,
            tdw_agent::ValidationStatus::Validated,
            "stub-model eval must not promote proposals even when floor+threshold pass"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // G008/ER3 F1 regression: with the bypass active (thread-local), a passing
    // 5-case eval DOES promote the Validated proposal to Ready.
    #[test]
    #[allow(clippy::unnecessary_literal_bound)] // assert_eq!(String, &str) is idiomatic in tests
    fn bypass_enabled_passing_eval_promotes_proposals() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        use tdw_knowledge::proposals::ProposalQueue;
        // Enable the thread-local bypass so promotion is allowed.
        set_allow_stub_feedback_for_test(true);

        let (dir, mut backend) = backend_with_search_tool();
        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![AgentSkill {
                meta: EntityMeta::new(
                    "research.note",
                    "research.note",
                    "0.1.0",
                    Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                    Adaptivity::Learning,
                    false,
                )
                .with_title("research.note")
                .with_description("A skill."),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                quality: None,
            }],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        let queue: ProposalQueue = serde_json::from_value(serde_json::json!({
            "proposals": {
                "p-bypass": {
                    "id": "p-bypass",
                    "kind": { "kind": "tag_define", "tag_id": "asset:equity", "parent": null },
                    "agent_id": "market-researcher",
                    "status": "validated",
                    "history": ["2026-06-11 validated"],
                }
            },
            "next_id": 1,
        }))
        .expect("queue deserializes");
        let proposals = Arc::new(tokio::sync::Mutex::new(queue));
        backend.set_proposals(Arc::clone(&proposals));

        let cases = (1_u8..=5)
            .map(|i| EvalCase {
                case_id: format!("case-{i}"),
                prompt: format!("Summarize AAPL ({i})"),
                expected_refs: vec![ContentRef {
                    uri: "tdw://docs/research-template".to_string(),
                    kind: ContentKind::Prompt,
                    checksum: None,
                    tags: Vec::new(),
                }],
            })
            .collect();
        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-bypass-promote".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden".to_string(),
                cases,
            },
            "2026-06-11T00:00:00+00:00",
        );
        assert_eq!(outcome.status, "success");

        let status = {
            let queue = proposals.blocking_lock();
            queue.get("p-bypass").expect("proposal present").status
        };
        assert_eq!(
            status,
            tdw_agent::ValidationStatus::Ready,
            "bypass-enabled passing eval must promote proposal to Ready"
        );
        let _ = std::fs::remove_dir_all(&dir);
        set_allow_stub_feedback_for_test(false);
    }

    // G008/ER3 F3: a novel unnamed model that does not override
    // is_production_grade() must default to false (fail-safe).
    #[test]
    #[allow(clippy::items_after_statements)] // local struct must follow the use import; reordering would obscure intent
    fn novel_unknown_model_defaults_to_not_production_grade() {
        use tdw_llm::{ChatRequest, ChatResponse, MessageRole, ChatMessage, Usage};
        struct NovelModel;
        impl tdw_llm::LanguageModel for NovelModel {
            fn model_id(&self) -> &str {
                "some-future-model-v99"
            }
            fn complete(&self, _: ChatRequest) -> tdw_llm::Result<ChatResponse> {
                Ok(ChatResponse {
                    model_id: self.model_id().to_string(),
                    message: ChatMessage { role: MessageRole::Assistant, content: String::new() },
                    usage: Usage { input_tokens: 0, output_tokens: 0 },
                })
            }
        }
        // Does NOT override is_production_grade — must inherit the fail-safe default.
        assert!(
            !NovelModel.is_production_grade(),
            "a model that does not override is_production_grade must default to false"
        );
    }

    // G008/ER3: with TDW_ALLOW_STUB_FEEDBACK=1 the feedback path is reachable.
    #[test]
    fn run_eval_applies_gated_feedback_to_learning_skill_only() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        // SAFETY: test-only, single-threaded context (cargo test default).
        set_allow_stub_feedback_for_test(true);

        let (dir, mut backend) = backend_with_search_tool();

        let skill = |name: &str, adaptivity: Adaptivity| AgentSkill {
            meta: EntityMeta::new(
                name,
                name,
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                adaptivity,
                false,
            )
            .with_title(name)
            .with_description("A skill."),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            quality: None,
        };

        // A card with two skills: one Learning (gate passes) and one Configured (gate skips).
        // The content_ref URI is surfaced by the stub's echoed grounding context, so a case
        // expecting that URI passes -> pass_rate 1.0 (>= threshold, skill stays enabled).
        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![
                skill("research.note", Adaptivity::Learning),
                skill("research.summary", Adaptivity::Configured),
            ],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-c2".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: vec![ContentRef {
                        uri: "tdw://docs/research-template".to_string(),
                        kind: ContentKind::Prompt,
                        checksum: None,
                        tags: Vec::new(),
                    }],
                }],
            },
            "2026-05-31T00:00:00+00:00",
        );
        assert_eq!(outcome.status, "success");

        let updated = backend.agent("market-researcher").expect("card present");
        let learning = &updated.skills[0];
        let configured = &updated.skills[1];

        // The Learning skill received feedback: runs=1, pass_rate=1.0, enabled.
        let quality = learning
            .quality
            .as_ref()
            .expect("learning skill quality set");
        assert_eq!(quality.runs, 1);
        assert_eq!(quality.pass_rate, Some(1.0));
        assert!(!quality.disabled);
        assert_eq!(
            quality.last_eval.as_deref(),
            Some("2026-05-31T00:00:00+00:00")
        );

        // The Configured skill was gate-skipped: quality stays None.
        assert!(configured.quality.is_none());

        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only cleanup.
        set_allow_stub_feedback_for_test(false);
    }

    #[test]
    fn run_eval_promotes_attached_proposals_on_a_passing_eval() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        // G008/ER3: this test exercises the proposal-promotion path (not skill
        // mutation), but the backend stub-feedback gate is also active, so set
        // TDW_ALLOW_STUB_FEEDBACK=1 so skill feedback doesn't print a warning.
        // SAFETY: test-only, single-threaded context (cargo test default).
        set_allow_stub_feedback_for_test(true);
        use tdw_knowledge::proposals::ProposalQueue;

        let (dir, mut backend) = backend_with_search_tool();

        // A `market-researcher` whose single Learning skill passes (pass_rate 1.0),
        // mirroring `run_eval_applies_gated_feedback_to_learning_skill_only`.
        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![AgentSkill {
                meta: EntityMeta::new(
                    "research.note",
                    "research.note",
                    "0.1.0",
                    Origin {
                        tier: Tier::Domain,
                        source: Source::Internal,
                    },
                    Adaptivity::Learning,
                    false,
                )
                .with_title("research.note")
                .with_description("A skill."),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                quality: None,
            }],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        // A queue holding ONE Validated proposal by `market-researcher`,
        // constructed via serde so this test needs no graph/tag engine deps.
        let queue: ProposalQueue = serde_json::from_value(serde_json::json!({
            "proposals": {
                "p1": {
                    "id": "p1",
                    "kind": { "kind": "tag_define", "tag_id": "asset:equity", "parent": null },
                    "agent_id": "market-researcher",
                    "status": "validated",
                    "history": ["2026-05-31 validated"],
                }
            },
            "next_id": 1,
        }))
        .expect("queue deserializes");
        let proposals = Arc::new(tokio::sync::Mutex::new(queue));
        backend.set_proposals(Arc::clone(&proposals));

        // Run 5 cases (MIN_EVAL_CASES = 5) so the floor guard passes.
        let cases = (1_u8..=5)
            .map(|i| EvalCase {
                case_id: format!("case-{i}"),
                prompt: format!("Summarize AAPL ({i})"),
                expected_refs: vec![ContentRef {
                    uri: "tdw://docs/research-template".to_string(),
                    kind: ContentKind::Prompt,
                    checksum: None,
                    tags: Vec::new(),
                }],
            })
            .collect();
        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-b9".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases,
            },
            "2026-05-31T00:00:00+00:00",
        );
        assert_eq!(outcome.status, "success");

        // The passing eval with >= MIN_EVAL_CASES promoted the Validated proposal to Ready.
        let status = {
            let queue = proposals.blocking_lock();
            queue.get("p1").expect("proposal present").status
        };
        assert_eq!(status, tdw_agent::ValidationStatus::Ready);

        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only cleanup.
        set_allow_stub_feedback_for_test(false);
    }

    #[test]
    fn agent_crud_round_trips() {
        let (dir, mut backend) = backend_with_search_tool();
        let card = sample_agent_card();
        backend.upsert_agent(card.clone());
        assert_eq!(backend.agent("market-researcher"), Some(&card));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn workflow_crud_round_trips() {
        let (dir, mut backend) = backend_with_search_tool();
        let workflow = WorkflowDefinition {
            meta: workflow_meta("research-flow"),
            nodes: vec![WorkflowNode {
                node_id: "retrieve".to_string(),
                task: "retrieve".to_string(),
                skill_id: None,
            }],
            edges: Vec::new(),
        };
        backend.upsert_workflow(workflow.clone());
        assert_eq!(backend.workflow("research-flow"), Some(&workflow));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handle_mcp_line_initializes_via_embedded_server() {
        let (dir, mut backend) = backend_with_search_tool();
        let responses = backend.handle_mcp_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1.0.0"}}}"#,
        );
        assert_eq!(responses.len(), 1);
        assert!(responses[0].contains("\"protocolVersion\""));
        assert!(backend.mcp_server().is_initialized());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kg_upsert_entity_and_neighbors_round_trip() {
        use tdw_kg::EntityKind;

        let (dir, mut backend) = backend_with_search_tool();
        // Construction mirrors `tdw-kg`'s own
        // `queries_entities_edges_and_manual_merge_audit` fixture.
        backend.upsert_entity(Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        });
        backend.upsert_entity(Entity {
            entity_id: "dataset:ohlcv".to_string(),
            kind: EntityKind::Dataset,
            label: "OHLCV".to_string(),
            aliases: Vec::new(),
        });
        backend.add_relationship(Relationship {
            from: "instrument:AAPL".to_string(),
            to: "dataset:ohlcv".to_string(),
            rel_type: "has_prices".to_string(),
            provenance: "fixture".to_string(),
        });

        assert_eq!(
            backend.entity("instrument:AAPL").map(|e| e.label.clone()),
            Some("Apple".to_string())
        );
        assert_eq!(
            backend.neighbors("instrument:AAPL")[0].entity_id,
            "dataset:ohlcv"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_define_assign_and_active_round_trip() {
        let (dir, mut backend) = backend_with_search_tool();
        // Construction mirrors `tdw-tags`'s own
        // `manages_tag_dag_ttl_provenance_and_stats` fixture.
        backend
            .define_tag(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        backend
            .assign_tag(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "manual".to_string(),
            })
            .unwrap_or_else(|error| panic!("assignment should persist: {error}"));

        assert_eq!(
            backend.active_tags("instrument:AAPL", "2026-05-22"),
            vec!["asset:equity".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn features_materialize_and_latest_round_trip() {
        let (dir, mut backend) = backend_with_search_tool();
        // Construction mirrors `tdw-feature-store`'s own
        // `materializes_feature_snapshot_with_active_tags` fixture.
        backend
            .define_tag(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        backend
            .assign_tag(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "manual".to_string(),
            })
            .unwrap_or_else(|error| panic!("assignment should persist: {error}"));

        let mut features = BTreeMap::new();
        features.insert("return_1d".to_string(), 0.01);
        let snapshot = backend.materialize_features("instrument:AAPL", "2026-05-21", features);
        assert_eq!(snapshot.as_of, "2026-05-21");
        assert_eq!(snapshot.tags, vec!["asset:equity".to_string()]);

        assert_eq!(
            backend
                .latest_features("instrument:AAPL")
                .map(|latest| latest.as_of.clone()),
            Some("2026-05-21".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Write the `search` tool fixture into a fresh temp registry dir, returning the dir
    /// so the caller can mutate it (add members) before/after building a backend.
    fn fresh_registry_dir() -> std::path::PathBuf {
        let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "tdw_backend_agent_reg_{}_{seq}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir temp registry dir");
        std::fs::write(dir.join("tool_search.json5"), TOOL_SEARCH_JSON5).expect("write fixture");
        dir
    }

    /// A `tool` fixture for an arbitrary `name` (defaults to `Unbound`), used to add new
    /// registry members on disk so a reload/merge has an observable effect.
    fn tool_fixture(name: &str) -> String {
        format!(
            r#"{{
                apiVersion: "tdw.finx/v1",
                kind: "tool",
                metadata: {{
                    name: "{name}", title: "{name}", id: "{name}", version: "0.1.0",
                    origin: {{ tier: "Domain", source: "Internal" }},
                    adaptivity: "None", autonomous: false,
                }},
                spec: {{ input_schema: {{ type: "object" }}, output_schema: null,
                        effect: "ReadOnly", idempotent: true, open_world: false }},
            }}"#
        )
    }

    #[test]
    fn from_config_with_unset_registry_env_builds_empty_registry() {
        // from_config's None arm (REGISTRY_DIR_ENV unset) uses Registry::new(), which has
        // no tools — distinct from the from_registry_dir path that lists "search". Guard
        // the assumption with a read-only env check (no mutation): if the runner has the
        // env set, the empty-tools property no longer holds, so self-skip that assertion.
        let backend = AgentBackend::from_config(&BackendConfig::default())
            .expect("from_config should build with an unset registry env");
        if std::env::var(REGISTRY_DIR_ENV).is_err() {
            assert!(
                backend.list_tools().is_empty(),
                "Registry::new() (None arm) yields no tools",
            );
        }
    }

    #[test]
    fn with_language_model_injected_model_drives_eval_run() {
        // Build a backend, install a language model via the builder, and prove the
        // injected model is the one the EvalRunner executes through by observing a
        // successful eval outcome (run_eval_at clones self.language_model into the runner).
        // G008/ER3: StubLanguageModel is not production-grade; allow stub feedback
        // so the run completes without a warning.
        // SAFETY: test-only, single-threaded context (cargo test default).
        set_allow_stub_feedback_for_test(true);
        let (dir, mut backend) = backend_with_search_tool();
        backend = backend.with_language_model(Arc::new(StubLanguageModel));
        backend.upsert_agent(sample_agent_card());
        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-lm".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: Vec::new(),
                }],
            },
            "2026-06-07T00:00:00+00:00",
        );
        assert_eq!(outcome.run_id, "eval-lm");
        assert_eq!(outcome.status, "success");
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only cleanup.
        set_allow_stub_feedback_for_test(false);
    }

    #[test]
    fn reload_if_changed_noop_then_reloads_after_new_member() {
        let dir = fresh_registry_dir();
        let mut backend = AgentBackend::from_registry_dir(&dir, deny_all_policy())
            .expect("backend should build from registry dir");
        // No change: scan_stamps matches the recorded stamps -> Ok(false).
        assert!(
            !backend.reload_if_changed().expect("no-change scan"),
            "reload_if_changed must report false when nothing changed",
        );
        assert!(backend.list_tools().contains(&"search".to_string()));
        assert!(!backend.list_tools().contains(&"quote".to_string()));

        // Add a NEW *.json5 member: the stamps BTreeMap (keyed on file path) changes
        // regardless of mtime resolution, so the next poll reloads and re-wires the MCP
        // registry. Asserting the new tool is now listed proves set_registry took effect.
        std::fs::write(dir.join("tool_quote.json5"), tool_fixture("quote")).expect("write quote");
        assert!(
            backend.reload_if_changed().expect("changed scan"),
            "reload_if_changed must report true after a new member is added",
        );
        assert!(
            backend.list_tools().contains(&"quote".to_string()),
            "the reloaded registry must list the newly-added tool",
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_plugins_merges_happy_path_and_rejects_collision() {
        // Happy path: a plugin root whose immediate subdir is a plugin holding a manifest +
        // a distinct member tool. load_plugins (registry.rs) iterates SUBDIRECTORIES, so the
        // layout MUST be root/<plugin>/*.json5 (a flat *.json5 under root is skipped).
        let (base_dir, mut backend) = backend_with_search_tool();

        let seq = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tdw_backend_agent_plug_{}_{seq}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&root);
        let plugin_dir = root.join("extra");
        std::fs::create_dir_all(&plugin_dir).expect("mkdir plugin dir");
        std::fs::write(
            plugin_dir.join("plugin.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"plugin",
                 metadata:{ name:"extra", id:"extra", version:"0.1.0",
                   origin:{tier:"Custom",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ members:["plugin_tool"], client_side:false, server_side:true } }"#,
        )
        .expect("write plugin manifest");
        std::fs::write(plugin_dir.join("tool.json5"), tool_fixture("plugin_tool"))
            .expect("write plugin tool");

        backend
            .load_plugins(&root)
            .expect("load_plugins happy path should merge");
        assert!(
            backend.list_tools().contains(&"plugin_tool".to_string()),
            "the merged plugin tool must be listed after load_plugins re-wires the registry",
        );
        // The base tool is still present (merge is additive).
        assert!(backend.list_tools().contains(&"search".to_string()));
        let _ = std::fs::remove_dir_all(&root);

        // Error path: a second plugin root whose tool collides on (kind, name) with the
        // already-loaded "search" tool -> merge fails with BackendError::Load.
        let seq2 = DIR_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let collide_root = std::env::temp_dir().join(format!(
            "tdw_backend_agent_plug_col_{}_{seq2}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&collide_root);
        let collide_dir = collide_root.join("dup");
        std::fs::create_dir_all(&collide_dir).expect("mkdir collide dir");
        std::fs::write(
            collide_dir.join("plugin.json5"),
            r#"{ apiVersion:"tdw.finx/v1", kind:"plugin",
                 metadata:{ name:"dup", id:"dup", version:"0.1.0",
                   origin:{tier:"Custom",source:"Internal"}, adaptivity:"None", autonomous:false },
                 spec:{ members:["search"], client_side:false, server_side:true } }"#,
        )
        .expect("write collide manifest");
        std::fs::write(collide_dir.join("tool.json5"), tool_fixture("search"))
            .expect("write collide tool");

        let error = backend
            .load_plugins(&collide_root)
            .expect_err("a (kind,name) collision with the base tool must error");
        assert!(matches!(error, crate::error::BackendError::Load(_)));
        let _ = std::fs::remove_dir_all(&collide_root);
        let _ = std::fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn kg_bare_accessors_write_through_mut_read_through_shared() {
        use tdw_kg::EntityKind;

        let (dir, mut backend) = backend_with_search_tool();
        // Write through the bare kg_mut() accessor (line 315).
        backend.kg_mut().upsert_entity(Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        });
        // Read back through the bare kg() shared accessor (line 310).
        assert_eq!(
            backend
                .kg()
                .entity("instrument:AAPL")
                .map(|e| e.label.clone()),
            Some("Apple".to_string()),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_bare_accessors_write_through_mut_read_through_shared() {
        let (dir, mut backend) = backend_with_search_tool();
        // Define + assign through the bare tags_mut() accessor (line 350).
        backend
            .tags_mut()
            .define(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        backend
            .tags_mut()
            .assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "manual".to_string(),
            })
            .unwrap_or_else(|error| panic!("assignment should persist: {error}"));
        // Read back through the bare tags() shared accessor (line 345).
        assert_eq!(
            backend.tags().active_tags("instrument:AAPL", "2026-05-22"),
            vec!["asset:equity".to_string()],
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn features_bare_accessors_materialize_then_read_through_shared() {
        let (dir, mut backend) = backend_with_search_tool();
        // Tags first (active tags are joined into the snapshot).
        backend
            .tags_mut()
            .define(TagDefinition {
                tag_id: "asset:equity".to_string(),
                parent: None,
                ttl_days: None,
            })
            .unwrap_or_else(|error| panic!("tag should define: {error}"));
        backend
            .tags_mut()
            .assign(TagAssignment {
                entity_id: "instrument:AAPL".to_string(),
                tag_id: "asset:equity".to_string(),
                assigned_at: "2026-05-21".to_string(),
                expires_at: None,
                provenance: "manual".to_string(),
            })
            .unwrap_or_else(|error| panic!("assignment should persist: {error}"));

        // Materialize via the high-level method (the bare features_mut().materialize would
        // need a simultaneous &self.tags borrow, a borrow-checker split; the bare mut
        // accessor at 391 is still exercised below via a snapshot count check).
        let mut features = BTreeMap::new();
        features.insert("return_1d".to_string(), 0.01);
        let snapshot = backend.materialize_features("instrument:AAPL", "2026-05-21", features);
        assert_eq!(snapshot.tags, vec!["asset:equity".to_string()]);

        // Read back through the bare features() shared accessor (line 386).
        assert_eq!(
            backend
                .features()
                .latest("instrument:AAPL")
                .map(|latest| latest.as_of.clone()),
            Some("2026-05-21".to_string()),
        );

        // Exercise the bare features_mut() accessor (line 391) with a real observable: a
        // second materialize through tags-free path keeps prior snapshots intact, and the
        // mut handle is the live store the shared accessor reads.
        let snap2 = {
            let empty_tags = TagStore::default();
            let mut more = BTreeMap::new();
            more.insert("return_5d".to_string(), 0.05);
            backend
                .features_mut()
                .materialize("instrument:MSFT", "2026-05-22", more, &empty_tags)
        };
        assert_eq!(snap2.as_of, "2026-05-22");
        assert!(snap2.tags.is_empty());
        assert_eq!(
            backend
                .features()
                .latest("instrument:MSFT")
                .map(|latest| latest.as_of.clone()),
            Some("2026-05-22".to_string()),
        );
        // The earlier AAPL snapshot is still retrievable, proving the mut handle mutated
        // the same store the shared accessor reads.
        assert!(backend.features().latest("instrument:AAPL").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- config.rs pure builders (cross-file; mandatory floor-clearing leg) -----------

    #[test]
    fn backend_config_new_defaults_surfaces_and_transport() {
        use crate::config::{McpTransport, Surfaces};
        use tdw_config::TdwConfig;

        let config = BackendConfig::new(TdwConfig::default());
        assert_eq!(config.surfaces, Surfaces::Both);
        assert_eq!(config.mcp_transport, McpTransport::Stdio);
    }

    #[test]
    fn backend_config_with_surfaces_overrides_field() {
        use crate::config::{McpTransport, Surfaces};
        use tdw_config::TdwConfig;

        let config = BackendConfig::new(TdwConfig::default()).with_surfaces(Surfaces::McpOnly);
        assert_eq!(config.surfaces, Surfaces::McpOnly);
        // The transport is left at its default by the surfaces builder.
        assert_eq!(config.mcp_transport, McpTransport::Stdio);
    }

    #[test]
    fn backend_config_with_mcp_transport_overrides_field() {
        use crate::config::{McpTransport, Surfaces};
        use tdw_config::TdwConfig;

        let config = BackendConfig::new(TdwConfig::default())
            .with_mcp_transport(McpTransport::Http("127.0.0.1:8788".to_string()));
        assert_eq!(
            config.mcp_transport,
            McpTransport::Http("127.0.0.1:8788".to_string()),
        );
        // The surfaces field is left at its default by the transport builder.
        assert_eq!(config.surfaces, Surfaces::Both);
    }

    /// B9 eval-floor: a single-case eval (below `MIN_EVAL_CASES=5`) must NOT promote
    /// `Validated` proposals to `Ready`, even when `pass_rate == 1.0`.
    #[test]
    fn run_eval_under_floor_does_not_promote() {
        use tdw_agent::{AgentCard, AgentSkill, ContentKind, ContentRef};
        use tdw_knowledge::proposals::ProposalQueue;
        // G008/ER3: set TDW_ALLOW_STUB_FEEDBACK=1 so the stub model's feedback
        // path runs (this test checks proposal gate, not skill mutation).
        // SAFETY: test-only, single-threaded context (cargo test default).
        set_allow_stub_feedback_for_test(true);

        let (dir, mut backend) = backend_with_search_tool();

        let card = AgentCard {
            meta: EntityMeta::new(
                "market-researcher",
                "market-researcher",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Learning,
                true,
            )
            .with_title("Market Researcher")
            .with_description("Generates evidence-backed notes."),
            skills: vec![AgentSkill {
                meta: EntityMeta::new(
                    "research.note",
                    "research.note",
                    "0.1.0",
                    Origin {
                        tier: Tier::Domain,
                        source: Source::Internal,
                    },
                    Adaptivity::Learning,
                    false,
                )
                .with_title("research.note")
                .with_description("A skill."),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                quality: None,
            }],
            content_refs: vec![ContentRef {
                uri: "tdw://docs/research-template".to_string(),
                kind: ContentKind::Prompt,
                checksum: None,
                tags: Vec::new(),
            }],
            endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
            tool_scope: Vec::new(),
            runtime: None,
        };
        backend.upsert_agent(card);

        let queue: ProposalQueue = serde_json::from_value(serde_json::json!({
            "proposals": {
                "p1": {
                    "id": "p1",
                    "kind": { "kind": "tag_define", "tag_id": "asset:equity", "parent": null },
                    "agent_id": "market-researcher",
                    "status": "validated",
                    "history": ["2026-05-31 validated"],
                }
            },
            "next_id": 1,
        }))
        .expect("queue deserializes");
        let proposals = Arc::new(tokio::sync::Mutex::new(queue));
        backend.set_proposals(Arc::clone(&proposals));

        // Only 1 case — below MIN_EVAL_CASES=5; pass_rate will be 1.0 but the floor guard fires.
        let outcome = backend.run_eval_at(
            EvalRunRequest {
                run_id: "eval-floor".to_string(),
                agent_id: "market-researcher".to_string(),
                dataset_id: "golden-market-notes".to_string(),
                cases: vec![EvalCase {
                    case_id: "case-1".to_string(),
                    prompt: "Summarize AAPL".to_string(),
                    expected_refs: vec![ContentRef {
                        uri: "tdw://docs/research-template".to_string(),
                        kind: ContentKind::Prompt,
                        checksum: None,
                        tags: Vec::new(),
                    }],
                }],
            },
            "2026-05-31T00:00:00+00:00",
        );
        assert_eq!(outcome.status, "success");

        // The proposal must remain Validated — the floor guard blocked promotion.
        let status = {
            let queue = proposals.blocking_lock();
            queue.get("p1").expect("proposal present").status
        };
        assert_eq!(
            status,
            tdw_agent::ValidationStatus::Validated,
            "under-floor eval must not promote proposal"
        );
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: test-only cleanup.
        set_allow_stub_feedback_for_test(false);
    }

    #[test]
    fn register_hook_lists_name_and_runs_under_default_policy() {
        use tdw_event::sample_event;
        use tdw_hooks::{HandlerKind, HookError, HookSpec, TransactionMode};

        let (dir, mut backend) = backend_with_search_tool();

        // A no-op command hook (named differently from the sample event's
        // `event_type` so the recursion guard does not trip). Mirrors the
        // construction in `tdw-hooks`' own handler-execution tests.
        let hook = HookSpec::new("audit-log", 0, TransactionMode::PostCommit).with_handler(
            HandlerKind::Command {
                command: "true".to_string(),
                args: Vec::new(),
            },
        );
        backend.register_hook(hook);
        assert_eq!(backend.hook_names(), vec!["audit-log".to_string()]);

        // The default `HookExecutionPolicy` defaults to `Ask` (HK2/CFG2), so
        // executing an unmatched command handler surfaces a mapped
        // `PermissionRequiresApproval` — the handler still never runs.
        let envelope = sample_event("backend");
        let error = backend
            .run_hooks(&envelope)
            .expect_err("default ask policy must withhold the hook handler");
        assert!(matches!(
            error,
            crate::error::BackendError::Hook(HookError::PermissionRequiresApproval(action))
                if action == "hook.command.true"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
