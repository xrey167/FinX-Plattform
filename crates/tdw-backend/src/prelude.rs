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

// Phase B — the agent memory consolidation loop's typed surface: the `Memory`
// entity + its `Retention` tier, the `ConsolidationAction` the loop applies, and
// the `MemoryStore` the daemon persists tier changes into.
pub use tdw_agent::{ConsolidationAction, Memory, Retention};
pub use tdw_agent_store::MemoryStore;
pub use tdw_eval_runner::{EvalRunOutcome, EvalRunner};
pub use tdw_mcp::McpServer;
pub use tdw_tool_exec::{CommandPolicy, ExecError, ToolOutcome};
pub use tdw_workflow_engine::{ExecutionPlan, WorkflowEngine};

// Phase 5/6 — the loopback daemon client the embedded MCP surface uses to reach
// the in-process daemon at its bound address (see
// [`AgentBackend::with_daemon_addr`](crate::agent::AgentBackend::with_daemon_addr)).
pub use tdw_app_client::{DaemonClient, DaemonClientConfig};

// Phase 3 — the knowledge group's typed surface (sync KG/tags/features + the
// async knowledge index).
pub use tdw_feature_store::{FeatureSnapshot, FeatureStore};
pub use tdw_kg::{Entity, KnowledgeGraph, Relationship};
pub use tdw_knowledge::{KnowledgeDocument, KnowledgeHit, KnowledgeIndex};
pub use tdw_tags::{TagAssignment, TagDefinition, TagStore};

// Phase 4 — the auth/hooks/policy/events group's typed surface. `auth` is the
// single source for this group's re-exports; the prelude pulls it in wholesale
// so the two surfaces can never drift.
pub use crate::auth::*;
