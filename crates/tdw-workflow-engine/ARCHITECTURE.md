# tdw-workflow-engine — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `ExecutionPlan` | `{ workflow_id, ordered_node_ids: Vec<String> }`. |
| `WorkflowEngine` | Zero-sized unit struct; namespaces `compile`. |
| `WorkflowEngine::compile` | `&WorkflowDefinition → Result<ExecutionPlan, AgentContractError>`. |

## Key types and traits

- `ExecutionPlan` derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.
- `WorkflowEngine` derives `Clone, Debug, Default`; it holds no state — `compile`
  is effectively an associated function.
- Inputs/errors are owned by `tdw-agent`: `WorkflowDefinition` (with `meta`,
  `nodes`, `edges`), `validate_workflow_contract`, and `AgentContractError`.

## Execution model

```
WorkflowDefinition { meta, nodes, edges }
            │
            │ WorkflowEngine::compile(&workflow)
            ▼
validate_workflow_contract(workflow)        (in tdw-agent)
   • identifier grammar (workflow name/id, node ids)
   • edge endpoints reference real nodes
   • DAG is acyclic; produce a topological node order
            │ Ok(ordered_node_ids)            │ Err(AgentContractError)
            ▼                                  ▼
ExecutionPlan {                          propagate unchanged
   workflow_id: meta.id.clone(),
   ordered_node_ids,
}
```

The engine does **not** execute nodes; it produces the *order* a downstream runner
should execute them in. All correctness rules (identifier safety, edge integrity,
acyclicity, ordering) live in `tdw_agent::validate_workflow_contract`; this crate's
sole job is to wrap that result with the workflow id into a serializable plan.

## Invariants

- `ordered_node_ids` is a topological order of the workflow's nodes — for an edge
  `from → to`, `from` precedes `to`.
- `plan.workflow_id == workflow.meta.id`.
- Compilation is **all-or-nothing**: an invalid workflow yields an
  `AgentContractError` (e.g. `InvalidIdentifier("workflow_name", …)` for a name
  like `../research`) and no partial plan.
- Pure and deterministic: same workflow in, same plan out; no I/O, no state.
