//! Synchronous agent/MCP facade.
//!
//! Phase 0 wires construction only: it composes the `tdw-agent` [`Registry`], the
//! [`ToolExecutor`], an [`McpServer`] (with the registry + executor attached), and
//! an [`AgentStore`]. Later phases add the execution methods on top.

use std::path::Path;

use tdw_agent::Registry;
use tdw_agent_store::AgentStore;
use tdw_mcp::McpServer;
use tdw_tool_exec::{CommandPolicy, ToolExecutor};

use crate::config::BackendConfig;
use crate::error::BackendResult;

/// Environment variable naming the directory of `tdw-agent` registry definitions
/// to load. Shared with `tdw-mcp`'s server entrypoints so the embedded and
/// stand-alone MCP surfaces resolve the same registry.
pub const REGISTRY_DIR_ENV: &str = tdw_mcp::REGISTRY_DIR_ENV;

/// The synchronous agent/MCP facade.
pub struct AgentBackend {
    #[allow(dead_code)] // surfaced via execution methods in a later phase.
    registry: Registry,
    #[allow(dead_code)]
    executor: ToolExecutor,
    #[allow(dead_code)]
    mcp: McpServer,
    #[allow(dead_code)]
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
}

/// Read the registry directory from [`REGISTRY_DIR_ENV`], treating an unset or
/// blank value as "not configured".
fn registry_dir_from_env() -> Option<String> {
    std::env::var(REGISTRY_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
