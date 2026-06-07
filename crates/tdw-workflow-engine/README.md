# tdw-workflow-engine

Compiles a validated `tdw-agent` `WorkflowDefinition` (a node/edge DAG) into an
ordered `ExecutionPlan` — the deterministic node execution order a runner follows.

## Purpose

`WorkflowEngine::compile` is the bridge between the *declared* workflow (nodes,
edges, metadata) and an *executable* plan:

- it delegates structural validation and topological ordering to
  `tdw_agent::validate_workflow_contract` (identifier checks, edge integrity,
  acyclic ordering);
- on success it returns an `ExecutionPlan { workflow_id, ordered_node_ids }` whose
  `ordered_node_ids` are the nodes in dependency-respecting order;
- on failure it propagates the `tdw_agent::AgentContractError` unchanged.

The crate is a thin, pure adapter: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `ExecutionPlan` (de)serialization.
- `tdw-agent` — `WorkflowDefinition`, `validate_workflow_contract`, and the error
  type; also the node/edge/metadata types used to build a workflow.

## Quickstart

```rust
use tdw_workflow_engine::WorkflowEngine;
// WorkflowDefinition, WorkflowNode, WorkflowEdge and the EntityMeta builders
// come from `tdw_agent`.

let plan = WorkflowEngine::compile(&workflow)?; // workflow: tdw_agent::WorkflowDefinition
assert_eq!(plan.ordered_node_ids, vec!["retrieve", "draft"]);
# Ok::<(), tdw_agent::AgentContractError>(())
```

See `examples/basic.rs` for a full, runnable construction of a `WorkflowDefinition`.

Run the worked example:

```text
cargo run -p tdw-workflow-engine --example tdw-workflow-engine-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — the execution-model and compile contract.
- `tdw-agent` — the workflow contract and validation this crate compiles.
