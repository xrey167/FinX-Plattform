# tdw-provider-ws-mock

Deterministic mock equity streamer for the TDW (Trading Data Warehouse)
platform.

A zero-IO `Streamer` that emits a single hardcoded `MarketDataBar` for the
requested symbol. It exists to exercise the streaming pipeline (`subscribe`,
`snapshot`, registry wiring) in tests and demos without any network, sockets,
feature flags, or external services.

## What it provides

- `EquityTickQuery` — subscription query (`symbol`).
- `MockEquityStreamer` — implements `tdw_core::Streamer<EquityTickQuery,
  MarketDataBar>`, producing a deterministic single-bar stream/snapshot.

## Feature flags

None. The crate has no optional dependencies — it is offline by construction
and always fully compiled.

## Environment variables

None.

## Quickstart

```rust,ignore
use tdw_core::{Credentials, Streamer};
use tdw_provider_ws_mock::{EquityTickQuery, MockEquityStreamer};

let streamer = MockEquityStreamer;
let query = EquityTickQuery { symbol: "aapl".into() };

// Symbols are validated and upper-cased.
let rows = streamer.snapshot(&query, &Credentials::default()).await?;
assert_eq!(rows[0].symbol, "AAPL");
assert_eq!(rows[0].source, "mock-ws");
# Ok::<(), tdw_core::Error>(())
```

The mock validates symbols (ASCII alphanumerics plus `.`/`-`/`_`, upper-cased)
and returns `Error::InvalidQuery` for empty or unsupported input.

## Example

```bash
cargo run -p tdw-provider-ws-mock --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — drains the deterministic
`subscribe` stream and prints the snapshot, no runtime or network required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
