# tdw-tools architecture

A single-module crate (`src/lib.rs`) implementing the in-process tool layer:
registry → router → permission-gated orchestrator.

## Module map

| Item | Role |
|------|------|
| `ToolHandler` | `fn(Value) -> Result<Value>` — the handler signature. |
| `ToolDefinition` | `{ name, description, input_schema, output_schema, permission_pattern }`. |
| `RegisteredTool` | A definition + its handler; `call(input)`. |
| `ToolRegistry` | `register` / `get` / `definitions`; dedup + validation. |
| `ToolRouter` | `route(name) -> &RegisteredTool`. |
| `ToolOrchestrator` | `run(call_id, name, args)` — the permission gate. |
| `OrchestratorRunResult` | `{ call_id, tool_name, permission, output, deferred_permission }`. |
| `validate_tool_definition` | Name / description / pattern guard. |
| `echo_tool()` | A contract-test fixture tool (`tdw.echo`). |
| `ToolError` | `DuplicateTool` / `UnknownTool` / `InvalidDefinition` / `PermissionDenied`. |

## Layered contract

The three layers separate concerns deliberately:

1. **`ToolRegistry`** owns *what exists*. `register` runs
   `validate_tool_definition` and rejects duplicate names, so the registry is
   always a set of valid, uniquely-named tools held in a `BTreeMap` (ordered,
   deterministic `definitions()`).
2. **`ToolRouter`** owns *resolution*. `route(name)` returns the registered tool
   or `ToolError::UnknownTool`. It is a thin, read-only view over a registry.
3. **`ToolOrchestrator`** owns *authorization + invocation*. It wraps a router
   and a `tdw-hooks` `PermissionRules`.

### The permission gate

`ToolOrchestrator::run` is the security-critical path:

```
route(name)
  -> permissions.evaluate(tool.permission_pattern)
       Allow -> output = Some(tool.call(args)); deferred = None
       Ask   -> output = None; deferred = Some(PermissionId "permission-<name-dashed>")
       Deny  -> Err(PermissionDenied)
```

The result is uniform (`OrchestratorRunResult`) regardless of branch, carrying
the evaluated `permission` so a caller can see *why* it got the outcome it did.
`Ask` is a first-class outcome — the call is **not** executed; instead a stable
`PermissionId` is minted for the deferred-approval flow (mirroring
`tdw-hooks::DeferredApprovals`).

### Definition validation

`validate_tool_definition` requires:

- `name` — a dotted tool name (`[A-Za-z0-9_-]` segments separated by `.`,
  rejecting e.g. `../tdw.echo`);
- `description` — non-empty;
- `permission_pattern` — `*`, an exact dotted name, or a `prefix.*` wildcard.

This runs at `register` time, so an invalid tool can never enter the registry,
and therefore can never be routed or orchestrated.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Authorization precedes invocation** — no handler runs before the permission
  check; `Ask` defers, `Deny` errors.
- **Validated, unique tools** — bad names/patterns and duplicates are rejected at
  registration.
- **Deterministic** — `BTreeMap`-backed registry.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
