//! The entity-kind registry: the 52 classified kinds and their manifest groups.
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
    /// Warehouse domain entities (instruments, accounts, datasets, …).
    Domain,
}

/// One of the platform's classified entity kinds.
///
/// NOTE: variant declaration order is load-bearing — the derived `Ord` follows declaration
/// order, which drives both [`EntityKind::ALL`] ordering and the deterministic `Registry`
/// iteration order. Do not reorder variants casually. The `domain` group is appended after
/// `meta` (rather than sorted into place) precisely so the pre-existing 43 kinds keep
/// their relative order.
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
    // domain (warehouse entities — unified taxonomy, knowledge-system overhaul A1)
    Instrument,
    Account,
    Strategy,
    Dataset,
    Provider,
    Symbol,
    Venue,
    /// A taxonomy tag, stored as a first-class graph node from A5 on (children
    /// via `subtag_of` edges, assignments via temporal `tagged` edges).
    Tag,
    /// A first-class analyst finding (knowledge-system K-X6): a user-authored
    /// research note with optional evidence pinning and typed links to other
    /// entities or findings. Findings belong to the `Knowledge` group and are
    /// retrievable via hybrid search. They carry user provenance
    /// (`Provenance::Agent { gated: false }`) and are trust-dial-filterable.
    ///
    /// # Inference boundary (F4)
    ///
    /// * **`PropagateTag` rules CAN reach findings** — tag propagation walks
    ///   existing graph edges *from* tag-holders outward; it does not consume
    ///   a finding's own edges as chain-join inputs.  Auto-tagging of findings
    ///   is therefore fine and aids retrieval.
    ///
    /// * **`DeriveEdge` rules do NOT consume finding edges by default** —
    ///   [`tdw_infer::InferEngine`] sets `exclude_user_authored = true` so
    ///   edges with `Provenance::Agent { gated: false }` are excluded from
    ///   chain matching.  An operator may opt in via
    ///   [`InferEngine::with_user_authored_inference(true)`].  This default
    ///   prevents user findings from silently minting derived facts the
    ///   operator never sanctioned.
    Finding,
    /// A mined subgraph pattern (knowledge-system K-R4): a frequent motif
    /// shape discovered by the deterministic motif-mining pass.  Pattern nodes
    /// are persisted by the `tdw-patterns` engine with
    /// `Provenance::System { detail: "pattern-mining:v0" }`.  They carry
    /// their canonical motif string, support count, temporal window, and
    /// capped instance provenance edges (`pattern_instance_of`).
    ///
    /// Pattern nodes belong to the `Knowledge` group and are why-explainable
    /// via `tdw.kg.why` (the chain composer walks `pattern_instance_of` edges
    /// to the supporting instances).
    ///
    /// # Mining posture
    ///
    /// Mining is disabled by default (`[knowledge.patterns] enabled = false`).
    /// The operator must flip the flag before the cron trigger fires.  A loud
    /// status note is emitted at every tick when disabled — this is honest,
    /// not silent.
    Pattern,
}

impl EntityKind {
    /// Every classified kind, in manifest-group order (domain appended last; see the
    /// declaration-order note on the enum).
    pub const ALL: [Self; 53] = [
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
        Self::Instrument,
        Self::Account,
        Self::Strategy,
        Self::Dataset,
        Self::Provider,
        Self::Symbol,
        Self::Venue,
        Self::Tag,
        // Finding appended after Tag so existing ordinals stay stable (K-X6).
        Self::Finding,
        // Pattern appended after Finding so existing ordinals stay stable (K-R4).
        Self::Pattern,
    ];

    /// The manifest group this kind belongs to.
    #[must_use]
    pub const fn group(self) -> Group {
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
            | Self::FeatureList
            // Finding is retrievable user-authored research (K-X6).
            | Self::Finding
            // Pattern is a mined subgraph shape (K-R4).
            | Self::Pattern => Group::Knowledge,
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
            Self::Instrument
            | Self::Account
            | Self::Strategy
            | Self::Dataset
            | Self::Provider
            | Self::Symbol
            | Self::Venue
            | Self::Tag => Group::Domain,
        }
    }

    /// Whether this is a data/content kind that carries the data facets
    /// (`plane` / `materialization` / `as_of` / `validation`).
    #[must_use]
    pub const fn is_data_kind(self) -> bool {
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
                // Finding carries content facets (as_of, plane) — K-X6.
                | Self::Finding
                // Pattern carries mined-shape content facets (canonical, support, window) — K-R4.
                | Self::Pattern
        )
    }

    /// Whether this kind can be an autonomous actor. Only `agent` is autonomy-capable —
    /// the cleanest validation that autonomy is an agent property, not an Origin axis.
    #[must_use]
    pub const fn is_autonomy_capable(self) -> bool {
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
        assert_eq!(seen.len(), 53);
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
        assert_eq!(count(Group::Knowledge), 10); // +Finding (K-X6) +Pattern (K-R4)
        assert_eq!(count(Group::Governance), 6);
        assert_eq!(count(Group::Infra), 5);
        assert_eq!(count(Group::Meta), 1);
        assert_eq!(count(Group::Domain), 8);
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
        check(EntityKind::Instrument, "instrument");
        check(EntityKind::Venue, "venue");
        check(EntityKind::Tag, "tag");
        check(EntityKind::Finding, "finding"); // K-X6
        check(EntityKind::Pattern, "pattern"); // K-R4
    }

    #[test]
    fn only_agent_is_autonomy_capable() {
        let capable: Vec<_> = EntityKind::ALL
            .into_iter()
            .filter(|kind| kind.is_autonomy_capable())
            .collect();
        assert_eq!(capable, vec![EntityKind::Agent]);
    }

    #[test]
    fn is_data_kind_matches_the_knowledge_group() {
        // `is_data_kind` and the `Group::Knowledge` arm enumerate the same kinds
        // as two independent lists. Pin their equivalence for every kind so a
        // silent divergence is caught — and any *intended* future divergence
        // becomes a deliberate, visible change to this assertion.
        for kind in EntityKind::ALL {
            assert_eq!(
                kind.is_data_kind(),
                kind.group() == Group::Knowledge,
                "is_data_kind disagrees with Group::Knowledge for {kind:?}"
            );
        }
    }

    #[test]
    fn domain_kinds_keep_pre_existing_ordinals_stable() {
        // The domain group is appended AFTER meta so the 43 pre-existing kinds keep
        // their relative declaration order (Ord is declaration-order-derived and
        // load-bearing for ALL ordering and Registry iteration).
        assert!(EntityKind::ResourceDefinition < EntityKind::Instrument);
        assert_eq!(EntityKind::ALL[42], EntityKind::ResourceDefinition);
        assert_eq!(EntityKind::ALL[43], EntityKind::Instrument);
    }

    #[test]
    fn finding_kind_is_in_knowledge_group_at_expected_ordinal() {
        // Finding is appended at index 51 (after Tag at 50) so existing 51 kinds
        // keep their ordinals. It is data-kind and Knowledge-group (K-X6).
        assert_eq!(EntityKind::ALL[51], EntityKind::Finding);
        assert_eq!(EntityKind::Finding.group(), Group::Knowledge);
        assert!(EntityKind::Finding.is_data_kind());
    }

    #[test]
    fn pattern_kind_is_in_knowledge_group_at_expected_ordinal() {
        // Pattern is appended at index 52 (after Finding at 51) so the 52 pre-existing
        // kinds keep their ordinals stable (K-R4).
        assert_eq!(EntityKind::ALL[52], EntityKind::Pattern);
        assert_eq!(EntityKind::Pattern.group(), Group::Knowledge);
        assert!(EntityKind::Pattern.is_data_kind());
    }
}
