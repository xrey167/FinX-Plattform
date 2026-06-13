#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use validator::Validate;

pub mod base;
pub mod consolidate;
pub mod facets;
pub mod kind;
pub mod loader;
pub mod mcp;
pub mod registry;
pub mod resource;
pub mod watch;

pub use base::{
    Adaptivity, AdaptivityError, BaseMetadata, EntityMeta, FEEDBACK_MIN_ADAPTIVITY, Icon, Origin,
    Reference, Retention, Source, Tier, ToolEffect, ToolImplementation,
    ensure_adaptive_for_feedback,
};
pub use consolidate::{
    ConsolidationAction, UsageHint, consolidation_plan, consolidation_plan_with_usage,
};
pub use facets::{
    DataFacets, EvalFacets, Materialization, OpsMetrics, Plane, ValidationState, ValidationStatus,
};
pub use kind::{EntityKind, Group};
pub use loader::{LoadError, load_resource, load_typed};
pub use mcp::{
    IconMimeSupport, MCP_PROTOCOL_VERSION, McpEntity, McpPrompt, McpPromptArgument, McpTool,
    ParallelSafety, ToolAnnotations, icon_mime_support, project_to_mcp,
};
pub use registry::Registry;
pub use resource::{
    RegistryEntity, Resource, ResourceDefinition, TDW_API_GROUP, TDW_API_VERSION,
    entity_from_resource,
};
pub use watch::{RegistryWatcher, WatchError};

/// DEPRECATED (pre-taxonomy): the legacy fixed set of agent schema names.
///
/// Q8 reclassified storage to a `resourcedefinition` persistence facet; prefer
/// [`resource_definitions`]. Retained because downstream crates still consume it.
pub const AGENT_SCHEMA_NAMES: [&str; 9] = [
    "agent_card",
    "agent_skill",
    "slash_command",
    "slash_command_invocation",
    "content_ref",
    "eval_run_request",
    "workflow_definition",
    "gotcha",
    "storage_mapping",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ContentKind {
    Document,
    Dataset,
    Prompt,
    Workflow,
    Tool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum GotchaSeverity {
    Info,
    Warning,
    Blocker,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ContentRef {
    #[validate(length(min = 1))]
    pub uri: String,
    pub kind: ContentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Eval-driven quality state attached to a skill once it has received feedback.
///
/// Defaults to "never evaluated" (all-`None`/zero). Mutated only through
/// [`AgentSkill::apply_eval_feedback`], which is gated by the [`Adaptivity`] axis — a
/// `None`/`Configured` skill never accrues quality state. The field is additive and skipped
/// when absent, so existing golden fixtures serialize byte-identically.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct SkillQuality {
    /// How many eval runs have fed back into this skill.
    pub runs: u32,
    /// The most recent run's aggregate pass rate (`0.0..=1.0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_rate: Option<f64>,
    /// The score from the most recent eval (currently the same as `pass_rate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_eval_score: Option<f64>,
    /// Whether the latest run disabled this skill (pass rate fell below the threshold).
    pub disabled: bool,
    /// When the last eval feedback was applied (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_eval: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentSkill {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub input_schema: Value,
    pub output_schema: Value,
    /// Eval-driven quality state, present only after gated feedback has been applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(nested)]
    pub quality: Option<SkillQuality>,
}

impl AgentSkill {
    /// Apply one eval run's `pass_rate` to this skill's quality state, GATED by the
    /// [`Adaptivity`] axis: only a skill whose `meta.adaptivity >= Adaptivity::Learning`
    /// may accrue feedback. Increments the run count, records the pass rate / score /
    /// timestamp, and disables the skill when `pass_rate < disable_below`.
    ///
    /// `now` (RFC 3339) is injected so the mutation is deterministic; the live path passes
    /// `chrono::Utc::now().to_rfc3339()`.
    ///
    /// # Errors
    ///
    /// Returns [`AdaptivityError::NotLearning`] (leaving `quality` untouched) when the
    /// skill's adaptivity is below [`Adaptivity::Learning`].
    pub fn apply_eval_feedback(
        &mut self,
        pass_rate: f64,
        now: &str,
        disable_below: f64,
    ) -> Result<(), base::AdaptivityError> {
        base::ensure_adaptive_for_feedback(&self.meta)?;
        let quality = self.quality.get_or_insert_with(SkillQuality::default);
        quality.runs = quality.runs.saturating_add(1);
        quality.pass_rate = Some(pass_rate);
        quality.last_eval_score = Some(pass_rate);
        quality.last_eval = Some(now.to_string());
        quality.disabled = pass_rate < disable_below;
        Ok(())
    }
}

/// A canonical tool definition (the `tool` kind), MCP-agnostic. Projects to an MCP
/// `Tool` via `McpTool::from(&tool)` in the `mcp` adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Tool {
    #[validate(nested)]
    pub meta: EntityMeta,
    /// JSON Schema (2020-12) for inputs; the root must be an object.
    pub input_schema: Value,
    /// Optional JSON Schema (2020-12) for structured output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Effect on the world (drives execution scheduling).
    #[serde(default)]
    pub effect: ToolEffect,
    /// Whether repeated calls with the same arguments are safe.
    #[serde(default)]
    pub idempotent: bool,
    /// Whether the tool interacts with an open/external world.
    #[serde(default)]
    pub open_world: bool,
    /// How the tool is executed (defaults to [`ToolImplementation::Unbound`]).
    #[serde(default)]
    pub implementation: ToolImplementation,
}

/// A single argument of a [`Prompt`]. The optional `default` is an R1 [`Reference`] so it
/// can resolve at runtime from a variable/file/git/etc. (domain-only; not on the MCP wire).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct PromptArgument {
    /// Argument name.
    #[validate(length(min = 1))]
    pub name: String,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the argument is required.
    #[serde(default)]
    pub required: bool,
    /// Optional default value, resolved at runtime via R1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Reference>,
}

/// A canonical prompt definition (the `prompt` kind), MCP-agnostic. Projects to an MCP
/// `Prompt` via `McpPrompt::from(&prompt)` in the `mcp` adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Prompt {
    #[validate(nested)]
    pub meta: EntityMeta,
    /// Template body (e.g. minijinja).
    pub template: String,
    /// Declared arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[validate(nested)]
    pub arguments: Vec<PromptArgument>,
}

/// Optional per-agent runtime overrides for scoped delegation.
///
/// Every field is additive and `None` by default, so existing `*.json5` cards and golden
/// snapshots deserialize byte-identically. Holds only `Option<String>`/`Option<u32>`, so it
/// derives `Eq` like the sibling [`SlashArg`]; it carries no validation invariants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentRuntime {
    /// Preferred provider (e.g. an LLM vendor key); `None` inherits from the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Preferred model identifier; `None` inherits from the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Per-invocation timeout in seconds; `None` inherits from the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u32>,
    /// Maximum agent iterations; `None` inherits from the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentCard {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[validate(nested)]
    pub skills: Vec<AgentSkill>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[validate(nested)]
    pub content_refs: Vec<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Allowlist of tools this agent may use. Empty (the default) inherits nothing — an agent
    /// with no declared scope grants no tools and hands no tools to its delegates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_scope: Vec<String>,
    /// Optional runtime overrides (provider/model/timeout/iterations) for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<AgentRuntime>,
}

impl AgentCard {
    /// Returns `true` when `name` is present in this agent's [`tool_scope`](AgentCard::tool_scope)
    /// allowlist. An empty scope grants nothing.
    #[must_use]
    pub fn grants_tool(&self, name: &str) -> bool {
        self.tool_scope.iter().any(|tool| tool == name)
    }

    /// Computes the tool scope to hand to a delegate: the intersection of `requested` with this
    /// agent's own scope.
    ///
    /// The result is order-stable and deduplicated relative to `self.tool_scope`, and is always a
    /// subset of the parent scope — a delegate can never receive a tool the parent does not hold.
    #[must_use]
    pub fn child_scope(&self, requested: &[String]) -> Vec<String> {
        self.tool_scope
            .iter()
            .filter(|tool| requested.iter().any(|req| req == *tool))
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashArg {
    #[validate(length(min = 1))]
    pub name: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashCommand {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<SlashArg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashCommandInvocation {
    #[validate(length(min = 1))]
    pub command: String,
    pub args: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalCase {
    #[validate(length(min = 1))]
    pub case_id: String,
    #[validate(length(min = 1))]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_refs: Vec<ContentRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalRunRequest {
    #[validate(length(min = 1))]
    pub run_id: String,
    #[validate(length(min = 1))]
    pub agent_id: String,
    #[validate(length(min = 1))]
    pub dataset_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<EvalCase>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalMetric {
    #[validate(length(min = 1))]
    pub metric_name: String,
    pub metric_value: f64,
}

/// A canonical evaluation definition (the `evaluation` kind).
///
/// Captures what to evaluate plus the ML-rigor facets (track A). The runtime DTOs
/// (`EvalRunRequest`/`EvalCase`/`EvalMetric`) are separate execution-plane types and are
/// not registry entities.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Evaluation {
    #[validate(nested)]
    pub meta: EntityMeta,
    /// Dataset this evaluation runs against.
    pub dataset_id: String,
    /// ML-rigor facets (baselines, fixed splits, slices, drift, failure attribution, ops).
    #[serde(default)]
    #[validate(nested)]
    pub facets: EvalFacets,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct WorkflowNode {
    #[validate(length(min = 1))]
    pub node_id: String,
    #[validate(length(min = 1))]
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct WorkflowEdge {
    #[validate(length(min = 1))]
    pub from: String,
    #[validate(length(min = 1))]
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct WorkflowDefinition {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Gotcha {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub severity: GotchaSeverity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applies_to: Vec<String>,
    #[validate(length(min = 1))]
    pub remediation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<ContentRef>,
}

/// DEPRECATED (pre-taxonomy): an entity→table mapping.
///
/// Q8 reclassified storage to a `resourcedefinition` persistence facet + relation, so this
/// is no longer a first-class kind. Retained because downstream crates still consume it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StorageMapping {
    pub entity: String,
    pub schema: String,
    pub table: String,
    pub primary_key: Vec<String>,
}

impl RegistryEntity for AgentCard {
    const KIND: EntityKind = EntityKind::Agent;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for AgentSkill {
    const KIND: EntityKind = EntityKind::Skill;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Tool {
    const KIND: EntityKind = EntityKind::Tool;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Prompt {
    const KIND: EntityKind = EntityKind::Prompt;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Evaluation {
    const KIND: EntityKind = EntityKind::Evaluation;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for SlashCommand {
    const KIND: EntityKind = EntityKind::Command;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Gotcha {
    const KIND: EntityKind = EntityKind::Gotcha;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for WorkflowDefinition {
    const KIND: EntityKind = EntityKind::Workflow;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

// === core ===

/// A `personality` entity: trait/tone profile applied to an agent's behavior.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Personality {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<String>,
    pub system_directive: String,
}

/// A `prompttemplate` entity: a reusable template with declared variable names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct PromptTemplate {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub template: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<String>,
}

/// A `template` entity: a formatted body with an associated output format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Template {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub body: String,
    pub format: String,
}

/// An `instruction` entity: a directive with an ordering priority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Instruction {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub directive: String,
    /// Ordering priority; higher = applied first.
    pub priority: u16,
}

/// A `context` entity: contextual content with an optional source reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Context {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub content: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Reference>,
}

/// A `config` entity: named configuration values resolved via R1 references.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Config {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub values: BTreeMap<String, Reference>,
}

/// A `primitive` entity: a bare JSON Schema fragment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Primitive {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub json_schema: Value,
}

/// An `environmentvariable` entity: a key bound to an R1-resolved value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EnvironmentVariable {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[validate(length(min = 1))]
    pub key: String,
    pub value: Reference,
    #[serde(default)]
    pub secret: bool,
}

// === tools ===

/// A `function` entity: an in-process callable with input/output schemas.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Function {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub implementation: String,
}

/// An `mcpserver` entity: a remote MCP server endpoint and transport.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct McpServer {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub url: String,
    pub transport: String,
}

/// An `mcptool` entity: a reference to a named tool exposed by an MCP server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct McpToolRef {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub server: String,
    pub tool_name: String,
}

/// A `connector` entity: an integration handle with endpoint and optional auth.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Connector {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub kind: String,
    pub endpoint: Reference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<Reference>,
}

/// A `webhook` entity: a callback URL subscribed to a set of events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Webhook {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub url: Reference,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

// === orchestration ===

/// A `task` entity: an action with declared dependencies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Task {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// A `hook` entity: an event-triggered handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Hook {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub event: String,
    pub handler: String,
}

/// An `agentrouter` entity: routes requests across agents with an optional default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentRouter {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
}

/// A `toolrouter` entity: routes calls across tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ToolRouter {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<String>,
}

// === knowledge (embed DataFacets: derive PartialEq, not Eq) ===

/// A `knowledge` entity: a knowledge body carrying data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Knowledge {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub body: String,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `document` entity: a referenced document carrying data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Document {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub uri: Reference,
    pub content_type: String,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `ragpipeline` entity: an ordered set of retrieval stages with data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct RagPipeline {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<String>,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `knowledgegraph` entity: a graph definition with declared node kinds and data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct KnowledgeGraph {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_kinds: Vec<String>,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `memory` entity: a retention policy over stored content with data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Memory {
    #[validate(nested)]
    pub meta: EntityMeta,
    /// Retention tier (working → core); drives TTL and consolidation.
    #[serde(default)]
    pub retention: Retention,
    /// When this memory was last consolidated (RFC 3339), if ever — the breadcrumb a
    /// background consolidator updates as it promotes/rewrites entries between tiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_consolidated: Option<String>,
    /// Ids of the source memories this entry was consolidated from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_entries: Vec<String>,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `featurestore` entity: a store of features carrying data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FeatureStore {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `feature` entity: a single typed feature carrying data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Feature {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub value_type: String,
    #[validate(nested)]
    pub facets: DataFacets,
}

/// A `featurelist` entity: a named list of features carrying data facets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FeatureList {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[validate(nested)]
    pub facets: DataFacets,
}

// === governance ===

/// A `guardrail` entity: a policy directive constraining behavior.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Guardrail {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub directive: String,
}

/// A `rule` entity: a boolean expression evaluated against context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Rule {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub expression: String,
}

/// A `plugin` entity: a bundle of members with client/server execution flags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Plugin {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    #[serde(default)]
    pub client_side: bool,
    #[serde(default)]
    pub server_side: bool,
}

/// Backoff strategy applied between retries under an [`ErrorPolicy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum Backoff {
    /// A constant delay between retries.
    Fixed,
    /// An exponentially growing delay between retries.
    #[default]
    Exponential,
    /// A linearly growing delay between retries.
    Linear,
}

/// An `errorpolicy` entity: retry/backoff behavior on error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ErrorPolicy {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub on_error: String,
    pub max_retries: u32,
    #[serde(default)]
    pub backoff: Backoff,
}

// === infra ===

/// A `network` entity: a set of reachable endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Network {
    #[validate(nested)]
    pub meta: EntityMeta,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<String>,
}

/// A `compute` entity: a runtime with optional resource sizing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Compute {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<String>,
}

/// A `datastore` entity: a storage engine addressed by a DSN reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct DataStore {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub engine: String,
    pub dsn: Reference,
}

/// A `secretstore` entity: a secret-management provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SecretStore {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub provider: String,
}

/// An `observability` entity: a telemetry exporter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Observability {
    #[validate(nested)]
    pub meta: EntityMeta,
    pub exporter: String,
}

// === RegistryEntity impls for the 34 spec types ===

impl RegistryEntity for Personality {
    const KIND: EntityKind = EntityKind::Personality;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for PromptTemplate {
    const KIND: EntityKind = EntityKind::PromptTemplate;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Template {
    const KIND: EntityKind = EntityKind::Template;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Instruction {
    const KIND: EntityKind = EntityKind::Instruction;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Context {
    const KIND: EntityKind = EntityKind::Context;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Config {
    const KIND: EntityKind = EntityKind::Config;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Primitive {
    const KIND: EntityKind = EntityKind::Primitive;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for EnvironmentVariable {
    const KIND: EntityKind = EntityKind::EnvironmentVariable;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Function {
    const KIND: EntityKind = EntityKind::Function;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for McpServer {
    const KIND: EntityKind = EntityKind::McpServer;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for McpToolRef {
    const KIND: EntityKind = EntityKind::McpTool;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Connector {
    const KIND: EntityKind = EntityKind::Connector;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Webhook {
    const KIND: EntityKind = EntityKind::Webhook;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Task {
    const KIND: EntityKind = EntityKind::Task;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Hook {
    const KIND: EntityKind = EntityKind::Hook;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for AgentRouter {
    const KIND: EntityKind = EntityKind::AgentRouter;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for ToolRouter {
    const KIND: EntityKind = EntityKind::ToolRouter;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Knowledge {
    const KIND: EntityKind = EntityKind::Knowledge;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Document {
    const KIND: EntityKind = EntityKind::Document;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for RagPipeline {
    const KIND: EntityKind = EntityKind::RagPipeline;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for KnowledgeGraph {
    const KIND: EntityKind = EntityKind::KnowledgeGraph;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Memory {
    const KIND: EntityKind = EntityKind::Memory;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for FeatureStore {
    const KIND: EntityKind = EntityKind::FeatureStore;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Feature {
    const KIND: EntityKind = EntityKind::Feature;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for FeatureList {
    const KIND: EntityKind = EntityKind::FeatureList;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Guardrail {
    const KIND: EntityKind = EntityKind::Guardrail;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Rule {
    const KIND: EntityKind = EntityKind::Rule;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Plugin {
    const KIND: EntityKind = EntityKind::Plugin;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for ErrorPolicy {
    const KIND: EntityKind = EntityKind::ErrorPolicy;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Network {
    const KIND: EntityKind = EntityKind::Network;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Compute {
    const KIND: EntityKind = EntityKind::Compute;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for DataStore {
    const KIND: EntityKind = EntityKind::DataStore;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for SecretStore {
    const KIND: EntityKind = EntityKind::SecretStore;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

impl RegistryEntity for Observability {
    const KIND: EntityKind = EntityKind::Observability;
    fn metadata(&self) -> &EntityMeta {
        &self.meta
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentParseError {
    #[error("expected a non-empty manifest")]
    EmptyManifest,
    #[error("missing field: {0}")]
    MissingField(&'static str),
    #[error("invalid identifier for {0}: {1}")]
    InvalidIdentifier(&'static str, String),
    #[error("invalid line: {0}")]
    InvalidLine(String),
    #[error("slash command must start with /")]
    MissingSlash,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowValidationError {
    #[error("duplicate workflow node: {0}")]
    DuplicateNode(String),
    #[error("workflow edge references missing node: {0}")]
    MissingNode(String),
    #[error("workflow contains a cycle")]
    Cycle,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentContractError {
    #[error("invalid identifier for {0}: {1}")]
    InvalidIdentifier(&'static str, String),
    #[error("missing field: {0}")]
    MissingField(&'static str),
    #[error("invalid uri: {0}")]
    InvalidUri(String),
    #[error("invalid endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("workflow validation failed: {0}")]
    Workflow(#[from] WorkflowValidationError),
}

impl WorkflowDefinition {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn validate_dag(&self) -> Result<Vec<String>, WorkflowValidationError> {
        let mut indegree = BTreeMap::<String, usize>::new();
        let mut outgoing = BTreeMap::<String, Vec<String>>::new();

        for node in &self.nodes {
            if indegree.insert(node.node_id.clone(), 0).is_some() {
                return Err(WorkflowValidationError::DuplicateNode(node.node_id.clone()));
            }
            outgoing.entry(node.node_id.clone()).or_default();
        }

        for edge in &self.edges {
            if !indegree.contains_key(&edge.from) {
                return Err(WorkflowValidationError::MissingNode(edge.from.clone()));
            }
            let Some(count) = indegree.get_mut(&edge.to) else {
                return Err(WorkflowValidationError::MissingNode(edge.to.clone()));
            };
            *count += 1;
            outgoing
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
        }

        let mut ready = indegree
            .iter()
            .filter_map(|(node_id, count)| (*count == 0).then_some(node_id.clone()))
            .collect::<VecDeque<_>>();
        let mut order = Vec::new();

        while let Some(node_id) = ready.pop_front() {
            order.push(node_id.clone());
            let next_nodes = std::mem::take(outgoing.entry(node_id.clone()).or_default());
            for next in next_nodes {
                let Some(count) = indegree.get_mut(&next) else {
                    return Err(WorkflowValidationError::MissingNode(next));
                };
                *count -= 1;
                if *count == 0 {
                    ready.push_back(next);
                }
            }
        }

        if order.len() == self.nodes.len() {
            Ok(order)
        } else {
            Err(WorkflowValidationError::Cycle)
        }
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn parse_skill_manifest(input: &str) -> Result<AgentSkill, AgentParseError> {
    if input.trim().is_empty() {
        return Err(AgentParseError::EmptyManifest);
    }

    let mut fields = BTreeMap::<String, String>::new();
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(AgentParseError::InvalidLine(line.to_string()));
        };
        fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    let tags = fields
        .get("tags")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let skill_id = take_required(&fields, "id")?;
    if !is_agent_identifier(&skill_id) {
        return Err(AgentParseError::InvalidIdentifier("id", skill_id));
    }
    let name = take_required(&fields, "name")?;
    let description = take_required(&fields, "description")?;

    let meta = EntityMeta::new(
        skill_id.clone(),
        skill_id,
        "0.1.0",
        Origin {
            tier: Tier::Domain,
            source: Source::Internal,
        },
        Adaptivity::Configured,
        false,
    )
    .with_title(name)
    .with_description(description)
    .with_tags(tags);

    Ok(AgentSkill {
        meta,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        quality: None,
    })
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn parse_slash_command_invocation(
    input: &str,
) -> Result<SlashCommandInvocation, AgentParseError> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Err(AgentParseError::MissingSlash);
    }

    let mut parts = trimmed[1..].split_whitespace();
    let command = parts
        .next()
        .ok_or(AgentParseError::MissingField("command"))?
        .to_string();
    if !is_agent_identifier(&command) {
        return Err(AgentParseError::InvalidIdentifier("command", command));
    }
    let mut args = BTreeMap::new();

    for (index, part) in parts.enumerate() {
        if let Some((key, value)) = part.split_once('=') {
            if !is_agent_identifier(key) {
                return Err(AgentParseError::InvalidIdentifier("arg", key.to_string()));
            }
            args.insert(key.to_string(), value.to_string());
        } else {
            args.insert(format!("arg{}", index + 1), part.to_string());
        }
    }

    Ok(SlashCommandInvocation { command, args })
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_agent_card_contract(card: &AgentCard) -> Result<(), AgentContractError> {
    validate_identifier_field("name", &card.meta.base.name)?;
    require_non_empty("version", &card.meta.version)?;
    require_non_empty(
        "description",
        card.meta.base.description.as_deref().unwrap_or_default(),
    )?;
    for skill in &card.skills {
        validate_identifier_field("skill_name", &skill.meta.base.name)?;
        require_non_empty(
            "skill_description",
            skill.meta.base.description.as_deref().unwrap_or_default(),
        )?;
    }
    for content_ref in &card.content_refs {
        validate_uri(&content_ref.uri)?;
    }
    if let Some(endpoint) = &card.endpoint {
        validate_endpoint(endpoint)?;
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_workflow_contract(
    workflow: &WorkflowDefinition,
) -> Result<Vec<String>, AgentContractError> {
    validate_identifier_field("workflow_name", &workflow.meta.base.name)?;
    for node in &workflow.nodes {
        validate_identifier_field("node_id", &node.node_id)?;
        require_non_empty("task", &node.task)?;
        if let Some(skill_id) = &node.skill_id {
            validate_identifier_field("skill_id", skill_id)?;
        }
    }
    workflow.validate_dag().map_err(AgentContractError::from)
}

/// DEPRECATED (pre-taxonomy): the legacy seed of entity→table storage mappings.
///
/// Q8 reclassified storage to a `resourcedefinition` persistence facet + relation. Retained
/// because downstream crates still consume it.
#[must_use]
pub fn agent_storage_mappings() -> Vec<StorageMapping> {
    vec![
        StorageMapping {
            entity: "agent_card".to_string(),
            schema: "agents".to_string(),
            table: "agent_card".to_string(),
            primary_key: vec!["agent_id".to_string()],
        },
        StorageMapping {
            entity: "agent_skill".to_string(),
            schema: "agents".to_string(),
            table: "agent_skill".to_string(),
            primary_key: vec!["skill_id".to_string()],
        },
        StorageMapping {
            entity: "workflow_definition".to_string(),
            schema: "agents".to_string(),
            table: "workflow_definition".to_string(),
            primary_key: vec!["workflow_id".to_string()],
        },
        StorageMapping {
            entity: "gotcha".to_string(),
            schema: "agents".to_string(),
            table: "gotcha".to_string(),
            primary_key: vec!["gotcha_id".to_string()],
        },
        StorageMapping {
            entity: "eval_run".to_string(),
            schema: "evals".to_string(),
            table: "eval_run".to_string(),
            primary_key: vec!["run_id".to_string()],
        },
    ]
}

/// The self-describing registry: one [`ResourceDefinition`] per classified kind.
///
/// Ordered by manifest-group. Kinds with a concrete Rust spec type carry their JSON Schema;
/// `candidate` kinds carry `None` until their spec lands.
#[must_use]
pub fn resource_definitions() -> Vec<ResourceDefinition> {
    EntityKind::ALL
        .into_iter()
        .map(|kind| ResourceDefinition {
            group: TDW_API_GROUP.to_string(),
            kind,
            manifest_group: kind.group(),
            spec_schema: spec_schema_for(kind),
            has_data_facets: kind.is_data_kind(),
            autonomy_capable: kind.is_autonomy_capable(),
        })
        .collect()
}

/// JSON Schema for a kind's spec, when a concrete Rust type backs it. The
/// `resourcedefinition` kind returns its own schema, making the registry self-describing.
/// The warehouse `domain` kinds are `candidate` kinds carrying `None` until their spec
/// types land (knowledge-system A3).
fn spec_schema_for(kind: EntityKind) -> Option<Value> {
    let schema = match kind {
        EntityKind::Instrument
        | EntityKind::Account
        | EntityKind::Strategy
        | EntityKind::Dataset
        | EntityKind::Provider
        | EntityKind::Symbol
        | EntityKind::Venue
        | EntityKind::Tag
        // Finding is a first-class knowledge kind (K-X6) with no dedicated
        // Rust spec type yet; schema will land with a future spec crate.
        // Pattern is a first-class knowledge kind (K-R4) with no dedicated
        // Rust spec type yet; schema will land with a future spec crate.
        // OpenQuestion is a first-class knowledge kind (K-X8) with no dedicated
        // Rust spec type yet; schema will land with a future spec crate.
        // Episode is a first-class episodic memory kind (K-M2) with no dedicated
        // Rust spec type yet; it is a data-plane document, not a registry entity.
        | EntityKind::Finding
        | EntityKind::Pattern
        | EntityKind::OpenQuestion
        | EntityKind::Episode => return None,
        EntityKind::Agent => schema_json::<AgentCard>(),
        EntityKind::Skill => schema_json::<AgentSkill>(),
        EntityKind::Tool => schema_json::<Tool>(),
        EntityKind::Prompt => schema_json::<Prompt>(),
        EntityKind::Command => schema_json::<SlashCommand>(),
        EntityKind::Gotcha => schema_json::<Gotcha>(),
        EntityKind::Workflow => schema_json::<WorkflowDefinition>(),
        EntityKind::Evaluation => schema_json::<Evaluation>(),
        EntityKind::ResourceDefinition => schema_json::<ResourceDefinition>(),
        EntityKind::Personality => schema_json::<Personality>(),
        EntityKind::PromptTemplate => schema_json::<PromptTemplate>(),
        EntityKind::Template => schema_json::<Template>(),
        EntityKind::Instruction => schema_json::<Instruction>(),
        EntityKind::Context => schema_json::<Context>(),
        EntityKind::Config => schema_json::<Config>(),
        EntityKind::Primitive => schema_json::<Primitive>(),
        EntityKind::EnvironmentVariable => schema_json::<EnvironmentVariable>(),
        EntityKind::Function => schema_json::<Function>(),
        EntityKind::McpServer => schema_json::<McpServer>(),
        EntityKind::McpTool => schema_json::<McpToolRef>(),
        EntityKind::Connector => schema_json::<Connector>(),
        EntityKind::Webhook => schema_json::<Webhook>(),
        EntityKind::Task => schema_json::<Task>(),
        EntityKind::Hook => schema_json::<Hook>(),
        EntityKind::AgentRouter => schema_json::<AgentRouter>(),
        EntityKind::ToolRouter => schema_json::<ToolRouter>(),
        EntityKind::Knowledge => schema_json::<Knowledge>(),
        EntityKind::Document => schema_json::<Document>(),
        EntityKind::RagPipeline => schema_json::<RagPipeline>(),
        EntityKind::KnowledgeGraph => schema_json::<KnowledgeGraph>(),
        EntityKind::Memory => schema_json::<Memory>(),
        EntityKind::FeatureStore => schema_json::<FeatureStore>(),
        EntityKind::Feature => schema_json::<Feature>(),
        EntityKind::FeatureList => schema_json::<FeatureList>(),
        EntityKind::Guardrail => schema_json::<Guardrail>(),
        EntityKind::Rule => schema_json::<Rule>(),
        EntityKind::Plugin => schema_json::<Plugin>(),
        EntityKind::ErrorPolicy => schema_json::<ErrorPolicy>(),
        EntityKind::Network => schema_json::<Network>(),
        EntityKind::Compute => schema_json::<Compute>(),
        EntityKind::DataStore => schema_json::<DataStore>(),
        EntityKind::SecretStore => schema_json::<SecretStore>(),
        EntityKind::Observability => schema_json::<Observability>(),
    };
    Some(schema)
}

/// DEPRECATED (pre-taxonomy): the legacy fixed schema bundle.
///
/// Q8 reclassified storage to a `resourcedefinition` persistence facet; prefer
/// [`resource_definitions`] / [`spec_schema_for`] for the self-describing registry.
/// Retained because downstream crates still consume it.
#[must_use]
pub fn schema_bundle() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("agent_card", schema_json::<AgentCard>()),
        ("agent_skill", schema_json::<AgentSkill>()),
        ("slash_command", schema_json::<SlashCommand>()),
        (
            "slash_command_invocation",
            schema_json::<SlashCommandInvocation>(),
        ),
        ("content_ref", schema_json::<ContentRef>()),
        ("eval_run_request", schema_json::<EvalRunRequest>()),
        ("workflow_definition", schema_json::<WorkflowDefinition>()),
        ("gotcha", schema_json::<Gotcha>()),
        ("storage_mapping", schema_json::<StorageMapping>()),
    ])
}

#[must_use]
pub fn sample_agent_card() -> AgentCard {
    AgentCard {
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
        .with_description("Generates evidence-backed market research notes."),
        skills: vec![AgentSkill {
            meta: EntityMeta::new(
                "research.note",
                "research.note",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Configured,
                false,
            )
            .with_title("Research Note")
            .with_description("Draft a research note from retrieved content.")
            .with_tags(vec!["research".to_string(), "mcp".to_string()]),
            input_schema: json!({"type": "object", "required": ["symbol"]}),
            output_schema: json!({"type": "object", "required": ["note"]}),
            quality: None,
        }],
        content_refs: vec![ContentRef {
            uri: "tdw://docs/research-template".to_string(),
            kind: ContentKind::Prompt,
            checksum: None,
            tags: vec!["prompt".to_string()],
        }],
        endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
        tool_scope: Vec::new(),
        runtime: None,
    }
}

#[must_use]
pub fn gotcha_seed() -> Vec<Gotcha> {
    vec![Gotcha {
        meta: EntityMeta::new(
            "agent-output-needs-provenance",
            "agent-output-needs-provenance",
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::None,
            false,
        )
        .with_title("Agent output needs provenance"),
        severity: GotchaSeverity::Warning,
        applies_to: vec!["research.note".to_string()],
        remediation: "Attach source content refs before persisting agent output.".to_string(),
        source_ref: None,
    }]
}

fn take_required(
    fields: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<String, AgentParseError> {
    fields
        .get(key)
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or(AgentParseError::MissingField(key))
}

fn schema_json<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T))
        .unwrap_or_else(|error| panic!("schema for agent type should serialize: {error}"))
}

#[must_use]
pub fn schema_name_set() -> BTreeSet<&'static str> {
    AGENT_SCHEMA_NAMES.into_iter().collect()
}

fn validate_identifier_field(field: &'static str, value: &str) -> Result<(), AgentContractError> {
    if is_agent_identifier(value) {
        Ok(())
    } else {
        Err(AgentContractError::InvalidIdentifier(
            field,
            value.to_string(),
        ))
    }
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), AgentContractError> {
    if value.trim().is_empty() {
        Err(AgentContractError::MissingField(field))
    } else {
        Ok(())
    }
}

fn validate_uri(value: &str) -> Result<(), AgentContractError> {
    if is_safe_uri(value) {
        Ok(())
    } else {
        Err(AgentContractError::InvalidUri(value.to_string()))
    }
}

fn validate_endpoint(value: &str) -> Result<(), AgentContractError> {
    if (value.starts_with("mcp://") || value.starts_with("https://")) && is_safe_uri(value) {
        Ok(())
    } else {
        Err(AgentContractError::InvalidEndpoint(value.to_string()))
    }
}

fn is_safe_uri(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains("..")
        && !value.chars().any(char::is_control)
        && (value.starts_with("tdw://")
            || value.starts_with("mcp://")
            || value.starts_with("https://"))
}

fn is_agent_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_round_trips_to_mcp_via_projection() {
        let tool = Tool {
            meta: EntityMeta::new(
                "search",
                "search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Search"),
            input_schema: json!({"type": "object"}),
            output_schema: None,
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: ToolImplementation::Unbound,
        };

        // Tool -> Resource (registry form) -> re-typed -> MCP wire.
        let resource = tool.to_resource().expect("to_resource");
        let projected = project_to_mcp(&resource)
            .expect("projection should not error")
            .expect("a tool is mcp-exposable");
        match projected {
            McpEntity::Tool(mcp_tool) => {
                assert_eq!(mcp_tool, McpTool::from(&tool));
                assert_eq!(mcp_tool.display_name(), "Search");
            }
            McpEntity::Prompt(_) => panic!("expected a tool projection"),
        }

        // A non-MCP-exposable kind (agent) projects to None.
        let card_resource = sample_agent_card().to_resource().expect("card resource");
        assert!(
            project_to_mcp(&card_resource)
                .expect("projection should not error")
                .is_none()
        );
    }

    #[test]
    fn tool_implementation_defaults_unbound_and_binds_via_json5() {
        // No implementation declared -> Unbound (still lists, not executable).
        let unbound: Tool = load_typed(
            r#"{ meta: { name:"t", id:"t", version:"0.1.0",
                 origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false },
                 input_schema: { type: "object" } }"#,
        )
        .expect("minimal tool parses");
        assert_eq!(unbound.implementation, ToolImplementation::Unbound);

        // A Command implementation binds via the adjacently-tagged enum in JSON5.
        let bound: Tool = load_typed(
            r#"{ meta: { name:"echo", id:"echo", version:"0.1.0",
                 origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false },
                 input_schema: { type: "object" },
                 implementation: { kind: "command",
                   value: { command: "echo", args: ["hi"], background: false } } }"#,
        )
        .expect("command tool parses");
        assert_eq!(
            bound.implementation,
            ToolImplementation::Command {
                command: "echo".to_string(),
                args: vec!["hi".to_string()],
                background: false,
            }
        );
    }

    #[test]
    fn canonical_tool_projects_to_mcp_tool() {
        let tool = Tool {
            meta: EntityMeta::new(
                "search",
                "search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Search"),
            input_schema: json!({"type": "object"}),
            output_schema: Some(json!({"type": "object"})),
            effect: ToolEffect::ReadOnly,
            idempotent: true,
            open_world: false,
            implementation: ToolImplementation::Unbound,
        };

        let projected = McpTool::from(&tool);
        assert_eq!(projected.base.name, "search");
        assert_eq!(projected.display_name(), "Search");
        // ReadOnly effect → parallel-safe via MCP annotations.
        assert_eq!(projected.parallel_safety(), ParallelSafety::Parallel);
        assert!(projected.is_idempotent());

        let destructive = Tool {
            effect: ToolEffect::Destructive,
            idempotent: false,
            ..tool
        };
        assert_eq!(
            McpTool::from(&destructive).parallel_safety(),
            ParallelSafety::Sequential
        );
    }

    #[test]
    fn canonical_prompt_projects_to_mcp_prompt() {
        let prompt = Prompt {
            meta: EntityMeta::new(
                "research.prompt",
                "research.prompt",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Research Prompt"),
            template: "Summarize {{ symbol }}".to_string(),
            arguments: vec![PromptArgument {
                name: "symbol".to_string(),
                description: Some("Ticker".to_string()),
                required: true,
                // R1: domain default resolves from an env variable at runtime.
                default: Some(Reference::Variable("DEFAULT_SYMBOL".to_string())),
            }],
        };

        let projected = McpPrompt::from(&prompt);
        assert_eq!(projected.base.name, "research.prompt");
        assert_eq!(projected.arguments.len(), 1);
        assert_eq!(projected.arguments[0].required, Some(true));

        // The R1 default is domain-only: it must not appear on the MCP wire.
        let encoded = serde_json::to_value(&projected).expect("prompt should serialize");
        let arg = &encoded["arguments"][0];
        assert!(arg.get("default").is_none());
        assert_eq!(arg.get("name").and_then(Value::as_str), Some("symbol"));
    }

    #[test]
    fn memory_defaults_retention_and_round_trips() {
        let memory = Memory {
            meta: EntityMeta::new(
                "session.scratch",
                "session.scratch",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::SelfModifying,
                false,
            ),
            retention: Retention::Core,
            last_consolidated: None,
            source_entries: vec!["m1".to_string(), "m2".to_string()],
            facets: DataFacets {
                plane: Plane::Agent,
                materialization: Materialization::Materialized,
                as_of: None,
                validation: None,
            },
        };
        let resource = memory.to_resource().expect("to_resource");
        let back: Memory = entity_from_resource(&resource).expect("from_resource");
        assert_eq!(back, memory);

        // A minimal hand-authored memory (no retention) defaults to ShortTerm.
        let minimal: Memory = load_typed(
            r#"{ meta: { name:"m", id:"m", version:"0.1.0",
                 origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false },
                 facets: { plane:"agent", materialization:"materialized" } }"#,
        )
        .expect("minimal memory should parse");
        assert_eq!(minimal.retention, Retention::ShortTerm);
    }

    #[test]
    fn evaluation_identity_round_trips_and_validates_facets() {
        let eval = Evaluation {
            meta: EntityMeta::new(
                "eval.market",
                "eval.market",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            ),
            dataset_id: "golden-market-notes".to_string(),
            facets: EvalFacets::default(),
        };
        let resource = eval.to_resource().expect("to_resource");
        let back: Evaluation = entity_from_resource(&resource).expect("from_resource");
        assert_eq!(back, eval);

        // The calibration range guard is now reachable through a registry entity.
        let mut bad = eval;
        bad.facets.ops.calibration = Some(5.0);
        assert!(bad.validate().is_err());
    }

    #[test]
    fn resource_definitions_describe_every_kind() {
        let defs = resource_definitions();
        assert_eq!(defs.len(), EntityKind::ALL.len());

        let find = |kind: EntityKind| {
            defs.iter()
                .find(|definition| definition.kind == kind)
                .unwrap_or_else(|| panic!("missing definition: {kind:?}"))
        };

        // kinds with a concrete Rust spec type carry their schema; the warehouse
        // `domain` kinds are `candidate` kinds carrying `None` until their spec
        // types land (knowledge-system A3). `Finding` (K-X6), `Pattern` (K-R4),
        // `OpenQuestion` (K-X8), and `Episode` (K-M2) are knowledge-group kinds
        // that likewise have no spec type yet — they carry `None` until a
        // dedicated spec crate lands.
        assert!(find(EntityKind::Agent).spec_schema.is_some());
        assert!(
            resource_definitions()
                .iter()
                .all(|d| d.spec_schema.is_some()
                    || d.kind.group() == Group::Domain
                    || d.kind == EntityKind::Finding
                    || d.kind == EntityKind::Pattern
                    || d.kind == EntityKind::OpenQuestion
                    || d.kind == EntityKind::Episode)
        );
        assert!(find(EntityKind::Instrument).spec_schema.is_none());
        // facet flags come straight from the kind registry.
        assert!(find(EntityKind::Agent).autonomy_capable);
        assert!(!find(EntityKind::Skill).autonomy_capable);
        assert!(find(EntityKind::Memory).has_data_facets);
        // self-describing: the meta-schema kind carries its own schema.
        assert!(find(EntityKind::ResourceDefinition).spec_schema.is_some());
        assert_eq!(find(EntityKind::Agent).group, "tdw.finx");
    }

    #[test]
    fn registry_entity_projects_to_resource() {
        let card = sample_agent_card();
        let resource = card
            .to_resource()
            .unwrap_or_else(|error| panic!("card should project: {error}"));

        assert_eq!(resource.kind, EntityKind::Agent);
        assert_eq!(resource.api_version, "tdw.finx/v1");
        assert_eq!(resource.metadata.id, "market-researcher");

        let spec = resource
            .spec
            .as_object()
            .unwrap_or_else(|| panic!("spec should be an object"));
        // Payload is present, but the metadata was lifted into the envelope (not duplicated).
        assert!(spec.contains_key("skills"));
        assert!(spec.contains_key("content_refs"));
        assert!(!spec.contains_key("meta"));
    }

    #[test]
    fn exports_stable_schema_bundle() {
        let bundle = schema_bundle();
        for name in AGENT_SCHEMA_NAMES {
            assert!(bundle.contains_key(name), "missing schema: {name}");
        }
        assert_eq!(bundle.len(), AGENT_SCHEMA_NAMES.len());
    }

    fn agent_card_with_scope(tool_scope: Vec<String>, runtime: Option<AgentRuntime>) -> AgentCard {
        let json = include_str!("../tests/golden/agent_card.json");
        let mut card = serde_json::from_str::<AgentCard>(json)
            .unwrap_or_else(|error| panic!("agent card fixture should parse: {error}"));
        card.tool_scope = tool_scope;
        card.runtime = runtime;
        card
    }

    #[test]
    fn grants_tool_respects_allowlist() {
        let card = agent_card_with_scope(vec!["read".into(), "search".into()], None);
        assert!(card.grants_tool("read"));
        assert!(card.grants_tool("search"));
        assert!(!card.grants_tool("write"));

        // An empty scope grants nothing.
        let empty = agent_card_with_scope(Vec::new(), None);
        assert!(!empty.grants_tool("read"));
    }

    #[test]
    fn child_scope_returns_intersection_never_superset() {
        let card =
            agent_card_with_scope(vec!["read".into(), "search".into(), "fetch".into()], None);

        // A requested tool NOT in the parent scope is dropped (never a superset).
        let requested = vec!["search".into(), "write".into(), "read".into()];
        let child = card.child_scope(&requested);
        assert_eq!(child, vec!["read".to_string(), "search".to_string()]);
        assert!(!child.iter().any(|tool| tool == "write"));

        // The child scope is always a subset of the parent scope.
        for tool in &child {
            assert!(card.grants_tool(tool));
        }

        // No requested tools => empty child scope.
        assert!(card.child_scope(&[]).is_empty());
    }

    #[test]
    fn agent_card_round_trips_with_scope_and_runtime() {
        let runtime = AgentRuntime {
            provider: Some("anthropic".into()),
            model: Some("claude-opus".into()),
            timeout_secs: Some(30),
            max_iterations: Some(8),
        };
        let card = agent_card_with_scope(vec!["read".into(), "search".into()], Some(runtime));

        let encoded = serde_json::to_string(&card)
            .unwrap_or_else(|error| panic!("agent card should serialize: {error}"));
        assert!(encoded.contains("tool_scope"));
        assert!(encoded.contains("runtime"));

        let decoded = serde_json::from_str::<AgentCard>(&encoded)
            .unwrap_or_else(|error| panic!("agent card should deserialize: {error}"));
        assert_eq!(decoded, card);
        assert_eq!(
            decoded.tool_scope,
            vec!["read".to_string(), "search".to_string()]
        );
        let decoded_runtime = decoded.runtime.expect("runtime present after round-trip");
        assert_eq!(decoded_runtime.provider.as_deref(), Some("anthropic"));
        assert_eq!(decoded_runtime.timeout_secs, Some(30));
        assert_eq!(decoded_runtime.max_iterations, Some(8));
    }

    #[test]
    fn agent_card_defaults_omit_new_fields() {
        // Empty/None defaults are skipped, keeping existing golden snapshots byte-identical.
        let card = agent_card_with_scope(Vec::new(), None);
        let encoded = serde_json::to_string(&card)
            .unwrap_or_else(|error| panic!("agent card should serialize: {error}"));
        assert!(!encoded.contains("tool_scope"));
        assert!(!encoded.contains("runtime"));
    }

    #[test]
    fn a2a_card_round_trips() {
        let json = include_str!("../tests/golden/agent_card.json");
        let card = serde_json::from_str::<AgentCard>(json)
            .unwrap_or_else(|error| panic!("agent card fixture should parse: {error}"));
        assert_eq!(card.meta.id, "market-researcher");
        assert!(card.validate().is_ok());

        let encoded = serde_json::to_string(&card)
            .unwrap_or_else(|error| panic!("agent card should serialize: {error}"));
        let decoded = serde_json::from_str::<AgentCard>(&encoded)
            .unwrap_or_else(|error| panic!("agent card should deserialize: {error}"));
        assert_eq!(decoded, card);
        assert_eq!(validate_agent_card_contract(&card), Ok(()));
    }

    #[test]
    fn parses_skill_manifest_fixture() {
        let skill = parse_skill_manifest(
            "id: research.note\nname: Research Note\ndescription: Draft note\ntags: research, mcp",
        )
        .unwrap_or_else(|error| panic!("skill fixture should parse: {error}"));

        assert_eq!(skill.meta.id, "research.note");
        assert_eq!(skill.meta.tags, vec!["research", "mcp"]);
    }

    #[test]
    fn parses_slash_command_invocation() {
        let invocation = parse_slash_command_invocation("/research symbol=AAPL horizon=1d")
            .unwrap_or_else(|error| panic!("slash command should parse: {error}"));

        assert_eq!(invocation.command, "research");
        assert_eq!(invocation.args.get("symbol"), Some(&"AAPL".to_string()));
    }

    #[test]
    fn rejects_unsafe_agent_contract_inputs() {
        assert_eq!(
            parse_slash_command_invocation("/../research symbol=AAPL"),
            Err(AgentParseError::InvalidIdentifier(
                "command",
                "../research".to_string()
            ))
        );

        let mut card = sample_agent_card();
        card.endpoint = Some("file:///etc/passwd".to_string());
        assert_eq!(
            validate_agent_card_contract(&card),
            Err(AgentContractError::InvalidEndpoint(
                "file:///etc/passwd".to_string()
            ))
        );
    }

    #[test]
    fn validates_workflow_dag_and_rejects_cycles() {
        let workflow = WorkflowDefinition {
            meta: EntityMeta::new(
                "research-flow",
                "research-flow",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::Configured,
                false,
            ),
            nodes: vec![
                WorkflowNode {
                    node_id: "retrieve".to_string(),
                    task: "retrieve context".to_string(),
                    skill_id: None,
                },
                WorkflowNode {
                    node_id: "draft".to_string(),
                    task: "draft note".to_string(),
                    skill_id: Some("research.note".to_string()),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "retrieve".to_string(),
                to: "draft".to_string(),
            }],
        };
        assert_eq!(
            validate_workflow_contract(&workflow)
                .unwrap_or_else(|error| panic!("workflow should validate: {error}")),
            vec!["retrieve".to_string(), "draft".to_string()]
        );

        let cycle = WorkflowDefinition {
            edges: vec![
                WorkflowEdge {
                    from: "retrieve".to_string(),
                    to: "draft".to_string(),
                },
                WorkflowEdge {
                    from: "draft".to_string(),
                    to: "retrieve".to_string(),
                },
            ],
            ..workflow
        };
        assert_eq!(cycle.validate_dag(), Err(WorkflowValidationError::Cycle));
    }

    #[test]
    fn minimal_tool_json5_parses_and_defaults() {
        // Hand-authored minimal tool: only meta + inputSchema. Everything else defaults.
        let tool = load_typed::<Tool>(
            r#"{ meta: { name:"t", id:"t", version:"0.1.0", origin:{tier:"Domain",source:"Internal"}, adaptivity:"None", autonomous:false }, input_schema: { type: "object" } }"#,
        )
        .unwrap_or_else(|error| panic!("minimal tool should parse: {error}"));

        assert_eq!(tool.effect, ToolEffect::WriteSafe);
        assert!(!tool.idempotent);
        assert!(!tool.open_world);
        assert!(tool.output_schema.is_none());
    }

    #[test]
    fn tool_identity_round_trip_through_resource() {
        let tool = Tool {
            meta: EntityMeta::new(
                "search",
                "search",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            )
            .with_title("Search"),
            input_schema: json!({"type": "object"}),
            output_schema: Some(json!({"type": "object"})),
            effect: ToolEffect::Destructive,
            idempotent: true,
            open_world: true,
            implementation: ToolImplementation::Unbound,
        };
        let resource = tool.to_resource().expect("to_resource");
        let back: Tool = entity_from_resource(&resource).expect("entity_from_resource");
        assert_eq!(back, tool);
    }

    #[test]
    fn prompt_identity_round_trip_through_resource() {
        let prompt = Prompt {
            meta: EntityMeta::new(
                "research.prompt",
                "research.prompt",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            ),
            template: "Summarize {{ symbol }}".to_string(),
            arguments: vec![PromptArgument {
                name: "symbol".to_string(),
                description: Some("Ticker".to_string()),
                required: true,
                default: Some(Reference::Variable("DEFAULT_SYMBOL".to_string())),
            }],
        };
        let resource = prompt.to_resource().expect("to_resource");
        let back: Prompt = entity_from_resource(&resource).expect("entity_from_resource");
        assert_eq!(back, prompt);
    }

    #[test]
    fn agent_card_identity_round_trip_through_resource() {
        let card = sample_agent_card();
        let resource = card.to_resource().expect("to_resource");
        let back: AgentCard = entity_from_resource(&resource).expect("entity_from_resource");
        assert_eq!(back, card);
    }

    #[test]
    fn knowledge_identity_round_trip_through_resource() {
        let knowledge = Knowledge {
            meta: EntityMeta::new(
                "market-facts",
                "market-facts",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                Adaptivity::None,
                false,
            ),
            body: "Equities settle T+1.".to_string(),
            facets: DataFacets {
                plane: Plane::Platform,
                materialization: Materialization::Materialized,
                as_of: Some("2026-05-31T00:00:00Z".to_string()),
                validation: Some(ValidationState {
                    status: ValidationStatus::Ready,
                    missing_fraction: Some(0.0),
                    schema_conformant: Some(true),
                    last_validated: None,
                }),
            },
        };
        let resource = knowledge.to_resource().expect("to_resource");
        let back: Knowledge = entity_from_resource(&resource).expect("entity_from_resource");
        assert_eq!(back, knowledge);
    }

    fn skill_with_adaptivity(adaptivity: Adaptivity) -> AgentSkill {
        AgentSkill {
            meta: EntityMeta::new(
                "research.note",
                "research.note",
                "0.1.0",
                Origin {
                    tier: Tier::Domain,
                    source: Source::Internal,
                },
                adaptivity,
                false,
            ),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            quality: None,
        }
    }

    #[test]
    fn apply_eval_feedback_updates_learning_skill_and_increments_runs() {
        let mut skill = skill_with_adaptivity(Adaptivity::Learning);
        skill
            .apply_eval_feedback(0.9, "2026-05-31T00:00:00+00:00", 0.5)
            .expect("learning skill accepts feedback");
        let quality = skill.quality.as_ref().expect("quality set");
        assert_eq!(quality.runs, 1);
        assert_eq!(quality.pass_rate, Some(0.9));
        assert_eq!(quality.last_eval_score, Some(0.9));
        assert!(!quality.disabled);
        assert_eq!(
            quality.last_eval.as_deref(),
            Some("2026-05-31T00:00:00+00:00")
        );

        // A second run increments the run count.
        skill
            .apply_eval_feedback(0.8, "2026-06-01T00:00:00+00:00", 0.5)
            .expect("second feedback");
        assert_eq!(skill.quality.as_ref().expect("quality").runs, 2);
    }

    #[test]
    fn apply_eval_feedback_disables_skill_below_threshold() {
        let mut skill = skill_with_adaptivity(Adaptivity::Learning);
        skill
            .apply_eval_feedback(0.2, "2026-05-31T00:00:00+00:00", 0.5)
            .expect("learning skill accepts feedback");
        assert!(skill.quality.as_ref().expect("quality").disabled);
    }

    #[test]
    fn apply_eval_feedback_gate_rejects_configured_skill_and_leaves_quality_none() {
        let mut skill = skill_with_adaptivity(Adaptivity::Configured);
        let result = skill.apply_eval_feedback(0.9, "2026-05-31T00:00:00+00:00", 0.5);
        assert_eq!(
            result,
            Err(AdaptivityError::NotLearning {
                name: "research.note".to_string(),
                adaptivity: Adaptivity::Configured,
            })
        );
        assert!(skill.quality.is_none());
    }

    #[test]
    fn ensure_adaptive_for_feedback_gates_on_learning() {
        let learning = skill_with_adaptivity(Adaptivity::Learning);
        assert!(ensure_adaptive_for_feedback(&learning.meta).is_ok());
        let self_modifying = skill_with_adaptivity(Adaptivity::SelfModifying);
        assert!(ensure_adaptive_for_feedback(&self_modifying.meta).is_ok());
        let none = skill_with_adaptivity(Adaptivity::None);
        assert!(ensure_adaptive_for_feedback(&none.meta).is_err());
    }

    #[test]
    fn skill_quality_is_omitted_when_none_in_golden() {
        // The additive `quality` field defaults to None and is skipped on serialize, so the
        // golden agent card round-trips byte-identically (no `quality` key appears).
        let json = include_str!("../tests/golden/agent_card.json");
        let card = serde_json::from_str::<AgentCard>(json).expect("golden parses");
        assert!(card.skills.iter().all(|skill| skill.quality.is_none()));
        let encoded = serde_json::to_value(&card).expect("serialize");
        let skill = &encoded["skills"][0];
        assert!(
            skill.get("quality").is_none(),
            "quality must be omitted when None"
        );
    }

    #[test]
    fn data_facets_rejects_out_of_range_missing_fraction() {
        let facets = DataFacets {
            plane: Plane::Platform,
            materialization: Materialization::Materialized,
            as_of: None,
            validation: Some(ValidationState {
                status: ValidationStatus::Draft,
                missing_fraction: Some(5.0),
                schema_conformant: None,
                last_validated: None,
            }),
        };
        assert!(facets.validate().is_err());
    }
}
