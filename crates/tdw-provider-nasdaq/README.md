# tdw-provider-nasdaq

Market-data provider for **NASDAQ Data Link** (formerly Quandl). Exposes a
typed query/row model and a `tdw_core::Fetcher` implementation for the dataset
data endpoint.

- **Vendor:** NASDAQ Data Link — REST API v3
- **Base URL:** `https://data.nasdaq.com/api/v3`
- **Endpoint:** `datasets` — `GET /datasets/{database}/{dataset}/data`
- **Auth:** API key passed as the `api_key` query parameter.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `NasdaqHttpDatasetFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed models, identifier/date validation, the
`dataset_request` request-contract builder, and the `NasdaqStubFetcher` are
available with no network dependencies.

## Environment variables

| Variable              | Required | Purpose |
| --------------------- | -------- | ------- |
| `TDW_NASDAQ_API_KEY`  | for live calls | NASDAQ Data Link API key, appended as `api_key`. |
| `TDW_NASDAQ_LIVE`     | no       | Set to `1` to enable the live network integration test. |

## Quickstart

Offline (default features):

```rust
use tdw_provider_nasdaq::{NasdaqDatasetQuery, NasdaqStubFetcher};

let query = NasdaqDatasetQuery::new("WIKI", "AAPL")?
    .with_start_date("2024-01-01")?;
for row in NasdaqStubFetcher::fetch(&query)? {
    println!("{}/{} cols={:?}", row.database, row.dataset, row.column_names);
}
# Ok::<(), tdw_provider_nasdaq::NasdaqProviderError>(())
```

Live HTTP (requires `--features http` and `TDW_NASDAQ_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_nasdaq::NasdaqHttpDatasetFetcher;

let fetcher = NasdaqHttpDatasetFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "database": "WIKI", "dataset": "AAPL" }),
           &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-nasdaq --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
