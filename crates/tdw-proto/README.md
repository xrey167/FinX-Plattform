# tdw-proto

Protobuf market-data types for the TDW (Trading Data Warehouse) platform.

This crate provides the on-the-wire message types that flow across the market
data bus: OHLCV bars, trade ticks, order-book snapshots, and a tagged envelope
that wraps any of them. The types are `prost`-generated and **vendored** (the
generated Rust is committed), so the crate builds with **no `protoc` and no
build-time codegen**.

## Why no `protoc` / `prost-build`?

`prost-build` would normally run at build time, shelling out to a system
`protoc` to compile `proto/market_data.proto` into Rust. That adds a build-time
toolchain dependency that is awkward in CI and offline environments. Instead:

- The generated output lives at `src/finance.gen.rs` and is committed.
- `lib.rs` pulls it in with `include!("finance.gen.rs")`.
- The only dependency is the **runtime** `prost` crate (for the derive macros
  and encode/decode), not `prost-build`.

The `.proto` source is kept at `proto/market_data.proto` as the source of
truth. To regenerate after editing it: install `protoc`, run `prost-build`
over the `.proto`, and copy the output into `src/finance.gen.rs` verbatim (see
the header comment in that file).

## Feature flags

None. The crate has a single dependency (`prost`) and no optional features.

## Environment variables

None.

## Message types

| Type | Purpose |
| ---- | ------- |
| `OhlcvBar` | OHLCV candlestick (symbol, provider, timeframe, `ts_ns`, OHLC, volume, vwap, trade_count). |
| `Tick` | Single trade (symbol, provider, `ts_ns`, price, size, `side`, trade_id). |
| `TradeSide` | Enum: `Unknown` (0), `Buy` (1), `Sell` (2). |
| `PriceLevel` | One book level (price, quantity). |
| `OrderBookSnapshot` | Bids/asks (`Vec<PriceLevel>`) plus `sequence`. |
| `MarketDataEnvelope` | Provider/symbol/ingestion metadata plus a `payload` `oneof` of bar / tick / order_book. |
| `Payload` | The envelope's `oneof` variants (`Payload::Bar`/`Tick`/`OrderBook`). |

## Quickstart

```rust
use prost::Message;
use tdw_proto::{MarketDataEnvelope, OhlcvBar, Payload};

let bar = OhlcvBar {
    symbol: "AAPL".to_string(),
    provider: "polygon".to_string(),
    timeframe: "1m".to_string(),
    ts_ns: 1_700_000_000_000_000_000,
    close: 151.0,
    ..OhlcvBar::default()
};
let env = MarketDataEnvelope {
    provider: "polygon".to_string(),
    symbol: "AAPL".to_string(),
    payload: Some(Payload::Bar(bar)),
    ..MarketDataEnvelope::default()
};

let mut buf = Vec::new();
env.encode(&mut buf)?;
let decoded = MarketDataEnvelope::decode(buf.as_slice())?;
assert!(matches!(decoded.payload, Some(Payload::Bar(_))));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Example

```bash
cargo run -p tdw-proto --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — constructs, encodes, and decodes
an envelope round-trip.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
