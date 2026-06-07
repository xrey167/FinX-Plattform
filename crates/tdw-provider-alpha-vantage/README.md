# tdw-provider-alpha-vantage

Alpha Vantage market-data provider for the TDW platform. Wraps the Alpha Vantage
REST API (`https://www.alphavantage.co/query`) and returns canonical
`tdw_domain::MarketDataBar` rows via a single `tdw_core::Fetcher` that supports
two functions.

| Function             | API value            | Output                                  |
| -------------------- | -------------------- | --------------------------------------- |
| Daily OHLCV          | `TIME_SERIES_DAILY`  | one `MarketDataBar` per trading day      |
| Latest quote         | `GLOBAL_QUOTE`       | a single snapshot `MarketDataBar`        |

> **Rate limit:** free-tier keys allow **25 requests per day**. Use a paid key or
> cache results for higher throughput.

The offline core (`AlphaVantageQuery`, `AlphaVantageFunction`, validation, and
`stub_*` JSON helpers) is always available; the network-backed fetcher and
`reqwest` dependency only exist under the `http` feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-alpha-vantage --features http
```

## Environment variables

| Variable                     | Required for          | Purpose                                                       |
| ---------------------------- | --------------------- | ------------------------------------------------------------ |
| `TDW_ALPHA_VANTAGE_API_KEY`  | any live HTTP call    | Appended as the `apikey` query parameter on every request.   |
| `TDW_ALPHA_VANTAGE_LIVE=1`   | live integration test | Opt-in gate; without it the live test skips so CI is offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_alpha_vantage::AlphaVantageHttpFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires TDW_ALPHA_VANTAGE_API_KEY in the environment.
let fetcher = AlphaVantageHttpFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "symbol": "AAPL", "function": "GLOBAL_QUOTE" }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} close={} vol={}", bar.symbol, bar.close, bar.volume);
}
# Ok(())
# }
```

`symbol` (alias `ticker`) is required; `function` defaults to
`TIME_SERIES_DAILY`. Rate-limit / informational responses from Alpha Vantage are
surfaced as `Error::Provider`.

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-alpha-vantage --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
