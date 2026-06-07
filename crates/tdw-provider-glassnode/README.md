# tdw-provider-glassnode

Market-data provider for **Glassnode**, the Bitcoin/crypto on-chain analytics
API. Exposes a typed metric query, a `GlassnodeDataPoint` model, and a
`tdw_core::Fetcher` implementation for a curated set of on-chain metrics.

- **Vendor:** Glassnode — REST API v1
- **Base URL:** `https://api.glassnode.com/v1`
- **Endpoint:** `metric` — `GET /metrics/{category}/{name}` (selected via the `GlassnodeMetric` enum)
- **Auth:** API key passed as the `api_key` query parameter.
- **Metrics:** `mvrv_z_score`, `lth_supply`, `nupl`.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `GlassnodeHttpFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the `GlassnodeMetric` enum, the typed query/data models,
validation, and the `mock_fetch` stub are available with no network
dependencies.

## Environment variables

| Variable                  | Required | Purpose |
| ------------------------- | -------- | ------- |
| `TDW_GLASSNODE_API_KEY`   | for live calls | Glassnode API key, appended as `api_key`. |
| `TDW_GLASSNODE_LIVE`      | no       | Set to `1` to enable the live network integration test. |

The env-var name is exported as `API_KEY_ENV`.

## Quickstart

Offline (default features):

```rust
use tdw_provider_glassnode::{GlassnodeMetric, GlassnodeMetricQuery, mock_fetch};

let query = GlassnodeMetricQuery::new("btc", GlassnodeMetric::MvrvZScore, "24h")?;
for point in mock_fetch(&query)? {
    println!("{} {} = {}", point.asset, point.timestamp, point.value);
}
# Ok::<(), tdw_provider_glassnode::GlassnodeProviderError>(())
```

Live HTTP (requires `--features http` and `TDW_GLASSNODE_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_glassnode::GlassnodeHttpFetcher;

let fetcher = GlassnodeHttpFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "asset": "BTC", "metric": "mvrv_z_score", "interval": "24h" }),
           &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-glassnode --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
