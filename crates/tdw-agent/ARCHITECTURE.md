# tdw-agent — Architecture

## Module map

| Module | Role |
| --- | --- |
| `lib.rs` | The spec types (one per `EntityKind`), their `RegistryEntity` impls, `resource_definitions`/`schema_bundle`, manifest + invocation parsing, contract validation, and `WorkflowDefinition::validate_dag`. |
| `base` | `EntityMeta` / `BaseMetadata` / `Origin` / `Tier` / `Source` / `Adaptivity` / `Retention` / `Reference` / `ToolEffect` / `ToolImplementation`; the adaptivity-for-feedback gate. |
| `kind` | The `EntityKind` enum (`ALL`), `Group`, and the `is_data_kind` / `is_autonomy_capable` classifiers. |
| `facets` | `DataFacets` / `EvalFacets` and the plane/materialization/validation enums. |
| `resource` | `Resource` / `ResourceDefinition` / `RegistryEntity` + `entity_from_resource`; the `tdw.dev` API group/version constants. |
| `loader` | `load_resource` / `load_typed` (JSON5 → typed entity). |
| `registry` | `Registry` — an in-memory typed registry over resources. |
| `watch` | `RegistryWatcher` — filesystem change watching (`notify`). |
| `consolidate` | The pure `consolidation_plan` planner (`ConsolidationAction`). |
| `mcp` | Projection of canonical entities to MCP (`project_to_mcp`, `McpTool`, `McpPrompt`, annotations). |

## Trait contract: `RegistryEntity`

Every spec type implements:

```rust
pub trait RegistryEntity {
    const KIND: EntityKind;
    fn metadata(&self) -> &EntityMeta;
}
```

This binds a Rust spec type to its taxonomy `EntityKind` and exposes its
`EntityMeta`, which is what the registry, loader, and MCP projection key off.

## The taxonomy + self-describing registry

`EntityKind::ALL` is the closed set of kinds. `resource_definitions()` walks it
and, for each kind, emits a `ResourceDefinition { group, kind, manifest_group,
spec_schema, has_data_facets, autonomy_capable }`. `spec_schema_for(kind)`
returns the kind's JSON Schema (via `schemars`) when a concrete Rust type backs
it — including `ResourceDefinition`'s own schema, which makes the registry
self-describing.

Deprecated pre-taxonomy helpers (`AGENT_SCHEMA_NAMES`, `schema_bundle`,
`agent_storage_mappings`, `StorageMapping`) are retained only because downstream
crates still consume them; new code should prefer `resource_definitions`.

## Workflow DAG validation

`WorkflowDefinition::validate_dag` runs Kahn's algorithm over the nodes/edges:

- duplicate `node_id` → `DuplicateNode`,
- an edge referencing an unknown node → `MissingNode`,
- a cycle (fewer ordered nodes than total) → `Cycle`,
- otherwise returns a topological order of node ids.

`validate_workflow_contract` layers identifier/field hygiene on top before calling
`validate_dag`.

## Eval feedback gate

`AgentSkill::apply_eval_feedback(pass_rate, now, disable_below)` is gated by the
`Adaptivity` axis: only a skill with `adaptivity >= Adaptivity::Learning` accrues
`SkillQuality`. A `None`/`Configured` skill returns `AdaptivityError::NotLearning`
and is left untouched. `now` is injected (RFC 3339) so the mutation is
deterministic.

## Offline test design

All tests are in-memory and deterministic: schema/projection round-trips, manifest
parsing, contract validation, and DAG validation. The crate reads JSON5 resources
and watches the filesystem, but its unit tests use literals and temp data — there
is no network anywhere in `tdw-agent`.
