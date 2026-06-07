# tdw-provider-polygon

Market-data provider for **Polygon.io**. Exposes a typed aggregates query and a
`tdw_core::Fetcher` implementation that yields daily OHLCV bars as the shared
`tdw_domain::MarketDataBar` type.

- **Vendor:** Polygon.io — Stocks REST API
- **Base URL:** `https://api.polygon.io`
- **Endpoint:** `aggregates` — `GET /v2/aggs/ticker/{ticker}/range/1/day/{from}/{to}`
- **Auth:** API key passed as the `apiKey` query parameter.
- **Output model:** `tdw_domain::MarketDataBar` (`venue`/`source` = `"polygon"`).

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `PolygonHttpAggregatesFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed `PolygonAggregatesQuery` model, ticker/date
validation, and the `aggregates_request` request-contract builder are available
with no network dependencies.

## Environment variables

| Variable             | Required | Purpose |
| -------------------- | -------- | ------- |
| `POLYGON_API_KEY`    | for live calls | Polygon API key, appended as `apiKey`. (Note: this provider uses the vendor's conventional env name, not a `TDW_`-prefixed one.) |
| `TDW_POLYGON_LIVE`   | no       | Set to `1` to enable the live network integration test. |

## Quickstart

Offline (default features):

```rust
use tdw_provider_polygon::{PolygonAggregatesQuery, aggregates_request};

let query = PolygonAggregatesQuery::new("msft", "2024-01-02", "2024-01-05")?
    .with_limit(100)?;
let request = aggregates_request(&query.ticker, /* api_key_present = */ true)?;
println!("{} {}", request.provider, request.path);
# Ok::<(), tdw_provider_polygon::PolygonProviderError>(())
```

Live HTTP (requires `--features http` and `POLYGON_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_polygon::PolygonHttpAggregatesFetcher;

let fetcher = PolygonHttpAggregatesFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "ticker": "MSFT", "from": "2024-01-02", "to": "2024-01-05" }),
           &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-polygon --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
