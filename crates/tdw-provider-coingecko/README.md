# tdw-provider-coingecko

CoinGecko crypto OHLC provider for the TDW platform. Wraps the CoinGecko v3 API
(`https://api.coingecko.com/api/v3`) and returns canonical
`tdw_domain::MarketDataBar` rows via a single `tdw_core::Fetcher`.

| Endpoint     | Fetcher                       | Path                                         | Row model        |
| ------------ | ----------------------------- | -------------------------------------------- | ---------------- |
| OHLC         | `CoinGeckoHttpOhlcFetcher`    | `GET /coins/{coin_id}/ohlc?vs_currency=&days=` | `MarketDataBar`  |

CoinGecko's free Demo tier needs **no API key**; an optional Demo key is
forwarded when present. The crate compiles and tests offline by default: the
`CoinGeckoOhlcQuery` validation type, the `ProviderRequest` contract, and
`ohlc_request` are always available, while the network-backed fetcher and
`reqwest` dependency only exist under the `http` feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-coingecko --features http
```

## Environment variables

| Variable             | Required for          | Purpose                                                          |
| -------------------- | --------------------- | -------------------------------------------------------------- |
| `COINGECKO_API_KEY`  | optional              | Forwarded as the `x-cg-demo-api-key` header when set.          |
| `TDW_COINGECKO_LIVE=1` | live integration test | Opt-in gate; without it the live test skips so CI is offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_coingecko::CoinGeckoHttpOhlcFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = CoinGeckoHttpOhlcFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "coin_id": "bitcoin", "vs_currency": "usd", "days": 30 }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} {} close={}", bar.ts, bar.symbol, bar.close);
}
# Ok(())
# }
```

`coin_id` (alias `symbol`) is required; `vs_currency` defaults to `usd` and
`days` defaults to `30` (must be one of 1/7/14/30/90/180/365).

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-coingecko --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
