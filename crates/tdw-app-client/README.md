# tdw-app-client

The client side of the daemon transport. Two surfaces:

- `DaemonClient` — a **blocking**, std-only socket client that frames an
  `OpEnvelope` to a running daemon (TCP, Unix socket, or HTTP/SSE) and reads
  `EventMsg` frames until the matching terminal event arrives.
- `AppClient` — an in-process client over a shared `SubmissionHandle` (from
  `tdw-app-server`), for embedding a client in the same process as the daemon.

`#![forbid(unsafe_code)]`, no async runtime, no extra dependencies beyond serde +
`tdw-app-server`/`tdw-protocol`. It mirrors the server's framing exactly (4-byte
big-endian length prefix; SSE `data:` frames for HTTP) and is hardened against
truncated, oversized, and never-terminating responses.

## Binaries produced

None. Library crate (used by `tdw-cli`, `tdw-mcp`, `tdw-worker`, `tdw-backend`).

## Feature flags

None.

## Key environment variables

None directly. Consumers map their own `TDW_*` vars onto a `DaemonClientConfig`
(e.g. `tdw-mcp` reads `TDW_MCP_DAEMON_ADDR`/`TDW_DAEMON_TCP_BIND`, `tdw-worker`
reads `TDW_WORKER_DAEMON_ADDR`). The default endpoint is loopback TCP
`127.0.0.1:7878` (`DEFAULT_DAEMON_TCP_ADDR`). See
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).

## Quickstart (library)

Configure a client and submit an op, waiting for the terminal event:

```rust,ignore
use std::time::Duration;
use tdw_app_client::{DaemonClient, DaemonClientConfig};

let client = DaemonClient::new(
    DaemonClientConfig::tcp("127.0.0.1:7878").with_timeout(Duration::from_secs(2)),
);
let submission = client.submit_and_wait(&envelope)?; // blocking
println!("daemon returned {} events", submission.events.len());
# Ok::<(), tdw_app_client::DaemonClientError>(())
```

`DaemonClientConfig::default()` targets the loopback TCP daemon. `validate()`
rejects unsupported transports (e.g. UDS on non-Unix) before any I/O.

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-app-client --example tdw_app_client_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — framing, transports, terminal-event read.
- `tdw-app-server` — the matching server transports and `SubmissionHandle`.
