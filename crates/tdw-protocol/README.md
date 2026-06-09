# tdw-protocol

Wire-protocol vocabulary for the TDW daemon request path. This crate defines the
serializable types every other service crate speaks: the operations a client
submits (`Op`), the events the daemon emits (`EventMsg`), the envelope that
carries an op (`OpEnvelope`), the replay frame that records an event
(`ReplayFrame`), and the typed identifiers (`SessionId`, `OpId`, `PlanId`,
`PermissionId`, `ToolCallId`).

It is a pure data crate: `#![forbid(unsafe_code)]`, no I/O, no async, no
transport. Producers and consumers (`tdw-app-server`, `tdw-app-client`,
`tdw-service-api`, `tdw-cli`, `tdw-worker`, `tdw-mcp`, `tdw-tui`) depend on it to
agree on the same JSON shapes.

## Binaries produced

None. This is a library crate.

## Feature flags

None.

## Key environment variables

None directly. The protocol's size/replay limits are configured by
`tdw-config`'s `ProtocolConfig` (`max_event_bytes`, `replay_enabled`); see
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the `TDW_*` reference
that drives those values.

## Quickstart (library)

Build an op envelope and a matching event, then round-trip them through JSON
exactly as the transport does:

```rust
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

let envelope = OpEnvelope::new(
    SessionId::new("session-1")?,
    1,
    ActorRef { actor_id: "user:cli".into(), kind: ActorKind::User, tenant_id: None },
    Op::RunQuery { sql: "select 1".into(), plan_id: None, cost_hint: None },
);

let json = serde_json::to_string(&envelope)?;        // what the client writes
let decoded: OpEnvelope = serde_json::from_str(&json)?; // what the daemon reads
assert_eq!(decoded.op_id, envelope.op_id);

let event = EventMsg::Completed { op_id: envelope.op_id, summary: None, result: None };
assert!(matches!(event, EventMsg::Completed { .. }));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`tdw_protocol::schema_bundle()` exports the JSON Schemas for `op`, `event_msg`,
`op_envelope`, and `replay_frame` for tooling/codegen.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-protocol --example tdw_protocol_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — type map and the op/event flow.
- `tdw-app-server` / `tdw-app-client` — the transports that frame these types.
