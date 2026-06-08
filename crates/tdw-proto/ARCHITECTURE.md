# tdw-proto — Architecture

## Module map

| Path | Role |
| ---- | ---- |
| `proto/market_data.proto` | Source of truth: the proto3 schema (`package tdw.finance`). Not compiled at build time. |
| `src/finance.gen.rs` | Vendored `prost-build` output (committed). Defines the message structs, the `TradeSide` enum, and the `market_data_envelope::Payload` `oneof`. |
| `src/lib.rs` | Wraps the generated code in a `finance` module via `include!`, then re-exports the public types. |

## Vendored-bindings design

The crate deliberately avoids build-time code generation:

- **No `build.rs`** — there is none.
- **No `prost-build`** — only the runtime `prost` crate is a dependency, used
  for the `::prost::Message` / `Oneof` / `Enumeration` derives and the
  encode/decode implementations.
- **No system `protoc`** — nothing shells out to a compiler. The generated
  Rust is committed at `src/finance.gen.rs` and included verbatim.

This keeps the build hermetic and offline: `cargo build` needs nothing beyond
crates.io dependencies. The `.proto` remains the human-authored source; the
header comment in `finance.gen.rs` documents the regeneration procedure
(install `protoc`, run `prost-build`, copy output back).

## Type / data flow

```
include!("finance.gen.rs")  ->  mod finance { OhlcvBar, Tick, PriceLevel,
                                              OrderBookSnapshot,
                                              MarketDataEnvelope, TradeSide,
                                              market_data_envelope::Payload }
lib.rs re-exports the above (Payload re-exported at crate root).
```

Encoding/decoding is provided by the `prost::Message` trait derived on each
message:

```
construct message  ->  Message::encode(&mut Vec<u8>)  ->  bytes
bytes              ->  Message::decode(&[u8])          ->  message
```

`MarketDataEnvelope.payload` is a proto `oneof` (`Payload::Bar` / `Tick` /
`OrderBook`), so a single envelope type carries any market-data message on the
bus.

## Offline / determinism design

- The crate is pure encoding/decoding logic — no IO, no network, no
  environment access. Everything runs offline by construction.
- proto3 semantics: default-valued fields are elided on the wire, so a
  `Default` message encodes to **zero bytes** (verified by the
  characterization tests in `tests/types.rs`).
- Repeated `encode` calls of the same message are byte-for-byte identical
  (deterministic), as the tests assert.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The wire format is fixed by the committed `.proto`: field tags and enum
  discriminants (`Unknown=0`, `Buy=1`, `Sell=2`) must stay stable — the tests
  guard these values.
- The generated file is the *only* source of the bindings; it is never edited
  by hand except by re-running the documented regeneration step.
