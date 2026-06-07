# tdw-tool-exec

The tool-execution backend: resolve a registry tool's
`tdw-agent::ToolImplementation` binding and dispatch it to a concrete backend,
with a hardened, deny-by-default command path.

`tdw-tool-exec` is Phase 1 of the execution backend. Given a `tdw-agent`
`Registry` and a tool name, it resolves the tool's implementation and runs it.
`Builtin` handlers run in-process; `Command { background: false }` runs a
validated, allow-listed bare command with a timeout. Variants that need
credential wiring or server resolution (`Http`, `Mcp`, `Pty`, `Wasm`, `Ref`,
background `Command`) are honestly deferred with `NotYetSupported`.

## What it provides

- `ToolExecutor` — `new`, `with_builtin(name, handler)`,
  `with_command_policy(policy)`, `execute(registry, name, args)`.
- `CommandPolicy` — the `Command` allow-list + timeout (`new`, `from_env`,
  `default`).
- `ToolOutcome` — `{ structured: Value }`.
- `ExecError` — the resolution/execution error enum.

## Feature flags

None. Depends on `serde_json`, `tdw-agent`, `tdw-tools`, and `thiserror`.

## Configuration (env, via `CommandPolicy::from_env` / `default`)

| Variable | Meaning |
|----------|---------|
| `TDW_TOOL_EXEC_ALLOWED_COMMANDS` | Comma-separated bare command names permitted for `Command` execution. **Unset or empty = deny all command execution.** |
| `TDW_TOOL_EXEC_TIMEOUT_SECS` | Per-command wall-clock timeout in whole seconds (default 30; `0` is rejected and falls back to the default). |

## Quickstart (builtin handler)

```rust
use serde_json::{json, Value};
use tdw_tool_exec::ToolExecutor;
// build a tdw-agent Registry containing a `Builtin { handler }` tool named "demo.tool"...

fn echo(input: Value) -> tdw_tools::Result<Value> {
    Ok(json!({ "echoed": input }))
}

let executor = ToolExecutor::new()
    .with_builtin("demo.tool.handler", echo)
    .expect("register builtin");

// let outcome = executor.execute(&registry, "demo.tool", &json!({ "ping": true }))?;
// assert_eq!(outcome.structured["echoed"], json!({ "ping": true }));
```

See [`examples/basic.rs`](examples/basic.rs) for a complete, runnable version
that builds the registry:

```sh
cargo run -p tdw-tool-exec --example tdw_tool_exec_basic
```

## Hardened command execution

The `Command` path is deny-by-default and defends the argv boundary:

- the command must be a **bare name** — no paths, no `..`, no shell
  metacharacters, no control/whitespace (else `BadArguments`);
- it must be on the `CommandPolicy` allow-list (else `NotPermitted`); an unset/
  empty allow-list denies all execution;
- `args` come from the **tool definition**, not the request, so a caller cannot
  inject argv;
- output is captured on dedicated reader threads (no pipe-buffer deadlock) and
  the child is killed on timeout (`Backend("command timed out")`).

## Invariants

- `#![forbid(unsafe_code)]` — including in tests, which inject an explicit
  `CommandPolicy` rather than mutating process-global env (edition-2024
  `set_var` is `unsafe`).
- **Deny-by-default commands**; argv is registry-controlled, not request-controlled.
- **Honest deferral.** Unsupported variants return `NotYetSupported`, not a
  silent no-op; `Unbound` returns `Unbound` (the MCP `-32601` path).
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
