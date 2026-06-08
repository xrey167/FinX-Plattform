# tdw-provider-ws — Architecture

## Module map

A single `lib.rs` houses everything, split by feature gate:

| Item | Gate | Responsibility |
| ---- | ---- | -------------- |
| `WsTickQuery`, `WsTickStreamer`, `registry_entry`, `decode_frame`, `snapshot_rows` | always | Query type, the streamer, the pure frame decoder, and the deterministic snapshot. |
| `subscribe` (live) | `feature = "ws"` | Connects via `tokio-tungstenite`, maps text frames through `decode_frame`, flattens to a `Tick` stream. |
| `subscribe` (offline) + `VecStream` | `not(feature = "ws")` | Returns an in-memory, always-ready stream built from `snapshot_rows`. |

## Trait implemented

`WsTickStreamer` implements `tdw_core::Streamer<WsTickQuery, Tick>`:

| `PROVIDER` | `ENDPOINT` |
| ---------- | ---------- |
| `ws` | `equity_ticks` |

`Streamer` provides `subscribe` (live stream), `snapshot` (one-shot), and a
default no-op `checkpoint`.

## Data flow

```
            ws feature ON:   url -> connect_async -> text frames -> decode_frame -> Tick stream
            ws feature OFF:  symbol -> snapshot_rows -> VecStream<Tick>
            snapshot (both): symbol -> snapshot_rows -> Vec<Tick>
```

`decode_frame` is the shared, pure parsing core. It accepts:

- a JSON array of tick objects (`[{…}, {…}]`), or
- newline-delimited JSON (one tick object per line), skipping blank lines.

Invalid payloads return `Error::Provider`.

## Offline / mock-streamer design

The crate is built so the network is never required for testing or examples:

- `decode_frame` is pure (`&str -> Result<Vec<Tick>>`) — no IO — and is the
  documented unit-test seam for the live path.
- When `ws` is **off**, `subscribe` yields a deterministic `VecStream` (a
  thin always-ready `Stream` over `snapshot_rows`), so consumers exercise the
  full streaming API with zero sockets and no async runtime — see
  `examples/basic.rs`, which polls the stream with a no-op waker.
- `snapshot` is feature-independent and always deterministic.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No captured websocket traffic is committed; only synthetic tick frames
  appear in tests and the example.
- `tokio-tungstenite` / `tokio` / `futures-util` are optional and gated behind
  `ws`; the default build is offline and deterministic.
- The streamer connects only to the caller-supplied URL; no hidden endpoints.
