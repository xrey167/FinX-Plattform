# tdw-provider-ccdata

CCData (formerly CryptoCompare) crypto market-data provider for the TDW platform.
Wraps the CCData REST API (`https://data-api.ccdata.io`) and exposes two read
endpoints as `tdw_core::Fetcher` implementations.

| Endpoint            | Fetcher                     | Path                                  | Row model        |
| ------------------- | --------------------------- | ------------------------------------- | ---------------- |
| Daily OHLCV         | `CCDataHttpFetcher`         | `GET /spot/v1/historical/days`        | `MarketDataBar`  |
| Asset metadata      | `CCDataAssetHttpFetcher`    | `GET /asset/v1/data/by/symbol`        | `CCDataAssetRow` |

The crate compiles and tests offline by default: query structs (`CCDataOhlcvQuery`
/ `CCDataAssetQuery`), validation, error types, and `stub_*_response` JSON helpers
are always available, while the network-backed fetchers and `reqwest` dependency
only exist under the `http` feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-ccdata --features http
```

## Environment variables

| Variable              | Required for          | Purpose                                                          |
| --------------------- | --------------------- | -------------------------------------------------------------- |
| `TDW_CCDATA_API_KEY`  | any live HTTP call    | Sent as the `authorization: Apikey <key>` header.              |
| `TDW_CCDATA_LIVE=1`   | live integration test | Opt-in gate; without it the live test skips so CI is offline.  |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_ccdata::CCDataHttpFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires TDW_CCDATA_API_KEY in the environment.
let fetcher = CCDataHttpFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "market": "ccix", "instrument": "BTC-USD", "limit": 30 }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} {} close={}", bar.ts, bar.symbol, bar.close);
}
# Ok(())
# }
```

The OHLCV fetcher takes `market` (default `ccix`), `instrument`
(`ASSET-CURRENCY`, e.g. `BTC-USD`), and `limit` (default `30`); the asset fetcher
takes `symbol` (alias `asset_symbol`).

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-ccdata --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
