# tdw-hooks architecture

A single-module crate (`src/lib.rs`) implementing the hook lifecycle engine and
the shared permission primitives.

## Module map

| Group | Items |
|-------|-------|
| Hook definition | `HookEvent`, `HandlerKind`, `TransactionMode`, `AdditionalContext`, `HookSpec` (+ builder), `validate_hook_spec` |
| Registry | `HookRegistry` (`register`, `disable`, `execute`, `execute_runtime`, `execute_handlers`, `hook_names`) |
| Run outcomes | `HookOutcome`, `HookRuntimeOutcome`, `HookExecutionOutcome` |
| Execution policy | `HookExecutionPolicy`, and the handler-execution backend trait |
| Backend | `HookHandlerBackend` (trait), `SystemHookHandlerBackend`, `McpHookHandler` alias |
| Permissions | `PermissionEffect`, `PermissionRule`, `PermissionRules` |
| Approvals | `DeferredApproval`, `DeferredApprovals` |
| Errors | `HookError` |
| Embedded asset | `TOOL_PROMPT_TEXT` / `tool_prompt_text()` (`include_str!("tool_prompt.txt")`) |

## Core contracts

### `HookSpec` and the registry

A `HookSpec` binds a `name` to an `event`, a `handler` (`HandlerKind`), an
integer `order`, a `TransactionMode`, a `max_depth` recursion budget, a
`should_stop` veto flag, and any `additional_contexts`. `HookSpec::new` defaults
the event to `Custom(name)` and the handler to `Command { command: name }`; the
`for_event` / `with_handler` / `with_context` / `should_stop` / `disabled`
builders refine it.

`HookRegistry::register` keeps the hook vector **sorted by `(order, name)`**, so
execution order is deterministic regardless of insertion order.

### Policy-gated handler execution

This is the central contract. `execute_handlers` walks the enabled, ordered
hooks and, for each one:

1. derives a stable **action string** from the handler via `hook_action`
   (`hook.command.<cmd>`, `hook.http.<host>`, `hook.mcp.<server>.<tool>`,
   `hook.prompt.<name>`, `hook.agent.<agent>.<skill>`);
2. evaluates that action against `policy.permissions`:
   - `Allow` → proceed,
   - `Ask` → return `HookError::PermissionRequiresApproval(action)`,
   - `Deny` → return `HookError::PermissionDenied(action)`;
3. if the hook has `should_stop` and the policy does **not**
   `allow_handler_vetoes`, return `HookError::VetoDenied`;
4. build the handler payload (the hook descriptor + the event envelope) and
   dispatch it to the matching `HookHandlerBackend` method.

The default `HookExecutionPolicy` is **deny-by-default**
(`PermissionRules { default_permission: Deny, .. }` and
`allow_handler_vetoes: false`), so a hook only runs when the policy explicitly
allows its action. (Note: a free-standing `PermissionRules::default()` instead
defaults to `Ask` — the *policy* default is the stricter one.)

`execute_runtime` performs steps that don't need a backend (validation,
recursion/depth guards, resolving the runtime outcome); `execute` is a thin
projection of that to the lightweight `HookOutcome`.

### Recursion and depth guards

Inside `execute_runtime`, each hook is `validate_hook_spec`-checked, then:

- if `envelope.depth >= hook.max_depth` → `HookError::DepthExceeded`;
- if `envelope.event_type == hook.name` **or** the hook name is already on the
  registry's `active` set → `HookError::RecursionGuard`.

The hook name is inserted into `active` for the duration of producing its outcome
and removed after, so a hook cannot transitively re-enter itself.

### `HookHandlerBackend` / `SystemHookHandlerBackend`

The backend trait has one method per `HandlerKind`: `run_command`, `call_http`,
`call_mcp`, `load_prompt`, `run_agent`, each returning `Result<Value, HookError>`.
`SystemHookHandlerBackend` is the production implementation: it dispatches MCP
handlers from an in-process `BTreeMap<(server, tool), McpHookHandler>` registered
via `register_mcp_tool`, and performs `Http` handlers with a blocking `reqwest`
client (optional bearer token, a 5s timeout, and a 64 KiB capped response body).

### Permissions and approvals

`PermissionRules::evaluate(action)` first rejects any action that is not a valid
action name (`[A-Za-z0-9._-]+`) with `Deny`, then returns the **last** matching
rule's effect (last-wins), falling back to `default_permission`. Patterns are
exact, `*` (all), or `prefix*`.

`DeferredApprovals` tracks `Ask` decisions: `request(action, pattern)` mints a
sequential `PermissionId` and stores a pending `DeferredApproval`; `resolve`
removes it and records the `ApprovalDecision`; `pending_count` reports the
backlog. This is the same permission vocabulary `tdw-tools` and `tdw-mask` build
on.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy lints); every fallible
  path returns `HookError`.
- **Policy-gated.** Handlers never run without clearing the permission policy;
  the default policy denies.
- **Deterministic, guarded execution.** Sorted by `(order, name)`; recursion and
  depth are bounded.
- **Bounded HTTP.** The system backend caps the response body and applies a
  timeout, so a hostile endpoint cannot exhaust memory or hang the engine.
- **Clean-room.** No vendor-derived code or branding.
