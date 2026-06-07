# tdw-event architecture

A single-module crate (`src/lib.rs`) defining the event identity plane and the
generic, validated event envelope.

## Module map

| Item | Role |
|------|------|
| `ActorKind`, `Actor` | The acting identity (`actor_id`, `kind`, `tenant_id`). |
| `Origin` | Where the event originated (`service`, `entrypoint`, `host`). |
| `TraceContext` | Distributed-trace coordinates (`trace_id`, `span_id`, `parent_span_id`) + `child`. |
| `EventEnvelope<P>` | The generic envelope; `new`, `child_event`. |
| `EventSchemaRef` | A reference triple (`event_type`, `schema_name`, `schema_version`). |
| `EVENT_SCHEMA_NAMES` | The five bundle keys. |
| `event_schema_bundle()` | JSON-schema export of all five types. |
| `sample_actor_context` / `sample_event` | Deterministic fixtures. |
| `schema_json::<T>()` (private) | `schemars` -> `serde_json::Value`. |

## Core contracts

### Identity plane

`Actor`, `Origin`, and `TraceContext` are the three identity axes every event
carries. Each derives `Serialize`, `Deserialize`, `JsonSchema`, and
`validator::Validate`, with `#[validate(length(min = 1))]` on every required
string — so an envelope with an empty `actor_id`, `service`, `entrypoint`,
`trace_id`, or `span_id` fails validation.

`TraceContext::child(span_id)` keeps the same `trace_id`, sets a new `span_id`,
and records the previous span as `parent_span_id` — the primitive that makes
lineage automatic.

### `EventEnvelope<P>`

Generic over the payload `P`, the envelope holds identity (`actor`, `origin`,
`trace`), event metadata (`event_id`, `event_type`, `schema_version`,
`occurred_at`), the `payload`, and lineage fields (`causation_id`,
`correlation_id`, `depth`).

- **`new`** computes a deterministic `event_id = "{trace_id}:{span_id}:{event_type}"`,
  defaults `schema_version` to `"1.0.0"`, and initializes `depth = 0` with no
  causation/correlation.
- **`child_event<Q>`** creates a child envelope on a `trace.child(...)` span,
  sets `causation_id = parent.event_id`, propagates the parent's
  `correlation_id` (or seeds it from the parent `event_id`), and sets
  `depth = parent.depth.saturating_add(1)`. The saturating add means a runaway
  chain caps at `u8::MAX` rather than wrapping to 0 — depth is a guard, not a
  silent overflow.

### Schema bundle

`event_schema_bundle()` materializes the JSON schema for the five named types
into an ordered `BTreeMap`. Paired with `EVENT_SCHEMA_NAMES`, this is what the
workspace's `events schema-check` tooling diffs against the committed schemas to
catch accidental wire-format drift. `EventEnvelope<Value>` is the bundled
envelope shape (payload typed as arbitrary JSON).

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Non-empty identity** enforced by `validator` field constraints.
- **Automatic, bounded lineage** — children always carry causation and a
  saturating depth.
- **Deterministic `event_id`** and **ordered schema bundle** for reproducibility.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
