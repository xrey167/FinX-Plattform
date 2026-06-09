# tdw-protocol architecture

`tdw-protocol` is the shared vocabulary at the center of the daemon request path.
Every service crate serializes/deserializes these exact types so a client, the
daemon, a worker, and a replay file all agree byte-for-byte.

## Module map

A single `src/lib.rs`. There are no submodules; the surface is small enough to
live in one file.

## Key types

### Typed identifiers (newtype wrappers over `String`)

| Type | Construction | Notes |
| --- | --- | --- |
| `SessionId` | `SessionId::new(s)` (rejects empty), `SessionId::generated()` | `session-<ULID>` when generated |
| `OpId` | `OpId::generated()` | `UUIDv7` — time-ordered |
| `PlanId` | `PlanId::new(s)` (rejects empty) | optional query plan id |
| `PermissionId` | `PermissionId::new(s)` (rejects empty) | approval correlation |
| `ToolCallId` | `ToolCallId::new(s)` (rejects empty) | tool-call correlation |

Empty-string construction returns `ProtocolError::EmptyField { field }`. All ids
are `serde(transparent)` so they serialize as bare strings.

### Actors

- `ActorKind` — `User | Service | Worker | Agent | System`.
- `ActorRef` — `{ actor_id, kind, tenant_id }`: who submitted an op.

### Operations (`Op`) — client → daemon

`#[serde(tag = "type", rename_all = "snake_case")]`, so each variant serializes
with a `"type"` discriminator. Variants:

- `AppendUserMessage { message }`
- `RunQuery { sql, plan_id?, cost_hint? }`
- `IngestBatch { provider, endpoint, symbols, range? }`
- `ToolCall { call_id, tool_name, arguments, permission_id? }`
- `ApprovalResponse { permission_id, decision, reason? }`
- `CompactContext { target_tokens }`
- `Cancel { op_id }`
- `StreamStart { provider, symbol, table? }` (`stream_start`)
- `StreamStop { stream_id }` (`stream_stop`)
- `Shutdown`

Supporting payloads: `TimeRange { start, end }`, `CostHint { backend,
estimated_bytes_scanned?, estimated_rows_read? }`, `ApprovalDecision`
(`AllowOnce | AlwaysAllow | Deny`).

### Events (`EventMsg`) — daemon → client

Also `#[serde(tag = "type")]`. Variants: `Started`, `Progress`,
`ApprovalRequested`, `ToolCallRequested`, `ToolCallCompleted`, `OutputChunk`,
`DomainEvent`, `Completed`, `Failed`, `Cancelled`. `OutputStream`
(`Stdout | Stderr | Model | System`) tags `OutputChunk`.

The terminal events are `Completed`, `Failed`, and `Cancelled`; a transport
reader stops once it sees the terminal event whose `op_id` matches the submitted
op (see `tdw-app-client`).

### Envelope and replay

- `OpEnvelope { op_id, session_id, sequence, submitted_by, op }` — the wire unit
  the client frames and the daemon dispatches. `OpEnvelope::new(...)` generates a
  fresh `OpId`.
- `ReplayFrame { session_id, sequence, event }` — one persisted event for
  replay/audit (`tdw-rollout`, `tdw-replay`, `tdw-session` consume it).

## Runtime flow

```text
client builds OpEnvelope ──serialize──▶ transport frame
                                          │
                          daemon decode ◀─┘
                          dispatch ──▶ Vec<EventMsg> (Started … terminal)
                                          │
                  client decode ◀──frame──┘  (stop at matching terminal)
                                          │
                  persist as ReplayFrame ─┘  (rollout / replay)
```

## Schema export

`schema_bundle()` returns a `BTreeMap<&'static str, Value>` of JSON Schemas for
`op`, `event_msg`, `op_envelope`, and `replay_frame` (via `schemars`). Used by
codegen and contract tooling. The crate is `JsonSchema`-deriving throughout.

## Security posture

No I/O and no trust decisions live here — it is a data vocabulary. Validation is
limited to non-empty id construction. Authorization, masking, and OIDC live in
`tdw-service-api` / `tdw-auth*`. The `__fuzz_protocol_json` shim (hidden) feeds
arbitrary bytes through the three wire decoders and must never panic; it backs a
nightly cargo-fuzz target.

## Integration points

- `tdw-app-server` re-frames `OpEnvelope`/`EventMsg` over TCP/UDS/HTTP-SSE.
- `tdw-app-client` writes envelopes and reads terminal events.
- `tdw-service-api` dispatches `Op` and emits `EventMsg`.
- `tdw-tui` renders `EventMsg` as terminal lines.
- `tdw-worker` carries `OpEnvelope` as a job payload.
