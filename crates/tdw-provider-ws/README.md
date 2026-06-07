# tdw-provider-ws

Generic websocket tick streamer for the TDW (Trading Data Warehouse) platform.

Provides a `Streamer` that connects to any `ws://` / `wss://` endpoint emitting
JSON ticks. The live socket path is feature-gated; the **frame decoder**, the
deterministic **snapshot**, and the **registry entry** are always available so
the parsing logic is testable and usable fully offline.

## What it provides

- `WsTickQuery` — subscription query (`url` + `symbol`).
- `WsTickStreamer` — implements `tdw_core::Streamer<WsTickQuery, Tick>`.
- `decode_frame(&str) -> Result<Vec<Tick>>` — pure decoder accepting either a
  JSON array of ticks or newline-delimited JSON (NDJSON). This is the unit-test
  seam for the live `subscribe` path.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `ws`    | off     | Compiles the live `subscribe` path (`tokio-tungstenite`, `tokio`, `futures-util`) that connects to a real socket. |

With `ws` **off**, `subscribe` still returns a deterministic in-memory stream
built from `snapshot`, so callers and tests run without opening a socket.
`decode_frame`, `snapshot`, and `registry_entry` are available in **both**
builds.

## Environment variables

None. The endpoint URL is supplied per-subscription via `WsTickQuery.url`.

## Quickstart

```rust
use tdw_provider_ws::decode_frame;

// Decode a JSON-array frame (pure, offline).
let frame = r#"[{"symbol":"AAPL","venue":"WS","ts":"2026-05-21T20:00:00Z","price":100.5,"size":10.0}]"#;
let ticks = decode_frame(frame)?;
assert_eq!(ticks[0].symbol, "AAPL");

// NDJSON is also accepted; blank lines are skipped.
let ndjson = "{\"symbol\":\"MSFT\",\"venue\":\"WS\",\"ts\":\"2026-05-21T20:00:01Z\",\"price\":420.0,\"size\":2.0}";
assert_eq!(decode_frame(ndjson)?.len(), 1);
# Ok::<(), tdw_core::Error>(())
```

With the `ws` feature, `subscribe` opens a real socket:

```rust,ignore
use tdw_core::{Credentials, Streamer};
use tdw_provider_ws::{WsTickQuery, WsTickStreamer};

let streamer = WsTickStreamer;
let query = WsTickQuery { url: "wss://example/ws".into(), symbol: "AAPL".into() };
let mut stream = streamer.subscribe(query, &Credentials::default()).await?;
```

## Example

```bash
cargo run -p tdw-provider-ws --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — decodes array + NDJSON frames
and drains the deterministic offline `subscribe` stream, no socket required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
