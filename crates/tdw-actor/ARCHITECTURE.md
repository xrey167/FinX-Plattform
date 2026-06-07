# tdw-actor architecture

A single-module crate (`src/lib.rs`) that layers task-hop context propagation on
top of `tdw-event`'s identity types.

## Module map

| Item | Role |
|------|------|
| `ActorContext` | Bundles `Actor` + `Origin` + `TraceContext`. |
| `ActorTaskContext` | A child-task context: the same three plus `task_name`. |
| `OmcSpawn` | The spawn seam: `capture(context, task_name) -> ActorTaskContext`. |

## Core contracts

### `ActorContext`

The platform's "ambient identity" for a unit of work. It re-exports `tdw-event`'s
`Actor`, `Origin`, and `TraceContext` rather than redefining them, so the same
identity plane validated and serialized by `tdw-event` is what travels here.

Two constructors:

- **`new(actor, origin, trace)`** — preserves an arbitrary caller's identity
  exactly. Used when the caller already has a fully-formed identity (e.g. an
  agent loop on a named worker host) that must not be flattened.
- **`service(entrypoint)`** — the convenience path for service-initiated work:
  a `System` actor `system:tdw` (tenant `default`), an origin of `tdw-service`
  at `entrypoint`, and a root trace (`trace-{entrypoint}` / span `root`).

### Task hops

`child_task(task_name)` is the propagation primitive. It constructs an
`ActorTaskContext` that:

- **copies** `actor` and `origin` unchanged (identity does not widen on a hop), and
- **advances** the trace via `TraceContext::child("task-{task_name}")`, so the
  parent span becomes the child's `parent_span_id` while the `trace_id` stays the
  same.

`OmcSpawn::capture(context, task_name)` is a thin wrapper over `child_task`,
naming the moment a parent context is captured into a spawned task. Keeping it as
an explicit seam means every fan-out point goes through one auditable call.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Identity preservation.** A hop never grants a task a different actor or origin
  than its parent — only the trace span changes.
- **Trace continuity.** `trace_id` is constant across hops; each new span records
  its parent, so a full causal span tree is reconstructable.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
