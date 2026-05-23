#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use validator::Validate;

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
    pub checksum: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentSkill {
    #[validate(length(min = 1))]
    pub skill_id: String,
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentCard {
    #[validate(length(min = 1))]
    pub agent_id: String,
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub version: String,
    #[validate(length(min = 1))]
    pub description: String,
    pub skills: Vec<AgentSkill>,
    pub content_refs: Vec<ContentRef>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashArg {
    #[validate(length(min = 1))]
    pub name: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashCommand {
    #[validate(length(min = 1))]
    pub name: String,
    #[validate(length(min = 1))]
    pub description: String,
    pub args: Vec<SlashArg>,
    pub workflow_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SlashCommandInvocation {
    #[validate(length(min = 1))]
    pub command: String,
    pub args: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalCase {
    #[validate(length(min = 1))]
    pub case_id: String,
    #[validate(length(min = 1))]
    pub prompt: String,
    pub expected_refs: Vec<ContentRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalRunRequest {
    #[validate(length(min = 1))]
    pub run_id: String,
    #[validate(length(min = 1))]
    pub agent_id: String,
    #[validate(length(min = 1))]
    pub dataset_id: String,
    pub cases: Vec<EvalCase>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalMetric {
    #[validate(length(min = 1))]
    pub metric_name: String,
    pub metric_value: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct WorkflowNode {
    #[validate(length(min = 1))]
    pub node_id: String,
    #[validate(length(min = 1))]
    pub task: String,
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
    #[validate(length(min = 1))]
    pub workflow_id: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Gotcha {
    #[validate(length(min = 1))]
    pub gotcha_id: String,
    #[validate(length(min = 1))]
    pub title: String,
    pub severity: GotchaSeverity,
    pub applies_to: Vec<String>,
    #[validate(length(min = 1))]
    pub remediation: String,
    pub source_ref: Option<ContentRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StorageMapping {
    pub entity: String,
    pub schema: String,
    pub table: String,
    pub primary_key: Vec<String>,
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
            let next_nodes = outgoing.get(&node_id).cloned().unwrap_or_default();
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

    Ok(AgentSkill {
        skill_id,
        name: take_required(&fields, "name")?,
        description: take_required(&fields, "description")?,
        input_schema: json!({"type": "object"}),
        output_schema: json!({"type": "object"}),
        tags,
    })
}

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

pub fn validate_agent_card_contract(card: &AgentCard) -> Result<(), AgentContractError> {
    validate_identifier_field("agent_id", &card.agent_id)?;
    require_non_empty("name", &card.name)?;
    require_non_empty("version", &card.version)?;
    require_non_empty("description", &card.description)?;
    for skill in &card.skills {
        validate_identifier_field("skill_id", &skill.skill_id)?;
        require_non_empty("skill_name", &skill.name)?;
        require_non_empty("skill_description", &skill.description)?;
    }
    for content_ref in &card.content_refs {
        validate_uri(&content_ref.uri)?;
    }
    if let Some(endpoint) = &card.endpoint {
        validate_endpoint(endpoint)?;
    }
    Ok(())
}

pub fn validate_workflow_contract(
    workflow: &WorkflowDefinition,
) -> Result<Vec<String>, AgentContractError> {
    validate_identifier_field("workflow_id", &workflow.workflow_id)?;
    for node in &workflow.nodes {
        validate_identifier_field("node_id", &node.node_id)?;
        require_non_empty("task", &node.task)?;
        if let Some(skill_id) = &node.skill_id {
            validate_identifier_field("skill_id", skill_id)?;
        }
    }
    workflow.validate_dag().map_err(AgentContractError::from)
}

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

pub fn sample_agent_card() -> AgentCard {
    AgentCard {
        agent_id: "market-researcher".to_string(),
        name: "Market Researcher".to_string(),
        version: "0.1.0".to_string(),
        description: "Generates evidence-backed market research notes.".to_string(),
        skills: vec![AgentSkill {
            skill_id: "research.note".to_string(),
            name: "Research Note".to_string(),
            description: "Draft a research note from retrieved content.".to_string(),
            input_schema: json!({"type": "object", "required": ["symbol"]}),
            output_schema: json!({"type": "object", "required": ["note"]}),
            tags: vec!["research".to_string(), "mcp".to_string()],
        }],
        content_refs: vec![ContentRef {
            uri: "tdw://docs/research-template".to_string(),
            kind: ContentKind::Prompt,
            checksum: None,
            tags: vec!["prompt".to_string()],
        }],
        endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
    }
}

pub fn gotcha_seed() -> Vec<Gotcha> {
    vec![Gotcha {
        gotcha_id: "agent-output-needs-provenance".to_string(),
        title: "Agent output needs provenance".to_string(),
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
    fn exports_stable_schema_bundle() {
        let bundle = schema_bundle();
        for name in AGENT_SCHEMA_NAMES {
            assert!(bundle.contains_key(name), "missing schema: {name}");
        }
        assert_eq!(bundle.len(), AGENT_SCHEMA_NAMES.len());
    }

    #[test]
    fn a2a_card_round_trips() {
        let json = include_str!("../tests/golden/agent_card.json");
        let card = serde_json::from_str::<AgentCard>(json)
            .unwrap_or_else(|error| panic!("agent card fixture should parse: {error}"));
        assert_eq!(card.agent_id, "market-researcher");
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

        assert_eq!(skill.skill_id, "research.note");
        assert_eq!(skill.tags, vec!["research", "mcp"]);
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
            workflow_id: "research-flow".to_string(),
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
}
