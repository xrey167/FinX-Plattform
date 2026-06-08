# tdw-app-client architecture

`tdw-app-client` frames an `OpEnvelope` to a daemon and reads back the terminal
`EventMsg`. It is blocking std I/O (no async runtime) so it can be called from any
context — including from a `spawn_blocking` task or a non-async MCP thread.

## Module map

A single `src/lib.rs`.

## Key types

### `DaemonClient` (socket client)

- `DaemonClientConfig` — `{ endpoint: DaemonEndpoint, timeout }`. Constructors:
  `default()` (loopback TCP `127.0.0.1:7878`), `tcp(addr)`, `new(endpoint)`;
  builders `with_timeout(d)` (zero ignored). `validate()` checks the endpoint and
  rejects UDS on non-Unix.
- `DaemonClient::new(config)` / `default()`. `submit_and_wait(&OpEnvelope) ->
  DaemonClientResult` validates the config then dispatches per transport:
  - `submit_tcp` — connect, set read/write timeouts, write the framed envelope,
    read terminal events.
  - `submit_uds` (Unix only) — same over a `UnixStream`.
  - `submit_http_sse` — open `GET <events>` (expect `200 text/event-stream`),
    `POST <op>` the JSON body (expect `202`), then read SSE `data:` event frames.
    Only plain `http://` endpoints are supported; `https://` fails closed.
- `DaemonSubmission { endpoint, op_id, events }` — the result.
- `DaemonClientError` — `Connect`, `Io`, `Serialize`/`Deserialize`, `Protocol`,
  `EmptyFrame`, `FrameTooLarge`, `TimedOut`, `TerminalEventMissing`,
  `UnsupportedTransport`/`UnsupportedEndpoint`, `InvalidEndpoint`.

### `AppClient` (in-process client)

- `ClientInfo { name, endpoint }` + `validate_client_info` (rejects control
  chars, path separators, shell metacharacters, and `..` in the name).
- `AppClient::try_new(info, SubmissionHandle)` validates then wraps a
  `tdw-app-server` `SubmissionHandle`; `submit(OpEnvelope)` enqueues in-process.

### Framing helpers

- `write_envelope_frame` — 4-byte big-endian length prefix + JSON body.
- `read_length_delimited_event_frame` — bounds: rejects `EmptyFrame` and
  `FrameTooLarge` (> `MAX_DAEMON_FRAME_BYTES` = 16 MiB).
- `read_terminal_events` — reads up to `MAX_DAEMON_EVENTS` (256) frames, stops at
  the first event whose `op_id` matches and is terminal (`Completed`/`Failed`/
  `Cancelled`); errors `TerminalEventMissing` if none arrives.
- HTTP helpers cap header bytes (`MAX_HTTP_HEADER_BYTES` = 8 KiB) and parse the
  status line / required content type.

## Runtime flow

```text
DaemonClient::submit_and_wait(&envelope)
   validate config ─▶ per transport:
     TCP/UDS: connect → write_envelope_frame → read_terminal_events
     HTTP/SSE: GET events (200) ; POST op (202) ; read SSE data frames
   stop at first matching terminal EventMsg ─▶ DaemonSubmission { events }
```

## Security posture

- **Loopback default** endpoint (`DEFAULT_DAEMON_TCP_ADDR`).
- **Bounded reads**: every reader is length/size/count-capped and timeout-guarded,
  so a hostile or stuck daemon cannot hang or OOM the client.
- **`https://` fails closed** on the HTTP/SSE path (plain `http://` only — TLS is
  expected to be terminated by a fronting proxy, not by this client).
- **Identity validation**: `validate_client_info` blocks injection-prone client
  names; `DaemonEndpoint::validate` (from `tdw-app-server`) blocks unsafe
  addresses before any connection attempt.
- The hidden `__fuzz_daemon_frame` shim feeds arbitrary bytes through the frame
  reader and must never panic (nightly cargo-fuzz target).

## Integration points

- `tdw-app-server` — `DaemonEndpoint`/`DaemonTransport`, `SubmissionHandle`, and
  the matching server transports/framing.
- `tdw-protocol` — `OpEnvelope`/`EventMsg`/`OpId`.
- `tdw-cli`, `tdw-mcp`, `tdw-worker`, `tdw-backend` — consumers that map config
  onto a `DaemonClientConfig`.
