# tdw-provider-alpaca

Alpaca market-data provider for the TDW platform. Wraps Alpaca's historical
stock-bars endpoint (`https://data.alpaca.markets/v2/stocks/bars`) and returns
canonical `tdw_domain::MarketDataBar` rows via a single `tdw_core::Fetcher`.

| Endpoint           | Fetcher                       | Path                       | Row model        |
| ------------------ | ----------------------------- | -------------------------- | ---------------- |
| Historical bars    | `AlpacaHttpStockBarsFetcher`  | `GET /v2/stocks/bars`      | `MarketDataBar`  |

The offline core of the crate (`AlpacaStockBarsQuery`, `ProviderEndpoint`/
`ProviderRequest`, `stock_bars_request`, validation) is always available; the
network-backed fetcher and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-alpaca --features http
```

## Environment variables

| Variable               | Required for          | Purpose                                                          |
| ---------------------- | --------------------- | --------------------------------------------------------------- |
| `APCA_API_KEY_ID`      | any live HTTP call    | Sent as the `APCA-API-KEY-ID` header.                           |
| `APCA_API_SECRET_KEY`  | any live HTTP call    | Sent as the `APCA-API-SECRET-KEY` header.                       |
| `TDW_ALPACA_LIVE=1`    | live integration test | Opt-in gate; without it the live test skips so CI is offline.   |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_alpaca::AlpacaHttpStockBarsFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires APCA_API_KEY_ID and APCA_API_SECRET_KEY in the environment.
let fetcher = AlpacaHttpStockBarsFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({
            "symbol": "AAPL",
            "start": "2024-01-02",
            "end": "2024-01-05",
            "timeframe": "1Day",
            "feed": "iex",
            "limit": 5
        }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} close={} vol={}", bar.ts, bar.close, bar.volume);
}
# Ok(())
# }
```

`symbol`/`start`/`end` are required (`start`/`end` are `YYYY-MM-DD`); `timeframe`
(default `1Day`), `limit` (default `1000`), and `feed` are optional.

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-alpaca --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
