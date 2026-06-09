# tdw-service

The standalone TDW daemon binary. A thin entrypoint: it resolves the layered
`TdwConfig`, builds the `AppState` composition root, wires the service loop +
outbox→bus relay + transport, and serves until ctrl-c or a dispatched
`Op::Shutdown`. All composition lives in `tdw-backend::server`, which this binary
calls back into so the standalone daemon and the unified `tdw-backend` binary
share one implementation.

It also exposes an offline `--smoke` mode (the `tdw-test-utils` end-to-end smoke)
used as the CI "still works" check.

## Binaries produced

- **`tdw-service`** — the daemon. Default mode binds the configured transport
  (default loopback TCP `127.0.0.1:7878`) and serves the daemon request path.

## Feature flags

| Feature | Effect |
| --- | --- |
| `transport-http` | forward `transport-http` to `tdw-app-server` + `tdw-backend` |
| `transport-uds` | forward `transport-uds` (Unix domain socket) |
| `daemon-postgres` | Postgres-backed daemon session/rollout stores (forwarded to `tdw-service-api`) |

`transport-tcp` is always available via the `tdw-app-server` default. A requested
transport not compiled in fails closed at startup.

## Key environment variables

Resolved through `tdw-backend::server::load_config` (full list in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md)):

- `TDW_CONFIG` — path to a TOML config layer (merged on top of defaults).
- `TDW_DAEMON_TCP_BIND` — TCP bind override (default `127.0.0.1:7878`; use
  `0.0.0.0:7878` only behind an auth layer).
- `TDW_PROFILE` — overrides the resolved profile (`live` wires real engines;
  `prod`/`production` require `TDW_OIDC_*`).
- Engine/auth vars: see `tdw-service-api` (`real-engines` path).

## Quickstart (binary)

```bash
# Offline end-to-end smoke (no daemon, no network):
cargo run -p tdw-service -- --smoke AAPL

# Run the daemon on the loopback TCP default:
cargo run -p tdw-service

# Bind elsewhere / select a profile:
TDW_DAEMON_TCP_BIND=127.0.0.1:9000 TDW_PROFILE=service cargo run -p tdw-service
```

The daemon prints its bound transport/address on startup and serves until
ctrl-c. A client (`tdw-cli run-query "select 1"`) submits ops over the transport.

See [`examples/basic.rs`](examples/basic.rs) for an offline config-resolution
demo (no socket is bound): `cargo run -p tdw-service --example tdw_service_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — boot sequence and serve lifecycle.
- `tdw-backend` — the shared serving glue (`server::run_daemon`).
- `tdw-service-api` — the `AppState` composition root and dispatch path.
