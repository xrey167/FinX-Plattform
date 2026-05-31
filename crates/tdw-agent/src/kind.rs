//! The entity-kind registry: the 43 classified kinds and their manifest groups.
//!
//! Serialized form matches the registry's lowercase token convention (e.g. `agentrouter`,
//! `knowledgegraph`, `resourcedefinition`). `storage_mapping` is intentionally absent — it
//! is a persistence facet + relation, not a kind.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Manifest group an [`EntityKind`] belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    /// Core primitives.
    Core,
    /// Tool integrations.
    Tools,
    /// Orchestration constructs.
    Orchestration,
    /// Knowledge & data kinds.
    Knowledge,
    /// Governance & policy kinds.
    Governance,
    /// Infrastructure kinds.
    Infra,
    /// Meta-schema.
    Meta,
}

/// One of the platform's classified entity kinds.
///
/// NOTE: variant declaration order is load-bearing — the derived `Ord` follows declaration
/// order, which drives both [`EntityKind::ALL`] ordering and the deterministic `Registry`
/// iteration order. Do not reorder variants casually.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    // core
    Agent,
    Personality,
    Prompt,
    PromptTemplate,
    Template,
    Instruction,
    Context,
    Config,
    Primitive,
    EnvironmentVariable,
    // tools
    Tool,
    Function,
    McpServer,
    McpTool,
    Connector,
    Webhook,
    // orchestration
    Workflow,
    Task,
    Skill,
    Command,
    Hook,
    AgentRouter,
    ToolRouter,
    // knowledge
    Knowledge,
    Document,
    RagPipeline,
    KnowledgeGraph,
    Memory,
    FeatureStore,
    Feature,
    FeatureList,
    // governance
    Guardrail,
    Rule,
    Evaluation,
    Plugin,
    ErrorPolicy,
    Gotcha,
    // infra
    Network,
    Compute,
    DataStore,
    SecretStore,
    Observability,
    // meta
    ResourceDefinition,
}

impl EntityKind {
    /// Every classified kind, in manifest-group order.
    pub const ALL: [EntityKind; 43] = [
        EntityKind::Agent,
        EntityKind::Personality,
        EntityKind::Prompt,
        EntityKind::PromptTemplate,
        EntityKind::Template,
        EntityKind::Instruction,
        EntityKind::Context,
        EntityKind::Config,
        EntityKind::Primitive,
        EntityKind::EnvironmentVariable,
        EntityKind::Tool,
        EntityKind::Function,
        EntityKind::McpServer,
        EntityKind::McpTool,
        EntityKind::Connector,
        EntityKind::Webhook,
        EntityKind::Workflow,
        EntityKind::Task,
        EntityKind::Skill,
        EntityKind::Command,
        EntityKind::Hook,
        EntityKind::AgentRouter,
        EntityKind::ToolRouter,
        EntityKind::Knowledge,
        EntityKind::Document,
        EntityKind::RagPipeline,
        EntityKind::KnowledgeGraph,
        EntityKind::Memory,
        EntityKind::FeatureStore,
        EntityKind::Feature,
        EntityKind::FeatureList,
        EntityKind::Guardrail,
        EntityKind::Rule,
        EntityKind::Evaluation,
        EntityKind::Plugin,
        EntityKind::ErrorPolicy,
        EntityKind::Gotcha,
        EntityKind::Network,
        EntityKind::Compute,
        EntityKind::DataStore,
        EntityKind::SecretStore,
        EntityKind::Observability,
        EntityKind::ResourceDefinition,
    ];

    /// The manifest group this kind belongs to.
    #[must_use]
    pub fn group(self) -> Group {
        match self {
            EntityKind::Agent
            | EntityKind::Personality
            | EntityKind::Prompt
            | EntityKind::PromptTemplate
            | EntityKind::Template
            | EntityKind::Instruction
            | EntityKind::Context
            | EntityKind::Config
            | EntityKind::Primitive
            | EntityKind::EnvironmentVariable => Group::Core,
            EntityKind::Tool
            | EntityKind::Function
            | EntityKind::McpServer
            | EntityKind::McpTool
            | EntityKind::Connector
            | EntityKind::Webhook => Group::Tools,
            EntityKind::Workflow
            | EntityKind::Task
            | EntityKind::Skill
            | EntityKind::Command
            | EntityKind::Hook
            | EntityKind::AgentRouter
            | EntityKind::ToolRouter => Group::Orchestration,
            EntityKind::Knowledge
            | EntityKind::Document
            | EntityKind::RagPipeline
            | EntityKind::KnowledgeGraph
            | EntityKind::Memory
            | EntityKind::FeatureStore
            | EntityKind::Feature
            | EntityKind::FeatureList => Group::Knowledge,
            EntityKind::Guardrail
            | EntityKind::Rule
            | EntityKind::Evaluation
            | EntityKind::Plugin
            | EntityKind::ErrorPolicy
            | EntityKind::Gotcha => Group::Governance,
            EntityKind::Network
            | EntityKind::Compute
            | EntityKind::DataStore
            | EntityKind::SecretStore
            | EntityKind::Observability => Group::Infra,
            EntityKind::ResourceDefinition => Group::Meta,
        }
    }

    /// Whether this is a data/content kind that carries the data facets
    /// (`plane` / `materialization` / `as_of` / `validation`).
    #[must_use]
    pub fn is_data_kind(self) -> bool {
        matches!(
            self,
            EntityKind::Knowledge
                | EntityKind::Document
                | EntityKind::RagPipeline
                | EntityKind::KnowledgeGraph
                | EntityKind::Memory
                | EntityKind::FeatureStore
                | EntityKind::Feature
                | EntityKind::FeatureList
        )
    }

    /// Whether this kind can be an autonomous actor. Only `agent` is autonomy-capable —
    /// the cleanest validation that autonomy is an agent property, not an Origin axis.
    #[must_use]
    pub fn is_autonomy_capable(self) -> bool {
        matches!(self, EntityKind::Agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_kinds_are_unique_and_counted() {
        let mut seen = std::collections::HashSet::new();
        for kind in EntityKind::ALL {
            assert!(seen.insert(kind), "duplicate kind: {kind:?}");
        }
        assert_eq!(seen.len(), 43);
    }

    #[test]
    fn group_counts_match_the_decision_table() {
        let count = |group: Group| {
            EntityKind::ALL
                .into_iter()
                .filter(|kind| kind.group() == group)
                .count()
        };
        assert_eq!(count(Group::Core), 10);
        assert_eq!(count(Group::Tools), 6);
        assert_eq!(count(Group::Orchestration), 7);
        assert_eq!(count(Group::Knowledge), 8);
        assert_eq!(count(Group::Governance), 6);
        assert_eq!(count(Group::Infra), 5);
        assert_eq!(count(Group::Meta), 1);
    }

    #[test]
    fn serializes_to_registry_lowercase_tokens() {
        let check = |kind: EntityKind, token: &str| {
            assert_eq!(
                serde_json::to_value(kind).expect("kind should serialize"),
                serde_json::Value::String(token.to_string())
            );
            let decoded: EntityKind =
                serde_json::from_value(serde_json::json!(token)).expect("token should deserialize");
            assert_eq!(decoded, kind);
        };
        check(EntityKind::AgentRouter, "agentrouter");
        check(EntityKind::KnowledgeGraph, "knowledgegraph");
        check(EntityKind::ResourceDefinition, "resourcedefinition");
        check(EntityKind::McpServer, "mcpserver");
        check(EntityKind::EnvironmentVariable, "environmentvariable");
    }

    #[test]
    fn only_agent_is_autonomy_capable() {
        let capable: Vec<_> = EntityKind::ALL
            .into_iter()
            .filter(|kind| kind.is_autonomy_capable())
            .collect();
        assert_eq!(capable, vec![EntityKind::Agent]);
    }
}
