# tdw-app-server architecture

`tdw-app-server` is the daemon's message plumbing: the in-process submission/event
channel, the dispatch/sink traits, the durable service loop, the outbox→bus relay,
the `serve` lifecycle, and the feature-gated network transports.

## Module map

| Path | Compiled when | Contents |
| --- | --- | --- |
| `src/lib.rs` | always | endpoints, channels, traits, `ServiceLoop`, relay, `serve` |
| `src/transport_tcp.rs` | `transport-tcp`, not `loom` | `serve_tcp` |
| `src/transport_uds.rs` | `unix` + `transport-uds`, not `loom` | `serve_uds` |
| `src/transport_http.rs` | `transport-http`, not `loom` | `serve_http` (SSE broadcast) |

The transport modules are excluded under `--cfg loom` (the loom model in
`tests/loom_relay.rs` exercises the relay locking, not `tokio::net`).

## Key types and traits

### Endpoints

- `DaemonTransport` — re-exported from `tdw-config` (`Tcp | Uds | HttpSse`).
- `DaemonEndpoint { transport, address }` with `validate()` →
  `validate_endpoint`: per-transport address checks (valid `SocketAddr` for TCP;
  no `://`, `;`, `|`, `&`, or `..` parent segments for UDS; `http(s)://host…`
  shape for HTTP/SSE). `EndpointError` enumerates the failure causes.
- `transport_label(DaemonTransport) -> &'static str`.

### Submission + events

- `SubmissionHandle { submit(OpEnvelope) }` — the cloneable sender side.
- `SubmissionError::into_envelope()` recovers the envelope on a closed channel.
- `channel() -> (SubmissionHandle, mpsc::UnboundedReceiver<EventMsg>, AgentLoop)`
  — the simple path; `AgentLoop::run_once` emits a `Started` per submission.

### Dispatch / persistence traits

- `Dispatcher` (`async fn dispatch(&self, OpEnvelope) -> Vec<EventMsg>`) — the
  handler seam; implementations emit `Started` then a terminal event in order.
- `EventSink` (`persist_event`, `record_cost`) — durable persistence per event +
  a per-op cost record; `SinkError`/`SinkResult` carry failures.
- `ServiceLoop<D, S>` pairs a `Dispatcher` with an `EventSink`. `run_once`
  receives one envelope, dispatches it, persists + forwards each event with a
  monotonic sequence, then records cost.
- `service_channel(dispatcher, sink)` mirrors `channel()` but wires a
  `ServiceLoop`.

### Lifecycle

- `CancellationToken` (re-exported from `tokio-util`).
- `spawn_inmemory_relay(outbox, bus, tick, cancel)` — periodically drains the
  in-memory outbox onto the bus and marks records dispatched.
- `serve(service_loop, relay, shutdown)` — runs the loop + relay until the
  submission channel closes, a dispatched `Op::Shutdown` resolves (a `Completed`
  carrying `{"shutdown":"requested"}`), `ctrl_c`, or the token cancels; then
  drains the relay.

## Runtime flow (daemon dispatch path)

```text
client ──frame──▶ serve_tcp / serve_uds / serve_http
                       │ decode OpEnvelope
                       ▼
               SubmissionHandle.submit ──▶ ServiceLoop.run_once
                       │                         ├─ Dispatcher::dispatch → Vec<EventMsg>
                       │                         ├─ EventSink::persist_event (seq)
                       │                         ├─ forward each EventMsg ──┐
                       │                         └─ EventSink::record_cost  │
                       ▼                                                    ▼
               (HTTP) broadcast to GET /events subscribers     event mpsc ─▶ transport ─frame─▶ client
```

The HTTP transport fans events to all live `GET /events` SSE subscribers via a
1024-slot broadcast channel with no replay for late joiners; a lagging consumer
is dropped without starving the others (`serve_http`).

## Security posture

- **Loopback default**: the daemon's default TCP bind is `127.0.0.1:7878`. Binding
  a non-loopback address without an auth-backed policy is flagged by the daemon
  bootstrap (`tdw-backend::server`), not silently allowed.
- **Frame bounds**: readers length-prefix frames and reject empty/oversized ones
  (the matching client caps at 16 MiB); the HTTP transport bounds header/body
  sizes.
- **Address validation**: `validate_endpoint` rejects shell-metacharacter and
  parent-path UDS addresses and malformed HTTP/SSE authorities.
- **Fail-closed transports**: a transport not compiled in errors at startup.
- This crate does **not** authenticate requests — OIDC/role/mask enforcement is
  the `Dispatcher` implementation's job (`tdw-service-api`).

## Integration points

- `tdw-protocol` — `OpEnvelope` / `EventMsg` wire types.
- `tdw-config` — `DaemonTransport` and bind addresses.
- `tdw-outbox` / `tdw-bus` — the relay's source and sink.
- `tdw-service-api` — implements `Dispatcher` + `EventSink` for `AppState`.
- `tdw-app-client` — the client side of every transport.
