//! Synchronous agent/MCP facade.
//!
//! Composes the `tdw-agent` [`Registry`], the [`ToolExecutor`], an [`McpServer`] (with the
//! registry + executor attached), and an [`AgentStore`]. Phase 2 adds the sync execution
//! surface on top: registry hot-reload + plugin loading (re-wiring the MCP cache), tool
//! listing/dispatch, single-line MCP JSON-RPC handling, workflow compilation, eval runs, and
//! agent/workflow CRUD passthrough. Pure composition / delegation — no business logic.

use std::path::Path;

use std::collections::BTreeMap;

use tdw_agent::{AgentCard, EntityKind, EvalRunRequest, Registry, WorkflowDefinition};
use tdw_agent_store::AgentStore;
use tdw_app_client::DaemonClientConfig;
use tdw_eval_runner::{EvalRunOutcome, EvalRunner};
use tdw_event::EventEnvelope;
use tdw_feature_store::{FeatureSnapshot, FeatureStore};
use tdw_hooks::{
    HookExecutionOutcome, HookExecutionPolicy, HookRegistry, HookSpec, SystemHookHandlerBackend,
};
use tdw_kg::{Entity, KnowledgeGraph, Relationship};
use tdw_mcp::McpServer;
use tdw_tags::{TagAssignment, TagDefinition, TagStore};
use tdw_tool_exec::{CommandPolicy, ToolExecutor, ToolOutcome};
use tdw_workflow_engine::{ExecutionPlan, WorkflowEngine};

use crate::config::BackendConfig;
use crate::error::BackendResult;

/// Environment variable naming the directory of `tdw-agent` registry definitions
/// to load. Shared with `tdw-mcp`'s server entrypoints so the embedded and
/// stand-alone MCP surfaces resolve the same registry.
pub const REGISTRY_DIR_ENV: &str = tdw_mcp::REGISTRY_DIR_ENV;

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
        let mcp = McpServer::new()
            .with_registry(registry.clone())
            .with_executor(ToolExecutor::new().with_command_policy(policy));
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
        }
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
    pub fn mcp_server(&mut self) -> &mut McpServer {
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

    /// Run an evaluation request against the embedded [`AgentStore`], persisting the run and
    /// returning its outcome.
    pub fn run_eval(&mut self, request: EvalRunRequest) -> EvalRunOutcome {
        EvalRunner::run(request, &mut self.store)
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
    pub fn kg(&self) -> &KnowledgeGraph {
        &self.kg
    }

    /// Mutable access to the embedded [`KnowledgeGraph`].
    pub fn kg_mut(&mut self) -> &mut KnowledgeGraph {
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
    pub fn tags(&self) -> &TagStore {
        &self.tags
    }

    /// Mutable access to the embedded [`TagStore`].
    pub fn tags_mut(&mut self) -> &mut TagStore {
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
    pub fn features(&self) -> &FeatureStore {
        &self.features
    }

    /// Mutable access to the embedded [`FeatureStore`].
    pub fn features_mut(&mut self) -> &mut FeatureStore {
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
            std::env::temp_dir().join(format!("tdw_backend_agent_{}_{seq}", std::process::id(),));
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

        // The default `HookExecutionPolicy` denies by default, so executing the
        // handler surfaces a mapped `PermissionDenied` for the command action.
        let envelope = sample_event("backend");
        let error = backend
            .run_hooks(&envelope)
            .expect_err("default deny policy must reject the hook handler");
        assert!(matches!(
            error,
            crate::error::BackendError::Hook(HookError::PermissionDenied(action))
                if action == "hook.command.true"
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
