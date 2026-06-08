# tdw-app-server

The daemon's transport and service-loop layer. It defines the in-process plumbing
that carries `tdw_protocol::OpEnvelope`s in and `EventMsg`s out — the submission
channel, the `Dispatcher`/`EventSink` traits, the durable `ServiceLoop`, the
outbox→bus relay, and the `serve` lifecycle — plus the network transports (TCP,
Unix domain socket, HTTP/SSE) that frame those messages for external clients.

`#![forbid(unsafe_code)]`. The transports are feature-gated; the in-process
channel/loop is always available. This crate owns *how* ops move, not *what* they
mean (`tdw-protocol`) or *how* they are handled (`tdw-service-api` implements the
`Dispatcher`/`EventSink`).

## Binaries produced

None. Library crate.

## Feature flags

| Feature | Default | Effect |
| --- | --- | --- |
| `transport-tcp` | yes | `serve_tcp`: length-delimited JSON frames over TCP. |
| `transport-http` | no | `serve_http`: `POST /op` + `GET /events` (SSE broadcast). |
| `transport-uds` | no | `serve_uds`: length-delimited frames over a Unix socket (Unix only). |

A requested transport that was not compiled fails closed at startup rather than
silently binding a different one (enforced by the daemon bootstrap in
`tdw-backend`).

## Key environment variables

None directly. The bind addresses and transport selection come from `TdwConfig`
(`daemon.transport` / `daemon.tcp_bind` / `daemon.http_bind` / `daemon.uds_path`),
resolved from the `TDW_*` layers documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) (e.g. `TDW_DAEMON_TCP_BIND`).
The default TCP bind is loopback `127.0.0.1:7878`.

## Quickstart (library)

Drive the in-process submission/event channel (no socket needed):

```rust,ignore
use tdw_app_server::channel;
use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

let (handle, mut events, mut agent_loop) = channel();
handle.submit(OpEnvelope::new(
    SessionId::new("s1")?, 1,
    ActorRef { actor_id: "user".into(), kind: ActorKind::User, tenant_id: None },
    Op::Shutdown,
))?;

let event = agent_loop.run_once().await.expect("event");
assert!(matches!(event, EventMsg::Started { .. }));
# Ok::<(), Box<dyn std::error::Error>>(())
```

For the durable path, pair a `Dispatcher` + `EventSink` via `service_channel`
into a `ServiceLoop`, then `serve(loop, relay, cancel)` until ctrl-c / shutdown.
`validate_endpoint(&DaemonEndpoint)` enforces address safety per transport.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-app-server --example tdw_app_server_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — channels, loop, relay, transports.
- `tdw-app-client` — the matching client that frames `OpEnvelope`s to the server.
- `tdw-service-api` — implements `Dispatcher`/`EventSink` for `AppState`.
