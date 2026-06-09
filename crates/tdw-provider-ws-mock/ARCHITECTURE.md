# tdw-provider-ws-mock — Architecture

## Module map

A single `lib.rs`, with no feature gates:

| Item | Responsibility |
| ---- | -------------- |
| `EquityTickQuery` | Subscription query (`symbol`). |
| `MockEquityStreamer` | The deterministic `Streamer` implementation. |
| `normalize_symbol` | Validates and upper-cases the requested symbol. |
| `snapshot_rows` | Builds the single hardcoded `MarketDataBar`. |
| `VecStream<T>` | A minimal always-ready `Stream` wrapping a `VecDeque`. |

## Trait implemented

`MockEquityStreamer` implements `tdw_core::Streamer<EquityTickQuery,
MarketDataBar>`:

| `PROVIDER` | `ENDPOINT` |
| ---------- | ---------- |
| `mock-ws` | `equity_ticks` |

`subscribe` returns a `VecStream` over the snapshot rows; `snapshot` returns the
same rows directly; `checkpoint` uses the default no-op.

## Data flow

```
symbol -> normalize_symbol (validate + upper-case)
       -> snapshot_rows (one MarketDataBar, venue "SIM", source "mock-ws")
       -> subscribe: VecStream<MarketDataBar>   (always-ready, single item)
       -> snapshot:  Vec<MarketDataBar>
```

The produced bar is fixed: `venue = "SIM"`, `source = "mock-ws"`,
`granularity = Tick`, all OHLC = 100.0, fixed timestamp — fully deterministic.

## Mock-streamer design

This crate **is** the mock seam for the streaming pipeline. There is no real
backend and no optional dependency: every consumer (tests, the registry, the
example) gets a deterministic single-bar stream with zero IO. `VecStream` is an
always-ready `Stream`, so it can be drained with a no-op waker and no async
runtime — see `examples/basic.rs`. Symbol validation mirrors the real providers
(reject empty / unsupported characters; upper-case the result) so wiring it in
place of a live streamer is behaviour-compatible.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No external data of any kind; the single bar is hardcoded.
- No network, filesystem, or environment access.
- Deterministic output for a given symbol, making it safe for snapshot-style
  tests and reproducible demos.
