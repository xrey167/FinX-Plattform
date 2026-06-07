# tdw-provider-deribit

Deribit crypto-derivatives provider for the TDW platform. Wraps Deribit's public
v2 REST API (`https://www.deribit.com/api/v2`) and exposes three read endpoints
as `tdw_core::Fetcher` implementations.

| Endpoint              | Fetcher                          | Path                                       | Row model               |
| --------------------- | -------------------------------- | ------------------------------------------ | ----------------------- |
| Instruments           | `DeribitHttpInstrumentsFetcher`  | `GET /public/get_instruments`              | `DeribitInstrument`     |
| Order book            | `DeribitHttpOrderBookFetcher`    | `GET /public/get_order_book`               | `DeribitOrderBook`      |
| Funding-rate history  | `DeribitHttpFundingFetcher`      | `GET /public/get_funding_rate_history`     | `DeribitFundingRecord`  |

**No API key is required** — all three are public endpoints. The crate compiles
and tests offline by default: query structs, the `DeribitKind` enum, request-path
helpers, domain models, and `stub_*` helpers are always available, while the
network-backed fetchers and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-deribit --features http
```

## Environment variables

| Variable              | Required for           | Purpose                                                       |
| --------------------- | ---------------------- | ------------------------------------------------------------ |
| _(none — no API key)_ | —                      | Public endpoints are unauthenticated.                        |
| `TDW_DERIBIT_LIVE=1`  | live integration tests | Opt-in gate; without it the live tests skip so CI is offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_deribit::DeribitHttpInstrumentsFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = DeribitHttpInstrumentsFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "currency": "BTC", "kind": "option" }),
        &Credentials::default(),
    )
    .await?;
for inst in obb.rows {
    println!("{} kind={} active={}", inst.instrument_name, inst.kind, inst.is_active);
}
# Ok(())
# }
```

- Instruments: `currency` (uppercase ASCII letters) + `kind`
  (option/future/perpetual/future_combo/option_combo, default option).
- Order book: `instrument_name` + `depth` (1/5/10/20, default 5).
- Funding: `instrument_name` + `start_ms`/`end_ms` + `count` (1..=1000, default 100).

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-deribit --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
