# tdw-actor

Actor context propagation across task hops: carry the caller's identity, origin,
and trace into spawned work without losing the identity plane.

`tdw-actor` builds on `tdw-event`'s `Actor` / `Origin` / `TraceContext`. An
`ActorContext` bundles those three; when work fans out into a child task, the
context produces an `ActorTaskContext` that preserves the actor and origin while
opening a fresh child trace span — so a spawned task stays attributable to whoever
initiated it.

## What it provides

- `ActorContext` — `{ actor, origin, trace }`, with `new`, `service(entrypoint)`,
  and `child_task(task_name)`.
- `ActorTaskContext` — the per-task context (adds `task_name`).
- `OmcSpawn::capture(context, task_name)` — the spawn seam that captures a child
  task context from a parent.

## Feature flags

None. Depends on `serde` and `tdw-event`.

## Quickstart

```rust
use tdw_actor::{ActorContext, OmcSpawn};

let context = ActorContext::service("worker"); // system actor, "worker" entrypoint
let task = OmcSpawn::capture(&context, "fetch");

assert_eq!(task.actor, context.actor);                 // identity preserved
assert_eq!(task.origin.entrypoint, "worker");          // origin preserved
assert_eq!(task.trace.trace_id, context.trace.trace_id); // same trace
assert_eq!(task.trace.parent_span_id.as_deref(), Some("root")); // child span
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-actor --example tdw_actor_basic
```

## Propagation contract

- `ActorContext::service(entrypoint)` is the convenience constructor for
  service-initiated work: a `System` actor (`system:tdw`, tenant `default`), an
  origin of `tdw-service` at the given entrypoint, and a root trace
  (`trace-{entrypoint}` / `root`).
- `ActorContext::new(actor, origin, trace)` preserves an arbitrary caller's
  identity plane verbatim (e.g. an `Agent` actor on a worker host).
- `child_task(task_name)` / `OmcSpawn::capture` keep the actor and origin and
  call `TraceContext::child("task-{task_name}")`, so the parent span becomes the
  child's `parent_span_id` while the `trace_id` is unchanged.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Identity never widens on a hop.** A spawned task carries the *same* actor and
  origin as its parent; only the trace span advances.
- **Trace continuity.** The `trace_id` is preserved across hops; the new span
  records its parent.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
