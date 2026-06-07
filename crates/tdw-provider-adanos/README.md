# tdw-provider-adanos

Adanos social-sentiment provider for the TDW platform. Wraps the Adanos REST API
(`https://api.adanos.io/v1`) and exposes three read endpoints as
`tdw_core::Fetcher` implementations:

| Endpoint               | Fetcher                        | Path                                | Row model                  |
| ---------------------- | ------------------------------ | ----------------------------------- | -------------------------- |
| Stock sentiment        | `AdanosSentimentHttpFetcher`   | `GET /sentiment/stocks/{ticker}`    | `AdanosSentimentResult`    |
| Trending stocks        | `AdanosTrendingHttpFetcher`    | `GET /trending/stocks?limit=`       | `AdanosTrendingItem`       |
| Polymarket events      | `AdanosPolymarketHttpFetcher`  | `GET /polymarket/events?limit=`     | `AdanosPolymarketEvent`    |

The crate compiles and tests offline by default. Query structs, error types,
domain models, and hard-coded `mock_fetch_*` helpers are always available; the
network-backed fetchers and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

Enable for live use or to run the cassette/example:

```bash
cargo build -p tdw-provider-adanos --features http
```

## Environment variables

| Variable               | Required for         | Purpose                                                                  |
| ---------------------- | -------------------- | ------------------------------------------------------------------------ |
| `TDW_ADANOS_API_KEY`   | any live HTTP call   | Sent as the `X-API-Key` header. Missing/empty key fails before any I/O.  |
| `TDW_ADANOS_LIVE=1`    | live integration tests | Opt-in gate; without it the live tests skip so CI stays offline.       |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_adanos::AdanosSentimentHttpFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires TDW_ADANOS_API_KEY in the environment.
let fetcher = AdanosSentimentHttpFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "ticker": "AAPL" }), &Credentials::default())
    .await?;
for row in obb.rows {
    println!("{} sentiment={} trend={}", row.ticker, row.sentiment_score, row.trend);
}
# Ok(())
# }
```

For an offline, network-free walkthrough that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-adanos --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
- Cross-cutting design: [`docs/architecture.md`](../../docs/architecture.md).
