//! Convenience re-exports. `use tdw_backend::prelude::*;` brings the facade
//! surface available so far into scope. Later phases extend this.

pub use crate::agent::AgentBackend;
pub use crate::config::{BackendConfig, Surfaces};
pub use crate::data::Backend;
pub use crate::error::{BackendError, BackendResult};

pub use tdw_config::TdwConfig;
pub use tdw_service_api::AppState;

// Phase 1 — the data facade's typed surface.
pub use tdw_core::{
    BlobEngine, Credentials, LexicalEngine, OBBject, OlapEngine, ProgressStream, ProviderRegistry,
    RelationalEngine, VectorEngine,
};
pub use tdw_domain::EquityHistoricalData;
pub use tdw_protocol::{EventMsg, Op, OpEnvelope};

// Phase 2 — the sync agent/MCP facade's typed surface.
pub use tdw_agent::{
    AgentCard, EvalRunRequest, Registry, RegistryWatcher, Tool, ToolImplementation,
    WorkflowDefinition,
};
pub use tdw_agent_store::AgentStore;
pub use tdw_eval_runner::{EvalRunOutcome, EvalRunner};
pub use tdw_mcp::McpServer;
pub use tdw_tool_exec::{CommandPolicy, ExecError, ToolOutcome};
pub use tdw_workflow_engine::{ExecutionPlan, WorkflowEngine};
