# tdw-provider-fred

Market-data provider for **FRED** (Federal Reserve Economic Data, St. Louis
Fed). Exposes a typed query/observation model and a `tdw_core::Fetcher`
implementation for the `series/observations` endpoint.

- **Vendor:** Federal Reserve Bank of St. Louis — FRED API
- **Base URL:** `https://api.stlouisfed.org/fred`
- **Endpoint:** `series_observations` — `GET /series/observations?file_type=json`
- **Auth:** API key passed as the `api_key` query parameter.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `FredHttpSeriesObservationsFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`. |

With `http` off the typed models, series-id validation, the
`series_observations_request` request-contract builder, and the `endpoints()`
metadata are available with no network dependencies.

## Environment variables

| Variable          | Required | Purpose |
| ----------------- | -------- | ------- |
| `FRED_API_KEY`    | for live calls | FRED API key, appended as `api_key`. (Note: this provider uses the vendor's conventional env name, not a `TDW_`-prefixed one.) |
| `TDW_FRED_LIVE`   | no       | Set to `1` to enable the live network integration test. |

## Quickstart

Offline (default features):

```rust
use tdw_provider_fred::{FredSeriesObservationsQuery, series_observations_request};

let query = FredSeriesObservationsQuery::new("gdp")?;
let request = series_observations_request(&query.series_id, /* api_key_present = */ true)?;
println!("{} {}", request.provider, request.path);
# Ok::<(), tdw_provider_fred::FredProviderError>(())
```

Live HTTP (requires `--features http` and `FRED_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_fred::FredHttpSeriesObservationsFetcher;

let fetcher = FredHttpSeriesObservationsFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "series_id": "UNRATE" }), &Credentials::default())
    .await?;
```

`FredHttpSeriesObservationsFetcher` builder methods (`with_observation_start`,
`with_observation_end`, `with_limit`) bound the live query window.

## Example

```bash
cargo run -p tdw-provider-fred --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
