//! Synchronous agent/MCP facade.
//!
//! Composes the `tdw-agent` [`Registry`], the [`ToolExecutor`], an [`McpServer`] (with the
//! registry + executor attached), and an [`AgentStore`]. Phase 2 adds the sync execution
//! surface on top: registry hot-reload + plugin loading (re-wiring the MCP cache), tool
//! listing/dispatch, single-line MCP JSON-RPC handling, workflow compilation, eval runs, and
//! agent/workflow CRUD passthrough. Pure composition / delegation — no business logic.

use std::path::Path;

use tdw_agent::{AgentCard, EntityKind, EvalRunRequest, Registry, WorkflowDefinition};
use tdw_agent_store::AgentStore;
use tdw_eval_runner::{EvalRunOutcome, EvalRunner};
use tdw_mcp::McpServer;
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
        }
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
    pub fn call_tool(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> BackendResult<ToolOutcome> {
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
        let dir = std::env::temp_dir().join(format!(
            "tdw_backend_agent_{}_{seq}",
            std::process::id(),
        ));
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
}
