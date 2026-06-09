# tdw-hooks

The hook engine: ordered, recursion-guarded, **policy-gated** handlers that fire
on lifecycle events, plus the permission-rule and deferred-approval primitives
the rest of the platform shares.

A hook is a named handler bound to a `HookEvent` (session start, pre/post tool
call, pre/post UDF run, pre-response, stop, or a custom event). The registry runs
the matching hooks in `order`, guarding against recursion and excessive depth.
When you run hooks *with handlers*, every hook must first clear a
`HookExecutionPolicy` (a permission table) before its backend is invoked.

## What it provides

- `HookSpec` (+ builder) and `HookRegistry` — define and run hooks.
- `HookEvent`, `HandlerKind` (`Command` / `Http` / `Mcp` / `Prompt` / `Agent`),
  `TransactionMode`, `AdditionalContext`.
- `HookHandlerBackend` — the trait that actually executes a handler;
  `SystemHookHandlerBackend` is the built-in implementation.
- `PermissionRules` / `PermissionRule` / `PermissionEffect` — the
  `Allow`/`Ask`/`Deny` policy engine (also re-used by `tdw-tools` and `tdw-mask`).
- `DeferredApprovals` / `DeferredApproval` — pending `Ask` approvals keyed by
  `PermissionId`.
- `validate_hook_spec` and the run outcomes
  (`HookOutcome` / `HookRuntimeOutcome` / `HookExecutionOutcome`).

## Feature flags

None. `reqwest` (blocking) is a hard dependency used by `SystemHookHandlerBackend`
for `Http` handlers.

## Quickstart

Run a hook's handler through a permission policy:

```rust
use serde_json::json;
use tdw_event::sample_event;
use tdw_hooks::{
    HookRegistry, HookSpec, HookExecutionPolicy, PermissionEffect, PermissionRule,
    PermissionRules, TransactionMode,
};

let mut registry = HookRegistry::default();
registry.register(HookSpec::new("audit.log", 10, TransactionMode::PostCommit));

let mut permissions = PermissionRules::default();             // default = Ask
// A `HookSpec::new` hook uses a `Command` handler named after the hook, so its
// action is `hook.command.<name>`; the `hook.*` pattern allows it.
permissions.push(PermissionRule::new(PermissionEffect::Allow, "hook.*", "allow-hooks"));
let policy = HookExecutionPolicy { permissions, allow_handler_vetoes: false };

let envelope = sample_event("service");
// `backend` implements HookHandlerBackend (e.g. SystemHookHandlerBackend::new()).
// let outcomes = registry.execute_handlers(&envelope, &policy, &mut backend)?;
```

See [`examples/basic.rs`](examples/basic.rs) for a complete, runnable version
with a tiny in-example backend:

```sh
cargo run -p tdw-hooks --example tdw_hooks_basic
```

## Three execution modes

| Method | Returns | Use |
|--------|---------|-----|
| `execute` | `Vec<HookOutcome>` | Lightweight: which hooks fired, their transaction mode and emitted event type. |
| `execute_runtime` | `Vec<HookRuntimeOutcome>` | Adds the resolved event/handler/contexts (no handler invocation). |
| `execute_handlers` | `Vec<HookExecutionOutcome>` | Full path: permission-checks each hook, optionally honors `should_stop` vetoes, then invokes the backend. |

## Invariants

- `#![forbid(unsafe_code)]`.
- **Policy-gated execution.** `execute_handlers` evaluates each hook's action
  against the policy: `Allow` runs, `Ask` → `PermissionRequiresApproval`, `Deny`
  → `PermissionDenied`. The default `HookExecutionPolicy` denies by default.
- **Recursion + depth guards.** A hook whose name equals the event type, or that
  is already on the active set, is a `RecursionGuard` error; exceeding
  `max_depth` is `DepthExceeded`.
- **Deterministic order.** Hooks run sorted by `(order, name)`.
- **Veto opt-in.** A `should_stop` hook is rejected (`VetoDenied`) unless the
  policy sets `allow_handler_vetoes`.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
