# tdw-cli

The TDW command-line client. Connects to a running daemon over loopback TCP,
frames an `OpEnvelope`, and prints the `EventMsg` frames it reads back. Also
exposes the offline `--smoke` end-to-end check.

It is a thin client: it builds the protocol envelope, writes the length-delimited
frame, and reads events until the connection closes or a 5-second timeout
elapses. Authorization, dispatch, and persistence all happen daemon-side.

## Binaries produced

- **`tdw-cli`** — the client.

## Feature flags

None.

## Key environment variables

None read directly — the daemon address is the loopback TCP default
`127.0.0.1:7878` (matching `tdw-service`). The daemon's own `TDW_*` configuration
is documented in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).

## Quickstart (binary)

```bash
# Offline end-to-end smoke (no daemon needed):
cargo run -p tdw-cli -- --smoke AAPL

# Submit a query to a running daemon (start `tdw-service` first):
cargo run -p tdw-cli -- run-query "select 1"

# Default: submit Op::Shutdown and print the response events.
cargo run -p tdw-cli
```

`run-query` prints one JSON `EventMsg` per line (typically `Started` then
`Completed`/`Failed`). Without a policy attached on the daemon, the response is
`Failed` — expected until the daemon is configured with a policy.

See [`examples/basic.rs`](examples/basic.rs) for an offline demo of the envelopes
the CLI builds (no socket): `cargo run -p tdw-cli --example tdw_cli_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — command tree and the frame protocol.
- `tdw-service` — the daemon the CLI talks to.
- `tdw-app-client` — the richer client library (`DaemonClient`).
