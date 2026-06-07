# tdw-tools

The tool registry, router, and permission-gated orchestrator: register typed
tools, route calls to them, and run them only when a permission policy allows.

`tdw-tools` is the in-process tool layer. A `ToolDefinition` describes a tool's
name, schemas, and permission pattern; a `RegisteredTool` pairs it with a handler
(`fn(Value) -> Result<Value>`). The `ToolOrchestrator` evaluates each call
against `tdw-hooks` `PermissionRules` and either runs it, defers it for approval,
or denies it.

## What it provides

- `ToolDefinition`, `RegisteredTool`, `ToolRegistry` (register/get/definitions).
- `ToolRouter` — name → registered tool.
- `ToolOrchestrator` — permission-gated `run(...)` returning `OrchestratorRunResult`.
- `validate_tool_definition`, `echo_tool()` (a contract-test fixture).
- `ToolError`, `ToolHandler` alias.

## Feature flags

None. Depends on `serde`, `serde_json`, `tdw-hooks`, and `tdw-protocol`.

## Quickstart

```rust
use serde_json::json;
use tdw_tools::{echo_tool, ToolOrchestrator, ToolRegistry};
use tdw_hooks::{PermissionEffect, PermissionRule, PermissionRules};
use tdw_protocol::ToolCallId;

let mut registry = ToolRegistry::default();
registry.register(echo_tool()).expect("register");

let mut permissions = PermissionRules::default();
permissions.push(PermissionRule::new(PermissionEffect::Allow, "tdw.echo", "tdw.echo"));
let orchestrator = ToolOrchestrator::new(registry, permissions);

let result = orchestrator
    .run(ToolCallId::new("call-1").expect("id"), "tdw.echo", json!({ "ok": true }))
    .expect("run");

assert_eq!(result.permission, PermissionEffect::Allow);
assert_eq!(result.output, Some(json!({ "ok": true })));
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-tools --example tdw_tools_basic
```

## Permission-gated orchestration

`ToolOrchestrator::run` routes the call, then evaluates the tool's
`permission_pattern` against the `PermissionRules`:

- `Allow` → runs the handler, returns the output.
- `Ask` → does **not** run; returns a deferred `PermissionId`
  (`permission-<tool-with-dashes>`) for the approval flow.
- `Deny` → `ToolError::PermissionDenied`.

`validate_tool_definition` enforces a safe dotted tool `name`, a non-empty
`description`, and a valid `permission_pattern` (`*`, an exact dotted name, or a
`prefix.*` wildcard). The registry rejects duplicate names and invalid
definitions on `register`.

## Invariants

- `#![forbid(unsafe_code)]`.
- **No tool runs without clearing the policy.** Permission evaluation precedes
  every handler invocation; `Ask` defers rather than runs.
- **Validated definitions.** Unsafe names / patterns are rejected at registration.
- **Deterministic registry** — tools held in a `BTreeMap`, ordered output.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
