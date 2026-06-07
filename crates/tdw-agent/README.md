# tdw-agent

The agent/entity taxonomy core: the classified `EntityKind` set, the spec types
for each kind, the self-describing resource registry, manifest parsing, workflow
DAG validation, and the MCP projection.

## Purpose

`tdw-agent` is the schema/contract heart of the agent layer. It defines:

- The closed [`EntityKind`] taxonomy and its `Group`ing, plus a concrete Rust
  spec type per kind ([`AgentCard`], [`AgentSkill`], [`Tool`], [`Prompt`],
  [`WorkflowDefinition`], [`Gotcha`], [`Evaluation`], [`Memory`], … 40+ kinds),
  each `JsonSchema` + `Validate`.
- The self-describing registry: [`resource_definitions`] emits one
  [`ResourceDefinition`] (with JSON Schema) per kind.
- Manifest + invocation parsing ([`parse_skill_manifest`],
  [`parse_slash_command_invocation`]) and contract validation
  ([`validate_agent_card_contract`], [`validate_workflow_contract`]).
- Workflow DAG validation ([`WorkflowDefinition::validate_dag`] — topological sort
  with cycle/duplicate/missing-node detection).
- The MCP projection (`mcp` module: `project_to_mcp`, `McpTool`, `McpPrompt`, …).
- Resource loading (`loader`, `resource`), the registry (`registry`), filesystem
  watching (`watch`), and memory consolidation planning (`consolidate`).

## Feature flags

None. Dependencies: `json5`, `notify`, `schemars`, `serde`, `serde_json`,
`thiserror`, `validator`.

## Environment variables

None read directly by the crate.

## Quickstart

```rust
use tdw_agent::{sample_agent_card, validate_agent_card_contract};

let card = sample_agent_card();
validate_agent_card_contract(&card)?;
assert_eq!(card.meta.id, "market-researcher");
# Ok::<(), tdw_agent::AgentContractError>(())
```

Workflow DAG validation:

```rust
use tdw_agent::{WorkflowDefinition, WorkflowEdge, WorkflowNode};
# use tdw_agent::{EntityMeta, Origin, Tier, Source, Adaptivity};
# let meta = EntityMeta::new("flow", "flow", "0.1.0", Origin { tier: Tier::Domain, source: Source::Internal }, Adaptivity::None, false);
let workflow = WorkflowDefinition {
    meta,
    nodes: vec![
        WorkflowNode { node_id: "retrieve".into(), task: "retrieve".into(), skill_id: None },
        WorkflowNode { node_id: "draft".into(), task: "draft".into(), skill_id: None },
    ],
    edges: vec![WorkflowEdge { from: "retrieve".into(), to: "draft".into() }],
};
let order = workflow.validate_dag()?; // topological order
# Ok::<(), tdw_agent::WorkflowValidationError>(())
```

## Example

```text
cargo run --example tdw_agent_basic -p tdw-agent
```

`examples/basic.rs` validates the sample agent card, validates a workflow DAG,
and emits the self-describing resource registry — all in-memory.

## Related crates

- `tdw-agent-store` — in-memory persistence + the live memory consolidator.
- `tdw-eval-runner` — runs evals against an agent through a `LanguageModel`.
