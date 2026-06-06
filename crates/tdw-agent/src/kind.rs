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
    pub const ALL: [Self; 43] = [
        Self::Agent,
        Self::Personality,
        Self::Prompt,
        Self::PromptTemplate,
        Self::Template,
        Self::Instruction,
        Self::Context,
        Self::Config,
        Self::Primitive,
        Self::EnvironmentVariable,
        Self::Tool,
        Self::Function,
        Self::McpServer,
        Self::McpTool,
        Self::Connector,
        Self::Webhook,
        Self::Workflow,
        Self::Task,
        Self::Skill,
        Self::Command,
        Self::Hook,
        Self::AgentRouter,
        Self::ToolRouter,
        Self::Knowledge,
        Self::Document,
        Self::RagPipeline,
        Self::KnowledgeGraph,
        Self::Memory,
        Self::FeatureStore,
        Self::Feature,
        Self::FeatureList,
        Self::Guardrail,
        Self::Rule,
        Self::Evaluation,
        Self::Plugin,
        Self::ErrorPolicy,
        Self::Gotcha,
        Self::Network,
        Self::Compute,
        Self::DataStore,
        Self::SecretStore,
        Self::Observability,
        Self::ResourceDefinition,
    ];

    /// The manifest group this kind belongs to.
    #[must_use]
    pub fn group(self) -> Group {
        match self {
            Self::Agent
            | Self::Personality
            | Self::Prompt
            | Self::PromptTemplate
            | Self::Template
            | Self::Instruction
            | Self::Context
            | Self::Config
            | Self::Primitive
            | Self::EnvironmentVariable => Group::Core,
            Self::Tool
            | Self::Function
            | Self::McpServer
            | Self::McpTool
            | Self::Connector
            | Self::Webhook => Group::Tools,
            Self::Workflow
            | Self::Task
            | Self::Skill
            | Self::Command
            | Self::Hook
            | Self::AgentRouter
            | Self::ToolRouter => Group::Orchestration,
            Self::Knowledge
            | Self::Document
            | Self::RagPipeline
            | Self::KnowledgeGraph
            | Self::Memory
            | Self::FeatureStore
            | Self::Feature
            | Self::FeatureList => Group::Knowledge,
            Self::Guardrail
            | Self::Rule
            | Self::Evaluation
            | Self::Plugin
            | Self::ErrorPolicy
            | Self::Gotcha => Group::Governance,
            Self::Network
            | Self::Compute
            | Self::DataStore
            | Self::SecretStore
            | Self::Observability => Group::Infra,
            Self::ResourceDefinition => Group::Meta,
        }
    }

    /// Whether this is a data/content kind that carries the data facets
    /// (`plane` / `materialization` / `as_of` / `validation`).
    #[must_use]
    pub fn is_data_kind(self) -> bool {
        matches!(
            self,
            Self::Knowledge
                | Self::Document
                | Self::RagPipeline
                | Self::KnowledgeGraph
                | Self::Memory
                | Self::FeatureStore
                | Self::Feature
                | Self::FeatureList
        )
    }

    /// Whether this kind can be an autonomous actor. Only `agent` is autonomy-capable —
    /// the cleanest validation that autonomy is an agent property, not an Origin axis.
    #[must_use]
    pub fn is_autonomy_capable(self) -> bool {
        matches!(self, Self::Agent)
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
