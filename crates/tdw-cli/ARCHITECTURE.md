# tdw-cli architecture

`tdw-cli` is a thin daemon client: it builds an `OpEnvelope`, frames it to the
loopback TCP daemon, and prints the events it reads back. It performs no
authorization or dispatch of its own.

## Module map

| Path | Contents |
| --- | --- |
| `src/main.rs` | the `tdw-cli` binary: arg dispatch, `connect_and_run`, framing |

## Command tree

```text
tdw-cli --smoke [SYMBOL]      → offline run_end_to_end_smoke, print one summary line
tdw-cli run-query [SQL]       → Op::RunQuery (default "select 1"), print event JSON lines
tdw-cli                       → Op::Shutdown (default), print event JSON lines
```

The daemon address is the static loopback default `127.0.0.1:7878`.

## Frame protocol (client side)

`connect_and_run(addr, op)`:

1. Connect to `addr` with a 5-second timeout.
2. Build an `OpEnvelope` (`SessionId::generated()`, sequence 1, a `User` actor).
3. `write_frame` — 4-byte big-endian length prefix + JSON body, flushed.
4. `read_frame` loop — read length-prefixed `EventMsg` frames (rejecting empty or
   `> 16 MiB`) until EOF or a 5-second deadline; decode each as `EventMsg`.

This mirrors `tdw-app-server::serve_tcp` exactly. (For the fuller client with
UDS/HTTP-SSE and terminal-event detection, use `tdw-app-client::DaemonClient`.)

## Runtime flow

```text
tdw-cli run-query "select 1"
   connect 127.0.0.1:7878
   write framed OpEnvelope{ RunQuery }
   read EventMsg frames ──▶ print each as JSON
   (daemon enforces policy + dispatches; CLI only transports)
```

## Security posture

- Targets **loopback** only (`127.0.0.1:7878`).
- **Bounded reads**: per-frame length cap (16 MiB) and a 5-second read deadline so
  a stuck/hostile daemon cannot hang the CLI.
- No credentials are held or sent by the CLI; the daemon performs all auth. The
  `--smoke` path is fully offline.

## Integration points

- `tdw-protocol` — `Op`/`EventMsg`/`OpEnvelope` it frames.
- `tdw-service-api` / `tdw-test-utils` — the `--smoke` end-to-end check.
- `tdw-service` — the daemon endpoint it connects to.
