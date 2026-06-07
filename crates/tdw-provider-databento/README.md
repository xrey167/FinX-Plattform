# tdw-provider-databento

Databento historical market-data provider for the TDW platform. Wraps the
Databento historical API (`https://hist.databento.com/v0`) and exposes two read
endpoints as `tdw_core::Fetcher` implementations.

| Endpoint            | Fetcher                            | Path                                | Row model         |
| ------------------- | ---------------------------------- | ----------------------------------- | ----------------- |
| Timeseries (OHLCV)  | `DatabentoHttpTimeseriesFetcher`   | `POST /timeseries.get_range`        | `MarketDataBar`   |
| Dataset metadata    | `DatabentoMetadataFetcher`         | `GET /metadata.list_datasets`       | `DatabentoDataset`|

Authentication is HTTP Basic with the API key as the username and an empty
password. The crate compiles and tests offline by default: query structs
(`DatabentoHistoricalQuery` / `DatabentoTimeseriesQuery`), validation, and error
types are always available, while the network-backed fetchers and `reqwest`
dependency only exist under the `http` feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-databento --features http
```

## Environment variables

| Variable                 | Required for          | Purpose                                                          |
| ------------------------ | --------------------- | -------------------------------------------------------------- |
| `TDW_DATABENTO_API_KEY`  | any live HTTP call    | Used as the HTTP Basic-auth username (empty password).         |
| `TDW_DATABENTO_LIVE=1`   | live integration test | Opt-in gate; without it the live test skips so CI is offline.  |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_databento::DatabentoHttpTimeseriesFetcher;

# async fn run() -> tdw_core::Result<()> {
// Requires TDW_DATABENTO_API_KEY in the environment.
let fetcher = DatabentoHttpTimeseriesFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({
            "dataset": "GLBX.MDP3",
            "symbols": ["ESH5"],
            "start": "2024-01-01",
            "end": "2024-01-31"
        }),
        &Credentials::default(),
    )
    .await?;
for bar in obb.rows {
    println!("{} {} close={}", bar.ts, bar.symbol, bar.close);
}
# Ok(())
# }
```

Timeseries requires `dataset`, `symbols` (array), and `start`/`end`
(`YYYY-MM-DD`); the daily OHLCV schema (`ohlcv-1d`) is requested internally. The
metadata fetcher takes no parameters.

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-databento --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
