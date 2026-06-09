# tdw-event

The event envelope and identity plane: actor, origin, trace context, and a
typed, validated `EventEnvelope<P>` with causation/correlation/depth tracking,
plus JSON-schema export of the whole bundle.

Every event that flows through the platform carries the same envelope so that
*who* (actor), *from where* (origin), and *as part of which trace* (trace
context) are uniform and auditable. Child events derived from a parent
automatically inherit identity and record causation.

## What it provides

- `Actor` (`actor_id`, `kind`, `tenant_id`) and `ActorKind`
  (`User`/`Service`/`Worker`/`Agent`/`System`).
- `Origin` (`service`, `entrypoint`, `host`).
- `TraceContext` (`trace_id`, `span_id`, `parent_span_id`) with `child(...)`.
- `EventEnvelope<P>` — the generic envelope, with `new`, `child_event`, and
  `validator::Validate` derived field constraints.
- `EventSchemaRef` and `event_schema_bundle()` — JSON-schema export of all five
  named schemas (`EVENT_SCHEMA_NAMES`).
- `sample_actor_context(...)` / `sample_event(...)` — fixtures for tests/examples.

## Feature flags

None. Depends on `schemars`, `serde`, `serde_json`, and `validator`.

## Quickstart

```rust
use serde_json::json;
use tdw_event::{sample_event};
use validator::Validate;

let event = sample_event("mcp");          // an EventEnvelope<Value>
assert!(event.validate().is_ok());        // field constraints hold

let child = event.child_event("hook.audit", json!({ "ok": true }));
assert_eq!(child.causation_id.as_deref(), Some(event.event_id.as_str()));
assert_eq!(child.depth, 1);
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-event --example tdw_event_basic
```

## Identity and lineage

- `EventEnvelope::new` derives a deterministic `event_id` of
  `"{trace_id}:{span_id}:{event_type}"`, stamps `schema_version = "1.0.0"`, and
  starts `depth = 0` with no causation/correlation.
- `child_event` produces a child on a `TraceContext::child` span, sets
  `causation_id` to the parent's `event_id`, propagates (or seeds)
  `correlation_id`, and increments `depth` (saturating at `u8::MAX`).
- All string fields carry `#[validate(length(min = 1))]`, so `Validate::validate`
  rejects an envelope with empty identity fields.

## Schema export

`event_schema_bundle()` returns a `BTreeMap<&'static str, Value>` of the JSON
schema for `Actor`, `Origin`, `TraceContext`, `EventEnvelope<Value>`, and
`EventSchemaRef`. `EVENT_SCHEMA_NAMES` lists the five keys, used by the
schema-check tooling to detect drift.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Non-empty identity.** Validation requires non-empty actor/origin/trace and
  envelope fields.
- **Lineage is automatic.** Children always carry causation + incremented depth;
  depth increment saturates rather than wraps.
- **Deterministic schema bundle** — ordered `BTreeMap`, stable keys.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
